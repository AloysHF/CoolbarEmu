//! CPU execution loop, interworking branches, and service dispatch entry.

use anyhow::{bail, Result};
use armv4t_emu::{reg, Memory, Mode};
use log::info;

use super::{
    arm_blx_immediate_target, fixed_manager_specs, service_trace_enabled, thumb_add_pc_target,
    CoolbarMachine, APP_STORE_MANAGER, DATA_PACKAGE_FULL_PATH_SERVICE, EXIT_ADDRESS,
    EXIT_REGION_SIZE, FIXED_GAMEOLD_OBJECT_SERVICE, FIXED_GAMEOLD_REGION_SERVICE,
    FIXED_MANAGER_GET, FIXED_MANAGER_INIT, IO_METHOD_BASE, IO_METHOD_STRIDE, IO_NV_READ_OFFSET,
    IO_NV_WRITE_OFFSET, LOG_NOOP_SERVICE, MANAGER_BASE, MANAGER_SIZE, MEMORY_BLOCK_SERVICE,
    METHOD_STUB_BASE, METHOD_STUB_KINDS, METHOD_STUB_STRIDE, NATIVE_BILLING_PAYNUM,
    NATIVE_BILLING_REMAIN_DAY, NATIVE_BILLING_SEND_SMS, NATIVE_DIRTY_RECT_SERVICE,
    NATIVE_DISPATCH_SERVICE, NATIVE_SYSTEM_TIME_SERVICE, OLDLIB_DRAW_SERVICE, SERVICE_BASE,
    SERVICE_SIZE, TABLE_STRIDE,
};

/// Whether an ARM condition code passes against `cpsr`, using the standard
/// N/Z/C/V flags in bits 31..28. Mirrors the core's own test so instructions
/// handled outside it stay conditional.
fn arm_condition_met(condition: u8, cpsr: u32) -> bool {
    let negative = cpsr & (1 << 31) != 0;
    let zero = cpsr & (1 << 30) != 0;
    let carry = cpsr & (1 << 29) != 0;
    let overflow = cpsr & (1 << 28) != 0;
    match condition {
        0 => zero,
        1 => !zero,
        2 => carry,
        3 => !carry,
        4 => negative,
        5 => !negative,
        6 => overflow,
        7 => !overflow,
        8 => carry && !zero,
        9 => !carry || zero,
        10 => negative == overflow,
        11 => negative != overflow,
        12 => !zero && negative == overflow,
        13 => zero || negative != overflow,
        _ => true,
    }
}

