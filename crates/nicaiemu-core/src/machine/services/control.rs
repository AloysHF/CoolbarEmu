//! Firmware list controls with guest-owned items and screen callbacks.

use armv4t_emu::Memory;

use super::super::{NicaiMachine, CONTROL_METHOD_SERVICE, SCREEN_IMAGE_STRUCT};

const SCREEN: u32 = 0x18;
const SELECTED: u32 = 0x170;
const CAPACITY: u32 = 0x140;
const COUNT: u32 = 0x144;
const ITEMS: u32 = 0x148;
const CALLBACK: u32 = 0x14c;
const RECT: u32 = 0x150;
const ROW_HEIGHT: u32 = 0x158;
const TEXT_COLOR: u32 = 0x15c;
const BACK_COLOR: u32 = 0x160;
const HIGHLIGHT: u32 = 0x164;
const TITLE_IMAGE: u32 = 0x168;
const BACK_IMAGE: u32 = 0x16c;
const SELECTED_COLOR: u32 = 0x174;
const FOOTER_IMAGE: u32 = 0x178;
const LEFT_TEXT: u32 = 0x17c;
const RIGHT_TEXT: u32 = 0x180;
const SOFT_COLOR: u32 = 0x184;
const ITEM_ICON: u32 = 0x188;

impl NicaiMachine {
    pub(crate) fn initialize_list_control(&mut self) {
        let slot = self.register(0);
        let capacity = self.register(1) & 0xffff;
        if slot == 0 || capacity == 0 || capacity > 1024 {
            self.set_result(0);
            return;
        }
        let object = self.allocate(0x200);
        let items = self.allocate(capacity * 8);
        if object == 0 || items == 0 {
            self.deallocate(object);
            self.deallocate(items);
            self.set_result(0);
            return;
        }
        self.memory.write_bytes(object, &[0; 0x200]);
        self.memory
            .write_bytes(items, &vec![0; capacity as usize * 8]);
        for offset in (0xd0..=0x138).step_by(4) {
            self.memory
                .w32(object + offset, (CONTROL_METHOD_SERVICE + offset) | 1);
        }
        self.memory
            .w32(object + SCREEN + 8, (CONTROL_METHOD_SERVICE + 0x204) | 1);
        self.memory
            .w32(object + SCREEN + 12, (CONTROL_METHOD_SERVICE + 0x200) | 1);
        self.memory.w32(object + CAPACITY, capacity);
        self.memory.w32(object + ITEMS, items);
        let callback = self.argument(6);
        self.memory.w32(object + CALLBACK, callback);
        self.memory.w32(object + RECT, self.register(2));
        self.memory.w32(object + RECT + 4, self.register(3));
        self.memory.w32(object + ROW_HEIGHT, 25);
        self.memory.w32(object + TEXT_COLOR, 0xffff);
        self.memory.w32(object + SELECTED_COLOR, 0xffff);
        self.memory.w32(object + SOFT_COLOR, 0xffff);
        self.memory.w32(object + BACK_COLOR, 0x2104);
        self.memory.w32(object + HIGHLIGHT, 0x001f);
        self.memory.w32(slot, object);
        self.screen_stack.push(object + SCREEN);
        self.pending_screen = object + SCREEN;
        self.set_result(1);
    }

    pub(crate) fn handle_control_method(&mut self, offset: u32) -> anyhow::Result<()> {
        let object = self.register(0);
        match offset {
            0xd8 => {
                let count = self.memory.r32(object + COUNT);
                let capacity = self.memory.r32(object + CAPACITY);
                let items = self.memory.r32(object + ITEMS);
                if count < capacity {
                    self.memory.w32(items + count * 8, self.register(1));
                    self.memory.w32(items + count * 8 + 4, self.register(2));
                    self.memory.w32(object + COUNT, count + 1);
                    self.set_result(1);
                } else {
                    self.set_result(0);
                }
            }
            0xe4 | 0x104 | 0x110 => {
                let field = match offset {
                    0xe4 => BACK_IMAGE,
                    0x104 => ITEM_ICON,
                    _ => TITLE_IMAGE,
                };
                self.memory.w32(object + field, self.register(1));
                self.set_result(0);
            }
            0xec | 0xf0 | 0x118 => {
                let field = match offset {
                    0xec => TEXT_COLOR,
                    0xf0 => BACK_COLOR,
                    _ => SOFT_COLOR,
                };
                self.memory.w32(object + field, self.register(1));
                if offset == 0xec {
                    self.memory.w32(object + SELECTED_COLOR, self.register(2));
                } else if offset == 0xf0 {
                    self.memory.w32(object + HIGHLIGHT, self.register(2));
                }
                self.set_result(0);
            }
            0x114 => {
                self.memory.w32(object + FOOTER_IMAGE, self.register(1));
                self.memory.w32(object + LEFT_TEXT, self.register(2));
                let right = self.argument(4);
                self.memory.w32(object + RIGHT_TEXT, right);
                self.set_result(0);
            }
            0x100 => {
                self.memory
                    .w32(object + ROW_HEIGHT, self.register(1).clamp(16, 128));
                self.set_result(0);
            }
            0x11c => {
                let selected = self.memory.r32(object + SELECTED);
                self.set_result(selected);
            }
            0x12c => {
                let count = self.memory.r32(object + COUNT);
                self.memory.w32(
                    object + SELECTED,
                    self.register(1).min(count.saturating_sub(1)),
                );
                self.set_result(0);
            }
            0x138 => {
                let control = self.memory.r32(object);
                let items = self.memory.r32(control + ITEMS);
                self.cpu
                    .reg_set(armv4t_emu::Mode::User, 0, control + SCREEN);
                self.handle_screen_service(6);
                self.deallocate(items);
                self.deallocate(control);
                self.memory.w32(object, 0);
                self.set_result(0);
            }
            0x200 => {
                self.render_list_control(object);
                self.set_result(0);
            }
            0x204 => self.update_list_control(object),
            _ => self.set_result(0),
        }
        Ok(())
    }

