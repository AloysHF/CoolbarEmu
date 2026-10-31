//! Memory manager services (groups 2 and 27).

use armv4t_emu::Memory;

use super::super::{CoolbarMachine, MEMORY_BLOCK_POOL, MEMORY_BLOCK_PTR, MEMORY_BLOCK_SERVICE};

impl CoolbarMachine {
    pub(crate) fn handle_memory_service(&mut self, index: u32) {
        match index {
            2 => {
                let destination = self.register(0);
                let size = self.register(1).max(2);
                let pointer = self.allocate(size);
                self.memory.w32(destination, pointer);
                self.set_result(1);
            }
            3 => {
                let destination = self.register(0);
                if destination != 0 {
                    let pointer = self.memory.r32(destination);
                    self.deallocate(pointer);
                    self.memory.w32(destination, 0);
                }
                self.set_result(0);
            }
            5 => {
                let block = self.register(0);
                let size = self.register(1);
                self.initialize_memory_block(block, size);
                self.set_result(block);
            }
            6 => {
                let block = self.register(0);
                let size = self.register(1);
                let pointer = self.allocate_from_memory_block(block, size);
                self.set_result(pointer);
            }
            7 => {
                let block = self.register(0);
                self.reset_memory_block(block);
                self.set_result(block);
            }
            8 => self.set_result(MEMORY_BLOCK_PTR),
            9 => {
                self.initialize_memory_block(MEMORY_BLOCK_PTR, 0x40_0000);
                self.set_result(MEMORY_BLOCK_PTR);
            }
            10 | 11 => {
                if self.memory.r32(MEMORY_BLOCK_PTR) == 0 {
                    self.initialize_memory_block(MEMORY_BLOCK_PTR, 0x40_0000);
                } else {
                    self.reset_memory_block(MEMORY_BLOCK_PTR);
                }
                self.set_result(MEMORY_BLOCK_PTR);
            }
            12 => {
                let size = self.register(0);
                if self.memory.r32(MEMORY_BLOCK_PTR) == 0 {
                    self.initialize_memory_block(MEMORY_BLOCK_PTR, 0x40_0000);
                }
                let pointer = self.allocate_from_memory_block(MEMORY_BLOCK_PTR, size);
                self.set_result(pointer);
            }
            13 => {
                let size = self.register(0).max(1);
                let pointer = self.allocate(size);
                self.set_result(pointer);
            }
            14 => {
                self.deallocate(self.register(0));
                self.set_result(0);
            }
            _ => self.set_result(0),
        }
    }

    pub(crate) fn initialize_memory_block(&mut self, block: u32, size: u32) {
        let base = if block == MEMORY_BLOCK_PTR {
            MEMORY_BLOCK_POOL
        } else {
            self.allocate(size)
        };
        self.release_memory_block_bookkeeping(block, base);
        self.memory.w32(block, base);
        self.memory.w32(block + 4, 0);
        self.memory.w32(block + 8, size);
        self.memory.w32(block + 12, MEMORY_BLOCK_SERVICE);
        self.memory.w32(block + 16, MEMORY_BLOCK_SERVICE + 4);
        self.memory.w32(block + 20, MEMORY_BLOCK_SERVICE + 8);
    }

    pub(crate) fn handle_memory_block_service(&mut self, index: u32) {
        let block = self.register(0);
        match index {
            0 => {
                let requested = self.register(1);
                let pointer = self.allocate_from_memory_block(block, requested);
                self.set_result(pointer);
            }
            1 => {
                self.reset_memory_block(block);
                self.set_result(block);
            }
            2 => {
                let base = self.memory.r32(block);
                self.release_memory_block_bookkeeping(block, base);
                if block != MEMORY_BLOCK_PTR {
                    self.deallocate(base);
                }
                self.memory.w32(block, 0);
                self.memory.w32(block + 4, 0);
                self.memory.w32(block + 8, 0);
                self.set_result(0);
            }
            _ => self.set_result(0),
        }
    }

    /// Allocate `requested` bytes inside a guest memory block.
    ///
    /// The firmware block manager is a general allocator, not a stack: guest
    /// code frees individual pointers (issue #106's scene rebuild allocates
    /// 11,490 objects and frees 1,411 of them per session). A pure bump
    /// pointer therefore ran the 4 MiB pool dry even though most of it was
    /// already dead. Fitting recycled space first keeps the pool's lifetime
    /// tied to its live set, which is what the guest expects.
    pub(crate) fn allocate_from_memory_block(&mut self, block: u32, requested: u32) -> u32 {
        let aligned = requested.saturating_add(3) & !3;
        let base = self.memory.r32(block);
        if base == 0 || aligned == 0 {
            return 0;
        }
        if let Some(offset) = self.take_block_free_space(base, aligned) {
            let pointer = base + offset;
            self.clear_allocation(pointer, aligned);
            self.block_allocations.insert(pointer, (aligned, base));
            return pointer;
        }
        let offset = self.memory.r32(block + 4);
        let size = self.memory.r32(block + 8);
        if offset.saturating_add(aligned) <= size {
            self.memory.w32(block + 4, offset + aligned);
            let pointer = base + offset;
            self.clear_allocation(pointer, aligned);
            self.block_allocations.insert(pointer, (aligned, base));
            pointer
        } else {
            0
        }
    }

    /// Fetch `size` bytes of recycled space inside `base`, splitting the
    /// first free run that fits. Returns the block-relative offset.
    fn take_block_free_space(&mut self, base: u32, size: u32) -> Option<u32> {
        let runs = self.block_free_runs.get_mut(&base)?;
        let index = runs.iter().position(|&(_, length)| length >= size)?;
        let (offset, length) = runs[index];
        if length == size {
            runs.remove(index);
        } else {
            runs[index] = (offset + size, length - size);
        }
        Some(offset)
    }