impl CoolbarMachine {
    pub(super) fn run_until_return(&mut self, instruction_limit: u64) -> Result<()> {
        for _ in 0..instruction_limit {
            let mut pc = self.cpu.reg_get(Mode::User, reg::PC);
            let aligned_pc = if self.cpu.thumb_mode() {
                pc & !1
            } else {
                pc & !3
            };
            if aligned_pc != pc {
                pc = aligned_pc;
                self.cpu.reg_set(Mode::User, reg::PC, pc);
            }
            // Record non-fallthrough transfers (2/4-byte adjacency assumed to
            // be sequential execution) so fault diagnostics can show where a
            // bad jump came from.
            if self.instruction_count > 0
                && pc != self.last_pc.wrapping_add(2)
                && pc != self.last_pc.wrapping_add(4)
            {
                if self.recent_branches.len() == 16 {
                    self.recent_branches.pop_front();
                }
                self.recent_branches.push_back((self.last_pc, pc));
                // Snapshot registers at transfer time: a bad jump into a
                // zero-filled region executes junk before faulting, which
                // would otherwise clobber the dispatch-time state.
                self.branch_regs = Some(std::array::from_fn(|index| {
                    self.cpu.reg_get(Mode::User, index as u8)
                }));
            }
            self.last_pc = pc;
            if self.recent_pcs.len() == 32 {
                self.recent_pcs.pop_front();
            }
            self.recent_pcs.push_back(pc);
            if pc & !1 == EXIT_ADDRESS {
                return Ok(());
            }
            let terminal_self_branch = if self.cpu.thumb_mode() {
                self.memory.r16(pc) == 0xe7fe
            } else {
                self.memory.r32(pc) == 0xeaff_fffe
            };
            if terminal_self_branch {
                self.state = super::MachineState::Halted;
                return Ok(());
            }
            // An indirect branch through an unpopulated method slot (e.g.
            // `bx r3` with r3 == 0) is how the DF-engine vtable dispatch
            // invokes a method the guest never implemented.  Branching to
            // address 0 would fault, so treat it as an unimplemented method:
            // resume at the caller's return address (LR is still the caller's
            // because `bx` does not relink) with a zero result.  This mirrors
            // the host-side fill_zero_method_slots handling for allocated
            // objects, but also covers guest-managed objects whose method
            // tables the host cannot pre-fill at construction time.
            //
            // A zero slot is only the cleanest case of the same thing: some
            // games ship tables whose slot holds guest payload — a resource
            // record, a length word, leftover heap bytes — instead of a
            // function pointer.  Those values are non-zero, so the zero-slot
            // rule misses them and the dispatch faults on a fetch from the
            // payload address.  Judging the slot by whether it addresses
            // mapped memory catches both cases without guessing at the table
            // layout: anything unreachable is a call with no implementation
            // to run.
            if self.cpu.thumb_mode() {
                let insn = self.memory.r16(pc);
                if insn & 0xff87 == 0x4700 {
                    let rm = ((insn >> 3) & 0x0f) as u8;
                    if rm < 13 {
                        let slot = self.register(rm);
                        let aligned = slot & !1;
                        // The host's own sentinel windows are unmapped by
                        // design: a fetch there is intercepted as a host
                        // call, not as a jump into nowhere.  Only values
                        // that address nothing at all are dead slots.
                        let host_window = (EXIT_ADDRESS..EXIT_ADDRESS + EXIT_REGION_SIZE)
                            .contains(&aligned)
                            || (MANAGER_BASE..MANAGER_BASE + MANAGER_SIZE as u32)
                                .contains(&aligned)
                            || (SERVICE_BASE..SERVICE_BASE + SERVICE_SIZE).contains(&aligned);
                        // Only the ROM image holds routines the guest
                        // compiled, so that is the one place a slot can
                        // legitimately name code.  Everything else — heap,
                        // data, stack — is payload if it lands there.  The
                        // ROM mapping is larger than what the archive loads,
                        // though: its zero-filled tail would fault the fetch,
                        // so a slot there names no routine either.
                        let in_rom_image = self
                            .memory
                            .region(aligned, 2)
                            .is_some_and(|r| r.base == self.executable.code_address())
                            && !self.pc_in_unloaded_rom(aligned);
                        // A slot can also land on mapped memory that simply
                        // holds no code — an uninitialised page, a zeroed
                        // resource buffer.  Fetching there decodes as a NOP
                        // and slides through the region until it faults, so
                        // the first instruction word says it: no real entry
                        // point ever begins with a zero encoding.
                        let starts_with_no_instruction = if slot % 2 == 1 {
                            self.memory.region(aligned, 2).is_some()
                                && self.memory.r16(aligned) == 0
                        } else {
                            self.memory.region(aligned, 4).is_some()
                                && self.memory.r32(aligned) == 0
                        };
                        // An even slot leaves Thumb state.  Inside the ROM
                        // image that is a real ARM entry point (058/059 call
                        // ARM routines that way); outside it the address is
                        // usually a slot the guest never filled, but payload
                        // and guest-generated ARM thunks share the heap, so
                        // parity alone does not settle it — ask whether the
                        // address can return before calling it a dead slot.
                        let unresolved = slot == 0
                            || (!host_window
                                && (self.memory.region(aligned, 2).is_none()
                                    || (!in_rom_image
                                        && ((slot.is_multiple_of(2)
                                            && !self.names_arm_routine(aligned))
                                            || starts_with_no_instruction))));
                        if unresolved {
                            if std::env::var_os("CBE_TRACE").is_some() {
                                eprintln!(
                                    "[bxstub] pc=0x{pc:08X} r{rm}=0x{slot:08X} even={} zero={}",
                                    slot.is_multiple_of(2),
                                    starts_with_no_instruction
                                );
                            }
                            let lr = self.register(reg::LR) & !1;
                            if lr != 0 && self.memory.region(lr, 2).is_some() {
                                self.set_result(0);
                                self.cpu.reg_set(Mode::User, reg::PC, lr);
                                continue;
                            }
                        }
                    }
                }
            }
            if (SERVICE_BASE..SERVICE_BASE + SERVICE_SIZE).contains(&pc) {
                self.handle_service(pc)?;
            } else if (MANAGER_BASE..MANAGER_BASE + MANAGER_SIZE as u32).contains(&pc) {
                // The manager region holds only data (tables and slots), so a
                // fetch here means the game is calling the application
                // interface as code — the native-dispatch convention: the id
                // travels in r0 and the argument in r1, and the call returns
                // through LR. Intercepting the fetch keeps the region usable
                // as both a writable structure (entry-point registration) and
                // a callable stub, whichever protocol the game follows.
                self.handle_native_dispatch_service();
                self.return_from_service();
            } else if (EXIT_ADDRESS..EXIT_ADDRESS + EXIT_REGION_SIZE).contains(&pc) {
                // The exit window doubles as the host trampoline table. The
                // sentinel at exactly EXIT_ADDRESS is the guest's "return to
                // host" address (handled above), but games also treat it as a
                // code base: they load the sentinel into a register and run a
                // `BL` second half-word, whose offset selects a slot at
                // EXIT_ADDRESS + offset. Fetching such a slot used to fault
                // because only the single sentinel address was recognised.
                // Each slot is an unanswered host call, so it must return to
                // the caller — `LR` already holds the resume address that the
                // `BL` half-word computed.
                let target = self.register(reg::LR);
                self.cpu.reg_set(Mode::User, reg::PC, target & !1);
                let mut cpsr = self.register(reg::CPSR);
                if target & 1 != 0 {
                    cpsr |= 1 << 5;
                } else {
                    cpsr &= !(1 << 5);
                }
                self.cpu.reg_set(Mode::User, reg::CPSR, cpsr);
            } else if self.is_semihosting_call(pc) {
                if self.handle_semihosting(pc)? {
                    return Ok(());
                }
            } else if self.handle_thumb_add_pc(pc) || self.handle_interworking_branch(pc) {
            } else if self.handle_clz(pc) {
                // ARMv5T `CLZ` handled outside the core; see the method.
            } else if self.pc_in_unloaded_rom(pc) {
                // The guest branched into the zero-filled BSS/data tail of
                // the ROM mapping — a bad function pointer, not real code.
                self.state = super::MachineState::Faulted;
                bail!(
                    "instruction fetch from unloaded ROM address 0x{pc:08X}{}",
                    self.unmapped_fetch_context()
                );
            } else if self
                .memory
                .region(pc, if self.cpu.thumb_mode() { 2 } else { 4 })
                .is_none()
            {
                self.state = super::MachineState::Faulted;
                bail!(
                    "instruction fetch from unmapped address 0x{pc:08X}{}",
                    self.unmapped_fetch_context()
                );
            } else if !self.cpu.step(&mut self.memory) {
                self.state = super::MachineState::Faulted;
                bail!("unsupported ARM instruction at 0x{pc:08X}");
            }
            self.instruction_count += 1;
        }
        self.state = super::MachineState::Faulted;
        bail!(
            "CBE execution exceeded {instruction_limit} instructions at 0x{:08X}{}",
            self.last_pc,
            self.unmapped_fetch_context()
        )
    }

