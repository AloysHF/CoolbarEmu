//! Bridge scene regions and XSE host calls to the guest application.

use std::collections::BTreeMap;

use armv4t_emu::Memory;
use serde::{Deserialize, Serialize};

use super::super::CoolbarMachine;
use super::scene::SceneRegion;
use crate::cbe::xse::{Script, Step, Value};

const REGION_COUNT: u32 = 0x614;
const REGION_TABLE: u32 = 0x618;
const REGION_SIZE: u32 = 0x14;
const INSTRUCTION_BUDGET: usize = 4096;

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Runtime {
    commands: BTreeMap<u32, BTreeMap<String, u32>>,
    tables: BTreeMap<u32, (u32, u32)>,
    scripts: BTreeMap<u32, ScriptSlot>,
    #[serde(skip)]
    current_call: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct ScriptSlot {
    script: Script,
    strings: BTreeMap<u32, u32>,
}

impl CoolbarMachine {
    pub(super) fn load_scene_scripts(&mut self, scene: u32, regions: &[SceneRegion<'_>]) {
        let owner = self.register(9);
        if !self.xse.commands.contains_key(&owner) {
            return;
        }
        if let Some((old_table, count)) = self.xse.tables.remove(&scene) {
            for index in 0..count {
                if let Some(slot) = self.xse.scripts.remove(&(old_table + index * REGION_SIZE)) {
                    for address in slot.strings.into_values() {
                        self.deallocate(address);
                    }
                }
            }
            self.deallocate(old_table);
            self.memory.w16(scene + REGION_COUNT, 0);
            self.memory.w32(scene + REGION_TABLE, 0);
        }

        let scripts: Vec<_> = regions
            .iter()
            .filter_map(|region| {
                if !region.name.ends_with(b".xse") || region.kind > 2 {
                    return None;
                }
                let bytes = self.scene_resource_bytes(region.name)?;
                Some((region, Script::parse(&bytes)?))
            })
            .collect();
        if scripts.is_empty() {
            return;
        }

        let count = scripts.len() as u32;
        let table = self.allocate(count * REGION_SIZE);
        if table == 0 {
            return;
        }
        for (index, (region, script)) in scripts.into_iter().enumerate() {
            let entry = table + index as u32 * REGION_SIZE;
            for (offset, value) in [
                region.center[0],
                region.center[1],
                region.size[0],
                region.size[1],
                region.kind,
            ]
            .into_iter()
            .enumerate()
            {
                self.memory.w16(entry + offset as u32 * 2, value);
            }
            self.memory.w32(entry + 0xc, entry);
            self.memory.w8(entry + 0x10, 1);
            self.xse.scripts.insert(
                entry,
                ScriptSlot {
                    script,
                    strings: BTreeMap::new(),
                },
            );
        }
        self.memory.w16(scene + REGION_COUNT, count as u16);
        self.memory.w32(scene + REGION_TABLE, table);
        self.xse.tables.insert(scene, (table, count));
    }

    pub(super) fn handle_xse_service(&mut self, index: u32) -> bool {
        let handle = self.register(0);
        let parameter = self.register(1);
        match index {
            // The global scope uses -1 in the firmware's script API.
            118 if handle == u32::MAX => {
                let name = self.read_c_string(parameter, 64).to_ascii_uppercase();
                let callback = self.register(2);
                if !name.is_empty() && self.memory.region(callback & !1, 2).is_some() {
                    self.xse
                        .commands
                        .entry(self.register(9))
                        .or_default()
                        .insert(name, callback);
                }
            }
            114 if handle == u32::MAX && !self.xse.scripts.is_empty() => {
                if self.xse.current_call.is_none() {
                    let handles: Vec<_> = self
                        .xse
                        .scripts
                        .iter()
                        .filter_map(|(&handle, slot)| slot.script.is_running().then_some(handle))
                        .collect();
                    for handle in handles {
                        self.run_xse_script(handle);
                    }
                }
            }
            116 | 117 if self.xse.scripts.contains_key(&handle) => {
                let function = self.read_c_string(parameter, 64).to_ascii_uppercase();
                if let Some(slot) = self.xse.scripts.get_mut(&handle) {
                    slot.script.start(&function);
                }
                if index == 116 && self.xse.current_call.is_none() {
                    self.run_xse_script(handle);
                }
            }
            119 if self.xse.scripts.contains_key(&handle) => {}
            120 if self.xse.scripts.contains_key(&handle) => {
                self.xse.scripts.get_mut(&handle).unwrap().script.stop();
            }
            126 | 127 if self.xse.current_call == Some(handle) => {
                let result = if index == 127 {
                    Value::Integer(self.register(2) as i32)
                } else {
                    Value::Zero
                };
                if let Some(slot) = self.xse.scripts.get_mut(&handle) {
                    slot.script.return_from_host(parameter as usize, result);
                }
            }
            130 | 132 if self.xse.current_call == Some(handle) => {
                let value = self
                    .xse
                    .scripts
                    .get(&handle)
                    .and_then(|slot| slot.script.parameter(parameter as usize))
                    .unwrap_or_default();
                let result = if index == 132 {
                    self.xse_string_address(handle, value)
                } else {
                    value.integer() as u32
                };
                self.set_result(result);
                return true;
            }
            _ => return false,
        }
        self.set_result(0);
        true
    }

    fn xse_string_address(&mut self, handle: u32, value: Value) -> u32 {
        let Value::String(index) = value else {
            return 0;
        };
        let Some(slot) = self.xse.scripts.get(&handle) else {
            return 0;
        };
        if let Some(&address) = slot.strings.get(&index) {
            return address;
        }
        let Some(bytes) = slot.script.string(index) else {
            return 0;
        };
        let bytes = bytes.to_vec();
        let address = self.allocate(bytes.len() as u32 + 1);
        if address != 0 {
            self.memory.write_bytes(address, &bytes);
            self.xse
                .scripts
                .get_mut(&handle)
                .unwrap()
                .strings
                .insert(index, address);
        }
        address
    }

    fn run_xse_script(&mut self, handle: u32) {
        let static_base = self.register(9);
        for _ in 0..INSTRUCTION_BUDGET {
            let Some(slot) = self.xse.scripts.get_mut(&handle) else {
                return;
            };
            match slot.script.step() {
                Step::Continue => {}
                Step::Finished => return,
                Step::Host(index) => {
                    let callback = self
                        .xse
                        .commands
                        .get(&static_base)
                        .and_then(|commands| commands.get(slot.script.command(index)?))
                        .copied();
                    let Some(callback) = callback else {
                        slot.script.stop();
                        return;
                    };
                    let cpu = self.cpu;
                    self.xse.current_call = Some(handle);
                    let result = self.invoke_callback(
                        callback,
                        handle,
                        0,
                        0,
                        crate::DEFAULT_INSTRUCTION_LIMIT,
                    );
                    self.cpu = cpu;
                    self.xse.current_call = None;
                    if let Err(error) = result {
                        log::warn!("XSE host callback failed: {error:#}");
                        if let Some(slot) = self.xse.scripts.get_mut(&handle) {
                            slot.script.stop();
                        }
                        return;
                    }
                }
            }
        }
        if let Some(slot) = self.xse.scripts.get_mut(&handle) {
            slot.script.stop();
        }
    }

    pub(in crate::machine) fn restore_xse_allocations(&mut self) {
        for &(table, count) in self.xse.tables.values() {
            self.heap_allocations
                .insert(table, (count * REGION_SIZE + 7) & !7);
        }
        for slot in self.xse.scripts.values() {
            for (&index, &address) in &slot.strings {
                if let Some(bytes) = slot.script.string(index) {
                    self.heap_allocations
                        .insert(address, (bytes.len() as u32 + 8) & !7);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{save_state, CbeArchive, MachineState};
    use std::path::PathBuf;

    #[test]
    #[ignore = "requires the local issue #257 CBE fixture (set COOLBAR_GAME_DIR)"]
    fn street_npc_opens_and_advances_native_dialogue() {
        let directory = PathBuf::from(std::env::var_os("COOLBAR_GAME_DIR").unwrap());
        let archive = CbeArchive::load(&directory.join("卧底风云(精品).CBE")).unwrap();
        let mut machine = CoolbarMachine::new(&archive).unwrap();
        let limit = 20_000_000;
        machine.boot(limit).unwrap();

        // This fixture's native SHOWDIALOG handler stores the active text
        // pointer in its module's static data.
        let text = |machine: &mut CoolbarMachine| {
            let address = machine.memory.r32(machine.register(9) + 0x2efc);
            if address == 0 {
                return String::new();
            }
            machine.read_gbk_string(address, 256)
        };

        let keys = [
            (30, 14),
            (60, 14),
            (100, 14),
            (120, 14),
            (140, 4),
            (160, 4),
            (180, 4),
            (200, 4),
            (220, 4),
            (240, 4),
            (260, 2),
            (280, 2),
            (300, 2),
            (320, 2),
            (340, 2),
            (360, 2),
            (380, 2),
            (400, 2),
            (460, 14),
            (500, 14),
            (550, 14),
            (610, 14),
            (670, 14),
        ];
        let mut snapshot = vec![0; save_state::SERIALIZED_SIZE];
        for frame in 0..700 {
            for &(at, key) in &keys {
                if frame == at || frame == at + 1 {
                    machine.set_key(key, frame == at);
                }
            }
            machine.run_frame(limit).unwrap();
            assert_eq!(machine.state(), MachineState::Ready);

            match frame {
                135 | 455 => assert!(text(&mut machine).is_empty()),
                490 => {
                    assert!(text(&mut machine).contains("年轻人"));
                    save_state::encode_machine(&machine, 257, &mut snapshot).unwrap();
                    machine = save_state::decode_machine(&snapshot, 257).unwrap();
                    assert!(text(&mut machine).contains("年轻人"));
                }
                530 => assert!(text(&mut machine).contains("怎么挣")),
                585 => assert!(text(&mut machine).contains("停车场")),
                650 => assert!(text(&mut machine).contains("没问题")),
                699 => assert!(text(&mut machine).contains("瘦三儿")),
                _ => {}
            }
        }
    }
}