    /// Return a block pointer to its block's recycle list, coalescing with
    /// neighbours so fragmentation cannot rebuild the old exhaustion.
    pub(crate) fn release_block_allocation(&mut self, pointer: u32) -> bool {
        let Some((size, base)) = self.block_allocations.remove(&pointer) else {
            return false;
        };
        let offset = pointer.wrapping_sub(base);
        let runs = self.block_free_runs.entry(base).or_default();
        runs.push((offset, size));
        runs.sort_unstable();
        let mut index = 0;
        while index + 1 < runs.len() {
            let (start, length) = runs[index];
            let (next_start, next_length) = runs[index + 1];
            if start.saturating_add(length) == next_start {
                runs[index].1 = length.saturating_add(next_length);
                runs.remove(index + 1);
            } else {
                index += 1;
            }
        }
        true
    }

    /// Drop the recycle state of one block, e.g. on an explicit reset.
    fn reset_memory_block(&mut self, block: u32) {
        let base = self.memory.r32(block);
        self.release_memory_block_bookkeeping(block, base);
        self.memory.w32(block + 4, 0);
    }

    fn release_memory_block_bookkeeping(&mut self, block: u32, base: u32) {
        self.block_free_runs.remove(&base);
        self.block_allocations
            .retain(|_, (_, owner)| *owner != base);
        let _ = block;
    }

    fn clear_allocation(&mut self, pointer: u32, size: u32) {
        if let Some(region) = self.memory.region_mut(pointer, size as usize) {
            // `region_mut` yields the whole mapped span; clear only the new
            // allocation or the block headers living above the pool vanish.
            let start = pointer.saturating_sub(region.base) as usize;
            let end = (start + size as usize).min(region.data.len());
            region.data[start..end].fill(0);
        }
    }
}

/// `block`'s header layout: base at +0, bump offset at +4, capacity at +8.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_memory_block_returns_zeroed_allocations() {
        let mut machine = CoolbarMachine::new_blank_for_tests();
        machine.initialize_memory_block(MEMORY_BLOCK_PTR, 64);
        let pointer = machine.allocate_from_memory_block(MEMORY_BLOCK_PTR, 5);
        machine.memory.write_bytes(pointer, &[0xff; 8]);
        machine.memory.w32(MEMORY_BLOCK_PTR + 4, 0);
        assert_eq!(
            machine.allocate_from_memory_block(MEMORY_BLOCK_PTR, 5),
            pointer
        );
        for offset in 0..8 {
            assert_eq!(machine.memory.r8(pointer + offset), 0);
        }
    }

    /// The firmware block manager is not a stack: a freed pointer must be
    /// handed out again instead of growing the bump offset forever.
    #[test]
    fn freed_block_allocation_is_reused() {
        let mut machine = CoolbarMachine::new_blank_for_tests();
        machine.initialize_memory_block(MEMORY_BLOCK_PTR, 64);
        let first = machine.allocate_from_memory_block(MEMORY_BLOCK_PTR, 16);
        let second = machine.allocate_from_memory_block(MEMORY_BLOCK_PTR, 16);
        assert_ne!(first, second);
        assert!(machine.release_block_allocation(first));
        assert_eq!(
            machine.allocate_from_memory_block(MEMORY_BLOCK_PTR, 16),
            first
        );
        for offset in 0..16 {
            assert_eq!(machine.memory.r8(first + offset), 0);
        }
        // The live allocation is untouched and still owned by the block.
        assert_eq!(machine.memory.r32(MEMORY_BLOCK_PTR + 4), 32);
        assert!(machine.block_allocations.contains_key(&second));
    }

    #[test]
    fn adjacent_block_frees_coalesce_into_a_larger_run() {
        let mut machine = CoolbarMachine::new_blank_for_tests();
        machine.initialize_memory_block(MEMORY_BLOCK_PTR, 64);
        let a = machine.allocate_from_memory_block(MEMORY_BLOCK_PTR, 12);
        let b = machine.allocate_from_memory_block(MEMORY_BLOCK_PTR, 12);
        let c = machine.allocate_from_memory_block(MEMORY_BLOCK_PTR, 12);
        assert!(machine.release_block_allocation(a));
        assert!(machine.release_block_allocation(b));
        let base = machine.memory.r32(MEMORY_BLOCK_PTR);
        let runs = machine
            .block_free_runs
            .get(&base)
            .cloned()
            .unwrap_or_default();
        assert_eq!(runs, vec![(a - base, 24)]);
        // The coalesced run serves a request neither half could satisfy.
        assert_eq!(machine.allocate_from_memory_block(MEMORY_BLOCK_PTR, 20), a);
        assert_ne!(c, a);
    }

    #[test]
    fn block_reset_discards_recycle_state() {
        let mut machine = CoolbarMachine::new_blank_for_tests();
        machine.initialize_memory_block(MEMORY_BLOCK_PTR, 64);
        let pointer = machine.allocate_from_memory_block(MEMORY_BLOCK_PTR, 16);
        assert!(machine.release_block_allocation(pointer));
        machine.reset_memory_block(MEMORY_BLOCK_PTR);
        assert!(machine.block_free_runs.is_empty());
        assert!(machine.block_allocations.is_empty());
        assert_eq!(machine.memory.r32(MEMORY_BLOCK_PTR + 4), 0);
    }
}