    /// Diagnostic context appended to unmapped-fetch faults: the recent PC
    /// history and the last service calls, so a fault can be traced back to
    /// the guest code path that computed the bad address.
    /// True when `pc` falls inside the mapped ROM region but past the end of
    /// the loaded code image — the zero-filled BSS/data tail where a guest
    /// should never fetch instructions.
    /// Whether `address` can be entered as ARM and come back: a routine ends
    /// with `bx lr`, `mov pc, lr`, or a `ldm`/`pop` that reloads the program
    /// counter.  Payload almost never reproduces those encodings inside a few
    /// words, so the scan separates a guest-generated thunk from a slot the
    /// guest forgot to fill — which matters because both live in the heap and
    /// neither is distinguishable by address (`A/小酷音乐(强推)V101`, report
    /// #45, calls a real heap thunk this way).
    fn names_arm_routine(&mut self, address: u32) -> bool {
        let mut pc = address & !3;
        for _ in 0..24 {
            if self.memory.region(pc, 4).is_none() {
                return false;
            }
            let word = self.memory.r32(pc);
            let returns = (word & 0x0fff_ffff) == 0x012f_ff1e // bx lr
                || (word & 0x0fff_ffff) == 0x01a0_f00e // mov pc, lr
                || (word & 0x0fff_0000) == 0x08bd_0000 // pop {…, pc}
                || (word & 0x0fbf_0000) == 0x089b_0000; // ldm {…, pc}
            if returns {
                return true;
            }
            pc = pc.wrapping_add(4);
        }
        false
    }