    fn update_list_control(&mut self, object: u32) {
        let count = self.memory.r32(object + COUNT);
        let selected = self
            .memory
            .r32(object + SELECTED)
            .min(count.saturating_sub(1));
        let event = self.register(1);
        if event == 0 || (event == 6 && self.native_app_parser != 0) {
            let keys = if event == 0 {
                self.memory.r32(self.register(2))
            } else {
                self.key_down
            };
            let next = if keys & (1 << 17) != 0 {
                selected.saturating_sub(1)
            } else if keys & (1 << 18) != 0 {
                (selected + 1).min(count.saturating_sub(1))
            } else {
                selected
            };
            self.memory.w32(object + SELECTED, next);
            let event = if keys & (1 << 14) != 0 {
                Some(2)
            } else if keys & (1 << 12) != 0 {
                Some(0)
            } else if keys & (1 << 13) != 0 {
                Some(1)
            } else {
                None
            };
            if let Some(event) = event {
                let callback = self.memory.r32(object + CALLBACK);
                self.defer_callback(callback, vec![object, event], "listControl");
            }
        }
        self.set_result(1);
    }

    fn render_list_control(&mut self, object: u32) {
        let x = self.memory.r16(object + RECT) as i16 as i32;
        let y = self.memory.r16(object + RECT + 2) as i16 as i32;
        let width = self.memory.r16(object + RECT + 4) as i16 as i32;
        let height = self.memory.r16(object + RECT + 6) as i16 as i32;
        let back = self.memory.r32(object + BACK_COLOR) as u16;
        self.fill_screen_rect(x, y, width, height, back);
        let image = self.memory.r32(object + BACK_IMAGE);
        if image != 0 {
            let (_, image_height) = self.image_dims(image);
            let image_y = if image_height == self.display_surface.height as i32 {
                0
            } else {
                y
            };
            self.draw_image_at(image, x, image_y, false);
        }
        let image = self.memory.r32(object + TITLE_IMAGE);
        if image != 0 {
            self.draw_image_at(image, x, 0, false);
        }
        let footer = self.memory.r32(object + FOOTER_IMAGE);
        let (_, footer_height) = self.image_dims(footer);
        let footer_y = self.display_surface.height as i32 - footer_height;
        if footer != 0 {
            self.draw_image_at(footer, 0, footer_y, false);
        }
        let soft_color = self.memory.r32(object + SOFT_COLOR) as u16;
        let text = self.memory.r32(object + LEFT_TEXT);
        let bytes = self.read_c_bytes(text, 256);
        self.draw_text_bytes_on_image(SCREEN_IMAGE_STRUCT, &bytes, 4, footer_y + 4, soft_color);
        let text = self.memory.r32(object + RIGHT_TEXT);
        let bytes = self.read_c_bytes(text, 256);
        let (label, _, _) = encoding_rs::GBK.decode(&bytes);
        let text_width: i32 = label
            .chars()
            .map(|c| if c.is_ascii() { 8 } else { 16 })
            .sum();
        self.draw_text_bytes_on_image(
            SCREEN_IMAGE_STRUCT,
            &bytes,
            width - text_width - 4,
            footer_y + 4,
            soft_color,
        );
        let row_height = self.memory.r32(object + ROW_HEIGHT).max(16) as i32;
        let count = self.memory.r32(object + COUNT);
        let selected = self.memory.r32(object + SELECTED);
        let visible = (height.max(0) / row_height).max(1) as u32;
        let first = selected.saturating_sub(visible - 1);
        let items = self.memory.r32(object + ITEMS);
        let color = self.memory.r32(object + TEXT_COLOR) as u16;
        let selected_color = self.memory.r32(object + SELECTED_COLOR) as u16;
        let highlight = self.memory.r32(object + HIGHLIGHT) as u16;
        let icon = self.memory.r32(object + ITEM_ICON);
        let (icon_width, icon_height) = self.image_dims(icon);
        for index in first..count.min(first + visible) {
            let line_y = y + (index - first) as i32 * row_height;
            if index == selected {
                self.fill_screen_rect(x, line_y, width, row_height, highlight);
                if icon != 0 {
                    self.draw_image_at(icon, x, line_y + (row_height - icon_height) / 2, true);
                }
            }
            let text = self.memory.r32(items + index * 8);
            let bytes = self.read_c_bytes(text, 512);
            self.draw_text_bytes_on_image(
                SCREEN_IMAGE_STRUCT,
                &bytes,
                x + icon_width + 4,
                line_y + (row_height - 16) / 2,
                if index == selected {
                    selected_color
                } else {
                    color
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::memory::MachineMemory;

    fn list_machine(big_endian: bool) -> (NicaiMachine, u32, u32) {
        let mut machine = NicaiMachine::new_blank_for_tests();
        let mut memory = MachineMemory::new(big_endian);
        memory.regions = std::mem::take(&mut machine.memory.regions);
        machine.memory = memory;
        machine.initialize_screen();
        let slot = machine.allocate(4);
        let stack = machine.allocate(16);
        let rect = machine.allocate(8);
        for (i, value) in [0, 24, 240, 272].into_iter().enumerate() {
            machine.memory.w16(rect + i as u32 * 2, value);
        }
        let first = machine.memory.r32(rect);
        let second = machine.memory.r32(rect + 4);
        machine.set_user_register(0, slot);
        machine.set_user_register(1, 2);
        machine.set_user_register(2, first);
        machine.set_user_register(3, second);
        machine.set_user_register(13, stack);
        machine.memory.w32(stack + 8, 0x0010_0001);
        machine.initialize_list_control();
        let object = machine.memory.r32(slot);
        (machine, slot, object)
    }

    #[test]
    fn list_control_preserves_items_selection_and_guest_callbacks() {
        for big_endian in [false, true] {
            let (mut machine, slot, object) = list_machine(big_endian);
            assert_ne!(object, 0);
            assert_eq!(machine.pending_screen, object + SCREEN);
            machine.memory.w32(object + 0x38, 0x0010_0101);
            let text = machine.allocate(4);
            machine.memory.write_bytes(text, b"one\0");
            for id in [10, 20] {
                machine.set_user_register(0, object);
                machine.set_user_register(1, text);
                machine.set_user_register(2, id);
                machine.handle_control_method(0xd8).unwrap();
                assert_eq!(machine.register(0), 1);
            }
            let items = machine.memory.r32(object + ITEMS);
            assert_eq!(machine.memory.r32(items), text);
            assert_eq!(machine.memory.r32(items + 12), 20);
            let payload = machine.allocate(4);
            machine.memory.w32(payload, 1 << 18);
            machine.set_user_register(0, object);
            machine.set_user_register(1, 0);
            machine.set_user_register(2, payload);
            machine.handle_control_method(0x204).unwrap();
            machine.set_user_register(0, object);
            machine.handle_control_method(0x11c).unwrap();
            assert_eq!(machine.register(0), 1);
            assert_eq!(machine.memory.r32(object + 0x38), 0x0010_0101);
            machine.memory.w32(payload, 1 << 14);
            machine.set_user_register(0, object);
            machine.set_user_register(1, 0);
            machine.set_user_register(2, payload);
            machine.handle_control_method(0x204).unwrap();
            let (entry, args, _) = machine.pending_callbacks.pop_front().unwrap();
            assert_eq!(entry, 0x0010_0001);
            assert_eq!(args, [object, 2]);
            machine.set_user_register(0, slot);
            machine.handle_control_method(0x138).unwrap();
            assert_eq!(machine.memory.r32(slot), 0);
            assert!(machine.screen_stack.is_empty());
        }
    }

    #[test]
    fn list_control_skin_colors_do_not_replace_item_text_colors() {
        let (mut machine, _, object) = list_machine(false);
        machine.set_user_register(0, object);
        machine.set_user_register(1, 0xf800);
        machine.set_user_register(2, 0x07e0);
        machine.handle_control_method(0xec).unwrap();
        machine.set_user_register(0, object);
        machine.set_user_register(1, 0xffff);
        machine.handle_control_method(0x118).unwrap();
        assert_eq!(machine.memory.r32(object + TEXT_COLOR), 0xf800);
        assert_eq!(machine.memory.r32(object + SELECTED_COLOR), 0x07e0);
        assert_eq!(machine.memory.r32(object + SOFT_COLOR), 0xffff);
        let mut state = vec![0; crate::save_state::SERIALIZED_SIZE];
        crate::save_state::encode_machine(&machine, 123, &mut state).unwrap();
        let mut restored = crate::save_state::decode_machine(&state, 123).unwrap();
        assert_eq!(restored.pending_screen, object + SCREEN);
        assert_eq!(restored.memory.r32(object + TEXT_COLOR), 0xf800);
    }
}