    fn pc_in_unloaded_rom(&self, pc: u32) -> bool {
        let code_start = self.executable.code_address();
        let code_end = code_start + self.executable.code_size as u32;
        if (code_start..code_end).contains(&pc) {
            return false;
        }
        // Only flag addresses that share the ROM mapping — other regions
        // (manager, service, heap) are legitimate.
        self.memory.region(pc, 2).is_some() && self.memory.region(code_start, 1).is_some() && {
            let rom = self.memory.region(code_start, 1).unwrap();
            (rom.base..rom.base + rom.data.len() as u32).contains(&pc)
        }
    }

    fn unmapped_fetch_context(&self) -> String {
        let pcs: Vec<String> = self
            .recent_pcs
            .iter()
            .map(|pc| format!("0x{pc:08X}"))
            .collect();
        let services: Vec<String> = self
            .recent_services
            .iter()
            .map(|(group, index, lr, r0)| format!("{group}:{index}(lr=0x{lr:08X},r0=0x{r0:08X})"))
            .collect();
        let bad: Vec<String> = self
            .memory
            .unmapped_accesses()
            .iter()
            .take(8)
            .map(|address| format!("0x{address:08X}"))
            .collect();
        let regs: Vec<String> = (0..16)
            .map(|index| format!("r{index}={:08X}", self.register(index)))
            .collect();
        let branches: Vec<String> = self
            .recent_branches
            .iter()
            .map(|(from, to)| format!("0x{from:08X}->0x{to:08X}"))
            .collect();
        let branch_regs = self.branch_regs.map(|regs| {
            let parts: Vec<String> = (0..16)
                .map(|index| format!("r{index}={:08X}", regs[index]))
                .collect();
            format!("\n  registers at last branch: [{}]", parts.join(", "))
        });
        format!(
            "\n  recent pcs: [{}]\n  recent branches: [{}]\n  recent services: [{}]\n  unmapped data accesses: [{}]\n  registers: [{}]{}",
            pcs.join(", "),
            branches.join(", "),
            services.join(", "),
            bad.join(", "),
            regs.join(", "),
            branch_regs.unwrap_or_default()
        )
    }

    fn handle_thumb_blx(&mut self, pc: u32) -> bool {
        self.handle_thumb_blx_register(pc) || self.handle_thumb_blx_immediate(pc)
    }

    fn handle_thumb_add_pc(&mut self, pc: u32) -> bool {
        if !self.cpu.thumb_mode() {
            return false;
        }
        let instruction = self.memory.r16(pc);
        let source = ((instruction >> 3) & 0x0f) as u8;
        let Some(target) = thumb_add_pc_target(pc, instruction, self.register(source)) else {
            return false;
        };
        self.cpu.reg_set(Mode::User, reg::PC, target);
        true
    }

    /// ARMv5T `CLZ Rd, Rm` — count leading zeros.
    ///
    /// The cores these games were built for are ARMv5TE, but the interpreter
    /// only knows ARMv4T, where this encoding decodes as a data-processing
    /// operation with a register-controlled shift: it never writes `Rd`, so the
    /// destination keeps whatever the caller left there. Every software-float
    /// helper opens with `CLZ` to normalise a mantissa, so the count comes back
    /// as a stale value and the arithmetic downstream is meaningless — `B/美女
    /// 桌球.CBE` (issue #190) turns a ball's step size into zero that way and
    /// then spins through its per-pixel draw loop forever. Implementing the
    /// instruction here keeps the core untouched.
    fn handle_clz(&mut self, pc: u32) -> bool {
        let (instruction, destination, source, next, condition) = if self.cpu.thumb_mode() {
            // Thumb-2 `CLZ` is a 32-bit encoding: 111110101011 Rn 1111 Rd 1000 Rm.
            let prefix = self.memory.r16(pc);
            let suffix = self.memory.r16(pc + 2);
            if prefix & 0xfff0 != 0xfab0 || suffix & 0xf0f0 != 0xf080 {
                return false;
            }
            (
                prefix as u32,
                ((suffix >> 8) & 0x0f) as u8,
                (suffix & 0x0f) as u8,
                pc.wrapping_add(4),
                14,
            )
        } else {
            let word = self.memory.r32(pc);
            if word & 0x0fff_0ff0 != 0x016f_0f10 {
                return false;
            }
            (
                word,
                ((word >> 12) & 0x0f) as u8,
                (word & 0x0f) as u8,
                pc.wrapping_add(4),
                (word >> 28) as u8,
            )
        };
        let _ = instruction;
        self.cpu.reg_set(Mode::User, reg::PC, next);
        if arm_condition_met(condition, self.register(reg::CPSR)) {
            let value = self.register(source);
            self.cpu
                .reg_set(Mode::User, destination, value.leading_zeros());
        }
        true
    }

    fn handle_interworking_branch(&mut self, pc: u32) -> bool {
        if self.cpu.thumb_mode() {
            self.handle_thumb_blx(pc)
        } else {
            self.handle_arm_blx_immediate(pc)
        }
    }

    fn handle_arm_blx_immediate(&mut self, pc: u32) -> bool {
        let instruction = self.memory.r32(pc);
        let Some(target) = arm_blx_immediate_target(pc, instruction) else {
            return false;
        };
        self.cpu.reg_set(Mode::User, reg::LR, pc.wrapping_add(4));
        self.cpu.reg_set(Mode::User, reg::PC, target & !1);
        let cpsr = self.register(reg::CPSR) | (1 << 5);
        self.cpu.reg_set(Mode::User, reg::CPSR, cpsr);
        true
    }

    fn handle_thumb_blx_register(&mut self, pc: u32) -> bool {
        let instruction = self.memory.r16(pc);
        if instruction & 0xff87 != 0x4780 {
            return false;
        }
        let source = ((instruction >> 3) & 0x0f) as u8;
        let target = self.register(source);
        self.cpu.reg_set(Mode::User, reg::LR, pc + 3);
        self.cpu.reg_set(Mode::User, reg::PC, target & !1);
        let mut cpsr = self.register(reg::CPSR);
        if target & 1 != 0 {
            cpsr |= 1 << 5
        } else {
            cpsr &= !(1 << 5)
        }
        self.cpu.reg_set(Mode::User, reg::CPSR, cpsr);
        true
    }

    fn handle_thumb_blx_immediate(&mut self, pc: u32) -> bool {
        let prefix = self.memory.r16(pc);
        let suffix = self.memory.r16(pc + 2);
        if prefix & 0xf800 != 0xf000 || suffix & 0xf800 != 0xe800 {
            return false;
        }
        let raw_offset = (((prefix & 0x07ff) as u32) << 12) | (((suffix & 0x07ff) as u32) << 1);
        let offset = ((raw_offset << 9) as i32 >> 9) as u32;
        let target = pc.wrapping_add(4).wrapping_add(offset) & !3;
        self.cpu.reg_set(Mode::User, reg::LR, pc + 5);
        self.cpu.reg_set(Mode::User, reg::PC, target);
        let cpsr = self.register(reg::CPSR) & !(1 << 5);
        self.cpu.reg_set(Mode::User, reg::CPSR, cpsr);
        true
    }

    /// Detects the semihosting SWI in either instruction set: Thumb `SVC #0xAB`
    /// and ARM `SWI #0x123456` (the Angel interface also accepts `#0xAB`).
    fn is_semihosting_call(&mut self, pc: u32) -> bool {
        if self.cpu.thumb_mode() {
            self.memory.r16(pc) == 0xdfab
        } else {
            let instruction = self.memory.r32(pc);
            instruction & 0xff00_0000 == 0xef00_0000
                && matches!(instruction & 0x00ff_ffff, 0x123456 | 0xab)
        }
    }

    /// Handles a semihosting call and returns `true` when the guest terminated
    /// itself (semihosting exit), which halts the machine like a normal stop.
    fn handle_semihosting(&mut self, pc: u32) -> Result<bool> {
        let reason = self.register(0);
        let argument = self.register(1);
        match reason {
            3 => {
                if std::env::var("CBE_TRACE").is_ok() {
                    eprint!("{}", self.memory.r8(argument) as char);
                }
            }
            4 => {
                let mut bytes = Vec::new();
                for offset in 0..4096u32 {
                    let byte = self.memory.r8(argument.wrapping_add(offset));
                    if byte == 0 {
                        break;
                    }
                    bytes.push(byte);
                }
                let message = String::from_utf8_lossy(&bytes);
                if std::env::var("CBE_TRACE").is_ok() {
                    eprint!("{message}");
                }
            }
            0x18 => {
                // Angel ReportException: the guest C library called exit().
                // r1 holds the stop reason; 0x20026 is ADP_Stopped_ApplicationExit.
                info!("guest requested exit through semihosting report 0x{argument:08X}");
                self.cpu.reg_set(Mode::User, reg::PC, pc + 4);
                self.state = super::MachineState::Halted;
                return Ok(true);
            }
            _ => bail!("unhandled semihosting call reason={reason} at 0x{pc:08X}"),
        }
        let next_pc = if self.cpu.thumb_mode() {
            pc + 2
        } else {
            pc + 4
        };
        self.cpu.reg_set(Mode::User, reg::PC, next_pc);
        Ok(false)
    }

    fn handle_service(&mut self, address: u32) -> Result<()> {
        if address == NATIVE_DIRTY_RECT_SERVICE {
            self.handle_dirty_rectangle_service();
            self.return_from_service();
            return Ok(());
        }
        if address == LOG_NOOP_SERVICE {
            self.return_from_service();
            return Ok(());
        }
        if (super::CONTROL_METHOD_SERVICE..super::CONTROL_METHOD_SERVICE + 0x400).contains(&address)
        {
            self.handle_control_method(address - super::CONTROL_METHOD_SERVICE)?;
            self.return_from_service();
            return Ok(());
        }
        if (METHOD_STUB_BASE..METHOD_STUB_BASE + METHOD_STUB_KINDS * METHOD_STUB_STRIDE)
            .contains(&address)
        {
            let relative = address - METHOD_STUB_BASE;
            if relative / METHOD_STUB_STRIDE == super::METHOD_KIND_PANEL {
                self.handle_panel_service(relative % METHOD_STUB_STRIDE)?;
            } else if relative / METHOD_STUB_STRIDE == super::METHOD_KIND_GAMEOLD
                && relative % METHOD_STUB_STRIDE == 0x660
            {
                self.repaint_scene_window(self.register(0))?;
            } else {
                self.handle_method_stub(address);
            }
            self.return_from_service();
            return Ok(());
        }
        if (IO_METHOD_BASE..IO_METHOD_BASE + 0x2000).contains(&address) {
            let rel = address - IO_METHOD_BASE;
            if rel == IO_NV_READ_OFFSET {
                self.handle_nv_read();
            } else if rel == IO_NV_WRITE_OFFSET {
                self.handle_nv_write();
            } else {
                self.handle_file_service(rel / IO_METHOD_STRIDE);
            }
            self.return_from_service();
            return Ok(());
        }
        if (FIXED_MANAGER_INIT..FIXED_MANAGER_INIT + fixed_manager_specs().len() as u32 * 4)
            .contains(&address)
        {
            let index = ((address - FIXED_MANAGER_INIT) / 4) as usize;
            let (_, group, count) = fixed_manager_specs()[index];
            let destination = self.register(0);
            if destination != 0 {
                self.populate_table(destination, SERVICE_BASE + TABLE_STRIDE * group, count);
            }
            self.set_result(destination);
            self.return_from_service();
            return Ok(());
        }
        if (FIXED_MANAGER_GET..FIXED_MANAGER_GET + fixed_manager_specs().len() as u32 * 4)
            .contains(&address)
        {
            let index = ((address - FIXED_MANAGER_GET) / 4) as usize;
            let (_, group, _) = fixed_manager_specs()[index];
            // Return the shared dense function table so bootstrap code can
            // treat the manager as an object with callable slots.
            self.set_result(MANAGER_BASE + TABLE_STRIDE * (group + 1));
            self.return_from_service();
            return Ok(());
        }
        if [
            NATIVE_BILLING_PAYNUM,
            NATIVE_BILLING_REMAIN_DAY,
            NATIVE_BILLING_SEND_SMS,
        ]
        .contains(&address)
        {
            self.handle_native_billing_service(address);
            self.return_from_service();
            return Ok(());
        }
        if (OLDLIB_DRAW_SERVICE..OLDLIB_DRAW_SERVICE + 0x40).contains(&address) {
            let index = (address - OLDLIB_DRAW_SERVICE) / 4;
            self.handle_oldlib_draw_service(index);
            self.return_from_service();
            return Ok(());
        }
        if (FIXED_GAMEOLD_OBJECT_SERVICE..FIXED_GAMEOLD_OBJECT_SERVICE + 15 * 4).contains(&address)
        {
            let index = (address - FIXED_GAMEOLD_OBJECT_SERVICE) / 4;
            self.handle_fixed_gameold_object_service(index);
            self.return_from_service();
            return Ok(());
        }
        if (FIXED_GAMEOLD_REGION_SERVICE..FIXED_GAMEOLD_REGION_SERVICE + 8 * 4).contains(&address) {
            let index = (address - FIXED_GAMEOLD_REGION_SERVICE) / 4;
            self.handle_fixed_gameold_region_service(index)?;
            self.return_from_service();
            return Ok(());
        }
        if address == NATIVE_DISPATCH_SERVICE {
            self.handle_native_dispatch_service();
            self.return_from_service();
            return Ok(());
        }
        if (NATIVE_SYSTEM_TIME_SERVICE..NATIVE_SYSTEM_TIME_SERVICE + 6 * 4).contains(&address) {
            let index = (address - NATIVE_SYSTEM_TIME_SERVICE) / 4;
            let value = match index {
                0 => 2026,
                1 => 8,
                2 => 7,
                3 => 12,
                4 => 0,
                _ => 0,
            };
            self.set_result(value);
            self.return_from_service();
            return Ok(());
        }
        if (MEMORY_BLOCK_SERVICE..MEMORY_BLOCK_SERVICE + 12).contains(&address) {
            let index = (address - MEMORY_BLOCK_SERVICE) / 4;
            *self.service_calls.entry((27, index)).or_default() += 1;
            self.handle_memory_block_service(index);
            self.return_from_service();
            return Ok(());
        }
        let offset = address - SERVICE_BASE;
        let group = offset / TABLE_STRIDE;
        let index = (offset % TABLE_STRIDE) / 4;
        *self.service_calls.entry((group, index)).or_default() += 1;
        let trace_service = service_trace_enabled(group, index);
        if self.recent_services.len() == 16 {
            self.recent_services.pop_front();
        }
        self.recent_services
            .push_back((group, index, self.register(reg::LR), self.register(0)));
        if trace_service {
            eprintln!(
                "service group={group} index={index} r0={:08X} r1={:08X} r2={:08X} r3={:08X} lr={:08X}",
                self.register(0),
                self.register(1),
                self.register(2),
                self.register(3),
                self.register(reg::LR),
            );
        }
        match group {
            0 => self.handle_root_service(index),
            1 => self.handle_system_service(index),
            2 => self.handle_memory_service(index),
            3 => {
                self.handle_game_service(index);
                if index == 77 && self.uses_fixed_manager_abi() {
                    self.initialize_scene_resources(self.register(0))?;
                }
            }
            4 => self.handle_lcd_service(index),
            5 => self.handle_file_service(index),
            6 => self.handle_stdio_service(index),
            7 => self.handle_timer_service(index),
            8 => self.handle_ctrl_service(index),
            9 => self.handle_network_service(index),
            10 => self.handle_game_util_service(index),
            11 => self.handle_df_engine_service(index),
            12 => self.handle_billing_service(index),
            13 => self.handle_ucs2_service(index),
            14 => self.handle_screen_service(index),
            16 => self.handle_game_lcd_service(index),
            17 => self.handle_manager_service(index),
            18 => self.handle_audio_service(index),
            29 => self.handle_net_app_service(index),
            20 => {
                if index == 6 {
                    let descriptor = self.register(0);
                    let destination = self.memory.r32(descriptor);
                    let capacity = self.memory.r16(descriptor + 4) as u32;
                    if destination != 0 {
                        self.populate_table(
                            destination,
                            SERVICE_BASE + TABLE_STRIDE * 28,
                            (capacity / 4).min(40),
                        );
                    }
                    self.set_result(APP_STORE_MANAGER);
                } else if address == DATA_PACKAGE_FULL_PATH_SERVICE {
                    // DF_DataPackage_GetFullPaths, reached by the boot
                    // loader's `ptr - 52` arithmetic on the data package's
                    // DP_GetFileID slot.  Must hand back a buffer that
                    // starts with the UCS2 path of the container the loader
                    // reopens; returning zero left it opening a NULL name.
                    let buf = self.data_package_full_path();
                    self.set_result(buf);
                } else {
                    self.set_result(0);
                }
            }
            21 => self.handle_data_package_service(index),
            22 => self.handle_download_service(index),
            23 => self.handle_record_service(index),
            24 => self.handle_payment_service(index),
            25 => self.handle_download_resource_service(index),
            26 => self.handle_download_image_service(index),
            28 => self.set_result(u32::from(index == 30)),
            _ => self.set_result(0),
        }
        if trace_service {
            eprintln!("service result r0={:08X}", self.register(0));
        }
        self.return_from_service();
        Ok(())
    }

    pub(super) fn register(&self, register: armv4t_emu::reg::Reg) -> u32 {
        self.cpu.reg_get(Mode::User, register)
    }

    /// Firmware argument `n` in the AAPCS sense: the first four arrive in
    /// r0..r3 and the rest are on the caller's stack.
    pub(super) fn argument(&mut self, n: u32) -> u32 {
        if n < 4 {
            return self.register(n as u8);
        }
        let sp = self.register(reg::SP);
        self.memory.r32(sp.wrapping_add((n - 4) * 4))
    }

    pub(super) fn set_result(&mut self, value: u32) {
        self.cpu.reg_set(Mode::User, 0, value);
    }

    pub(super) fn return_from_service(&mut self) {
        let target = self.register(reg::LR);
        self.cpu.reg_set(Mode::User, reg::PC, target & !1);
        let mut cpsr = self.register(reg::CPSR);
        if target & 1 != 0 {
            cpsr |= 1 << 5
        } else {
            cpsr &= !(1 << 5)
        }
        self.cpu.reg_set(Mode::User, reg::CPSR, cpsr);
    }

    pub(super) fn invoke_callback(
        &mut self,
        entry: u32,
        r0: u32,
        r1: u32,
        r2: u32,
        instruction_limit: u64,
    ) -> Result<()> {
        if entry == 0 {
            return Ok(());
        }
        self.cpu.reg_set(Mode::User, 0, r0);
        self.cpu.reg_set(Mode::User, 1, r1);
        self.cpu.reg_set(Mode::User, 2, r2);
        self.cpu.reg_set(Mode::User, reg::LR, EXIT_ADDRESS | 1);
        self.cpu.reg_set(Mode::User, reg::PC, entry & !1);
        let mut cpsr = self.cpu.reg_get(Mode::User, reg::CPSR);
        if entry & 1 != 0 {
            cpsr |= 1 << 5;
        } else {
            cpsr &= !(1 << 5);
        }
        self.cpu.reg_set(Mode::User, reg::CPSR, cpsr);
        self.run_until_return(instruction_limit)
    }

    /// Invoke a guest callback with the four-argument firmware ABI used by
    /// asynchronous network events (`r3` carries the event type).
    pub(super) fn invoke_callback_with_r3(
        &mut self,
        entry: u32,
        r0: u32,
        r1: u32,
        r2: u32,
        r3: u32,
        instruction_limit: u64,
    ) -> Result<()> {
        if entry == 0 {
            return Ok(());
        }
        self.cpu.reg_set(Mode::User, 0, r0);
        self.cpu.reg_set(Mode::User, 1, r1);
        self.cpu.reg_set(Mode::User, 2, r2);
        self.cpu.reg_set(Mode::User, 3, r3);
        self.cpu.reg_set(Mode::User, reg::LR, EXIT_ADDRESS | 1);
        self.cpu.reg_set(Mode::User, reg::PC, entry & !1);
        let mut cpsr = self.cpu.reg_get(Mode::User, reg::CPSR);
        if entry & 1 != 0 {
            cpsr |= 1 << 5;
        } else {
            cpsr &= !(1 << 5);
        }
        self.cpu.reg_set(Mode::User, reg::CPSR, cpsr);
        self.run_until_return(instruction_limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_blx_immediate_switch_target_includes_h_bit() {
        assert_eq!(
            arm_blx_immediate_target(0x0102_e6d4, 0xfa00_0004),
            Some(0x0102_e6ec)
        );
        assert_eq!(arm_blx_immediate_target(0x1000, 0xfb00_0000), Some(0x100a));
        assert_eq!(arm_blx_immediate_target(0x1000, 0xea00_0000), None);
    }

    #[test]
    fn thumb_add_pc_preserves_halfword_aligned_program_counter() {
        assert_eq!(
            thumb_add_pc_target(0x0100_112c, 0x449f, 0x18),
            Some(0x0100_1148)
        );
        assert_eq!(
            thumb_add_pc_target(0x0100_81e6, 0x449f, 0x20),
            Some(0x0100_820a)
        );
        assert_eq!(thumb_add_pc_target(0x1000, 0x4478, 4), None);
    }
}
