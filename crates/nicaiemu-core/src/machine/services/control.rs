//! Firmware list and numeric input controls with guest-owned state.

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
// Reserved service storage keeps the input list inside serialized guest memory.
const INPUT_HEAD: u32 = CONTROL_METHOD_SERVICE + 0x3f0;
const INPUT_TEXT: u32 = 0x100;
const INPUT_TAG: u32 = 0xe80;
const INPUT_RECT: u32 = 0xe84;
const INPUT_OWNER: u32 = 0xe8c;
const INPUT_FOCUSED: u32 = 0xe90;
const INPUT_PASSWORD: u32 = 0xe94;
const INPUT_NEXT: u32 = 0xe98;
const INPUT_CALLBACK: u32 = 0xe9c;
const INPUT_MAGIC: u32 = 0x494e5054;
const POPUP_TEXT: u32 = 0x80;
const POPUP_CALLBACK: u32 = 0x84;
const POPUP_CONTEXT: u32 = 0x88;
const POPUP_SLOT: u32 = 0x8c;

impl NicaiMachine {
    pub(crate) fn initialize_popup_control(&mut self) {
        let slot = self.register(0);
        if slot == 0 || self.memory.region(slot, 4).is_none() {
            self.set_result(0);
            return;
        }
        let callback = self.argument(5);
        let object = self.allocate(0x100);
        if object == 0 {
            self.set_result(0);
            return;
        }
        self.memory.write_bytes(object, &[0; 0x100]);
        for (field, method) in [
            (0x74, 0x2ac),
            (0x78, 0x2a8),
            (SCREEN + 8, 0x2b4),
            (SCREEN + 12, 0x2b0),
        ] {
            self.memory
                .w32(object + field, (CONTROL_METHOD_SERVICE + method) | 1);
        }
        self.memory.w32(object + POPUP_CALLBACK, callback);
        self.memory.w32(object + POPUP_SLOT, slot);
        self.memory.w32(slot, object);
        self.set_result(1);
    }

    fn close_popup_control(&mut self, object: u32) {
        let slot = self.memory.r32(object + POPUP_SLOT);
        self.cpu.reg_set(armv4t_emu::Mode::User, 0, object + SCREEN);
        self.handle_screen_service(6);
        if slot != 0 {
            self.memory.w32(slot, 0);
        }
        self.deallocate(object);
    }

    pub(crate) fn initialize_input_control(&mut self, password: bool) {
        let slot = self.register(0);
        if slot == 0 || self.memory.region(slot, 4).is_none() {
            self.set_result(0);
            return;
        }
        let object = self.allocate(0xf00);
        if object == 0 {
            self.set_result(0);
            return;
        }
        self.memory.write_bytes(object, &[0; 0xf00]);
        for offset in (0xe00..0xe80).step_by(4) {
            self.memory.w32(
                object + offset,
                (CONTROL_METHOD_SERVICE + 0x300 + offset - 0xe00) | 1,
            );
        }
        self.memory.w32(object + INPUT_TAG, INPUT_MAGIC);
        self.memory.w32(object + INPUT_RECT, self.register(2));
        self.memory.w32(object + INPUT_RECT + 4, self.register(3));
        self.memory.w32(object + INPUT_OWNER, self.pending_screen);
        self.memory
            .w32(object + INPUT_PASSWORD, u32::from(password));
        let callback = self.argument(4);
        self.memory.w32(object + INPUT_CALLBACK, callback);
        let next = self.memory.r32(INPUT_HEAD);
        self.memory.w32(object + INPUT_NEXT, next);
        self.memory.w32(INPUT_HEAD, object);
        self.memory.w32(slot, object);
        self.set_result(1);
    }

    fn input_controls(&mut self) -> Vec<u32> {
        let mut controls = Vec::new();
        let mut object = self.memory.r32(INPUT_HEAD);
        while object != 0 && controls.len() < 128 && !controls.contains(&object) {
            if self.memory.region(object, 0xf00).is_none()
                || self.memory.r32(object + INPUT_TAG) != INPUT_MAGIC
            {
                break;
            }
            controls.push(object);
            object = self.memory.r32(object + INPUT_NEXT);
        }
        controls
    }

    pub(crate) fn input_control_key(&mut self, key: u8) {
        if key > 9 {
            return;
        }
        for object in self.input_controls() {
            if self.memory.r32(object + INPUT_OWNER) != self.active_screen
                || self.memory.r32(object + INPUT_FOCUSED) == 0
            {
                continue;
            }
            let text = self.read_c_bytes(object + INPUT_TEXT, 255);
            if text.len() < 255 {
                self.memory
                    .w8(object + INPUT_TEXT + text.len() as u32, b'0' + key);
                self.memory
                    .w8(object + INPUT_TEXT + text.len() as u32 + 1, 0);
                let callback = self.memory.r32(object + INPUT_CALLBACK);
                self.defer_callback(callback, vec![object, 2], "inputChanged");
            }
        }
    }

    pub(crate) fn render_input_controls(&mut self) {
        for object in self.input_controls() {
            if self.memory.r32(object + INPUT_OWNER) != self.active_screen {
                continue;
            }
            let top = self.memory.r32(object + INPUT_RECT);
            let bottom = self.memory.r32(object + INPUT_RECT + 4);
            let (x, y) = ((top >> 16) as i16 as i32, top as i16 as i32);
            let (right, below) = ((bottom >> 16) as i16 as i32, bottom as i16 as i32);
            let focused = self.memory.r32(object + INPUT_FOCUSED) != 0;
            self.fill_screen_rect(
                x,
                y,
                right - x + 1,
                below - y + 1,
                if focused { 0x001f } else { 0x8410 },
            );
            self.fill_screen_rect(x + 1, y + 1, right - x - 1, below - y - 1, 0xffff);
            let mut text = self.read_c_bytes(object + INPUT_TEXT, 255);
            if self.memory.r32(object + INPUT_PASSWORD) != 0 {
                text.fill(b'*');
            }
            let visible = ((right - x - 4).max(0) / 8) as usize;
            let start = text.len().saturating_sub(visible);
            self.draw_text_bytes_on_image(SCREEN_IMAGE_STRUCT, &text[start..], x + 2, y + 2, 0);
        }
    }
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
        if (0x300..0x380).contains(&offset) {
            if self.memory.region(object, 0xf00).is_none()
                || self.memory.r32(object + INPUT_TAG) != INPUT_MAGIC
            {
                self.set_result(0);
                return Ok(());
            }
            if offset == 0x34c {
                let record = self.register(1);
                let capacity = self.register(2).min(256);
                let destination = if self.memory.region(record, 8).is_some() {
                    self.memory.r32(record + 4)
                } else {
                    0
                };
                let text = self.read_c_bytes(object + INPUT_TEXT, capacity.saturating_sub(1));
                if capacity > 0
                    && destination != 0
                    && self.memory.region(destination, capacity as usize).is_some()
                {
                    self.memory.write_bytes(destination, &text);
                    self.memory.w8(destination + text.len() as u32, 0);
                    self.memory.w16(record, text.len() as u16);
                }
                self.set_result(text.len() as u32);
                return Ok(());
            } else if offset == 0x35c {
                self.memory
                    .w32(object + INPUT_FOCUSED, u32::from(self.register(1) != 0));
            }
            self.set_result(0);
            return Ok(());
        }
        match offset {
            0x2a8 => {
                self.memory.w32(object + POPUP_TEXT, self.register(2));
                self.memory.w32(object + POPUP_CONTEXT, self.register(3));
                let screen = object + SCREEN;
                if !self.screen_stack.contains(&screen) {
                    self.screen_stack.push(screen);
                }
                self.pending_screen = screen;
                self.set_result(1);
            }
            0x2ac => {
                let control = self.memory.r32(object);
                if control != 0 {
                    self.close_popup_control(control);
                }
                self.set_result(0);
            }
            0x2b0 => {
                let (width, height) = self.framebuffer_size();
                let y = (height as i32 - 120) / 2;
                self.fill_screen_rect(8, y, width as i32 - 16, 120, 0x8410);
                self.fill_screen_rect(9, y + 1, width as i32 - 18, 118, 0xffff);
                let source = self.memory.r32(object + POPUP_TEXT);
                let text = self.read_c_bytes(source, 256);
                self.draw_text_bytes_on_image(SCREEN_IMAGE_STRUCT, &text, 16, y + 12, 0);
                self.draw_text_bytes_on_image(
                    SCREEN_IMAGE_STRUCT,
                    b"\xc8\xb7\xb6\xa8",
                    16,
                    y + 92,
                    0,
                );
                self.draw_text_bytes_on_image(
                    SCREEN_IMAGE_STRUCT,
                    b"\xb7\xb5\xbb\xd8",
                    width as i32 - 48,
                    y + 92,
                    0,
                );
                self.set_result(0);
            }
            0x2b4 => {
                let event = self.register(1);
                let keys = if event == 0 {
                    self.memory.r32(self.register(2))
                } else if event == 6 {
                    self.key_down
                } else {
                    0
                };
                let choice = if keys & ((1 << 12) | (1 << 14)) != 0 {
                    Some(0)
                } else if keys & (1 << 13) != 0 {
                    Some(1)
                } else {
                    None
                };
                if let Some(choice) = choice {
                    let callback = self.memory.r32(object + POPUP_CALLBACK);
                    let context = self.memory.r32(object + POPUP_CONTEXT);
                    self.close_popup_control(object);
                    self.defer_callback(callback, vec![object, choice, context], "popupResult");
                }
                self.set_result(1);
            }
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

    #[test]
    fn popup_control_restores_parent_and_defers_button_result() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        let parent = machine.allocate(64);
        machine.screen_stack.push(parent);
        machine.active_screen = parent;
        machine.pending_screen = parent;
        let slot = machine.allocate(4);
        let stack = machine.allocate(8);
        machine.memory.w32(stack + 4, 0x0100_2001);
        machine.set_user_register(13, stack);
        machine.set_user_register(0, slot);
        machine.handle_ctrl_service(7);
        let object = machine.memory.r32(slot);
        assert_ne!(object, 0);
        assert_ne!(machine.memory.r32(object + 0x78), 0);
        machine.set_user_register(0, object);
        machine.set_user_register(2, 0);
        machine.set_user_register(3, 123);
        machine.handle_control_method(0x2a8).unwrap();
        assert_eq!(machine.screen_stack, [parent, object + SCREEN]);
        machine.active_screen = object + SCREEN;
        let payload = machine.allocate(4);
        machine.memory.w32(payload, 1 << 14);
        machine.set_user_register(0, object);
        machine.set_user_register(1, 0);
        machine.set_user_register(2, payload);
        machine.handle_control_method(0x2b4).unwrap();
        assert_eq!(machine.active_screen, parent);
        assert_eq!(machine.pending_screen, parent);
        assert_eq!(machine.memory.r32(slot), 0);
        let (callback, args, _) = machine.pending_callbacks.pop_front().unwrap();
        assert_eq!(callback, 0x0100_2001);
        assert_eq!(args, [object, 0, 123]);
    }

    #[test]
    fn input_getter_writes_the_callers_text_record_not_its_pointer() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        let slot = machine.allocate(4);
        machine.set_user_register(0, slot);
        machine.initialize_input_control(false);
        let object = machine.memory.r32(slot);
        machine.memory.write_bytes(object + INPUT_TEXT, b"123\0");
        let record = machine.allocate(8);
        let text = machine.allocate(8);
        machine.memory.w32(record + 4, text);
        machine.memory.w32(text + 4, 0x1234_5678);
        machine.set_user_register(0, object);
        machine.set_user_register(1, record);
        machine.set_user_register(2, 4);
        machine.handle_control_method(0x34c).unwrap();
        assert_eq!(machine.memory.r32(record + 4), text);
        assert_eq!(machine.memory.r16(record), 3);
        assert_eq!(machine.read_c_bytes(text, 4), b"123");
        assert_eq!(machine.memory.r8(text + 3), 0);
        assert_eq!(machine.memory.r32(text + 4), 0x1234_5678);
    }

    #[test]
    fn input_control_focus_text_bounds_and_snapshot_are_preserved() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        let screen = machine.allocate(64);
        machine.pending_screen = screen;
        machine.active_screen = screen;
        let slot = machine.allocate(4);
        let stack = machine.allocate(4);
        machine.memory.w32(stack, 0x0100_1001);
        machine.set_user_register(13, stack);
        machine.set_user_register(0, slot);
        machine.set_user_register(2, (10 << 16) | 40);
        machine.set_user_register(3, (210 << 16) | 65);
        machine.handle_ctrl_service(11);
        assert_eq!(machine.register(0), 1);
        let object = machine.memory.r32(slot);
        assert_ne!(object, 0);
        assert_eq!(machine.memory.r32(object + INPUT_PASSWORD), 1);
        assert_eq!(machine.memory.r32(object + INPUT_RECT), (10 << 16) | 40);
        assert_ne!(machine.memory.r32(object + 0xe4c), 0);
        machine.set_user_register(0, object);
        machine.set_user_register(1, 1);
        machine.handle_control_method(0x35c).unwrap();
        machine.set_key(4, true);
        machine.set_key(4, false);
        machine.set_key(2, true);
        machine.set_key(2, false);
        assert_eq!(machine.pending_callbacks.len(), 2);
        let (callback, args, _) = &machine.pending_callbacks[0];
        assert_eq!(*callback, 0x0100_1001);
        assert_eq!(args, &[object, 2]);
        let output = machine.allocate(8);
        let record = machine.allocate(8);
        machine.memory.w32(record + 4, output);
        machine.memory.write_bytes(output, &[0xcc; 8]);
        machine.set_user_register(0, object);
        machine.set_user_register(1, record);
        machine.set_user_register(2, 2);
        machine.handle_control_method(0x34c).unwrap();
        assert_eq!(machine.memory.r8(output), b'4');
        assert_eq!(machine.memory.r8(output + 1), 0);
        assert_eq!(machine.memory.r8(output + 2), 0xcc);
        assert_eq!(machine.memory.r16(record), 1);
        assert_eq!(machine.memory.r32(record + 4), output);
        let mut state = vec![0; crate::save_state::SERIALIZED_SIZE];
        crate::save_state::encode_machine(&machine, 456, &mut state).unwrap();
        let mut restored = crate::save_state::decode_machine(&state, 456).unwrap();
        assert_eq!(restored.input_controls(), [object]);
        assert_eq!(restored.read_c_bytes(object + INPUT_TEXT, 256), b"42");
        restored.active_screen = 0;
        restored.set_key(8, true);
        assert_eq!(restored.read_c_bytes(object + INPUT_TEXT, 256), b"42");
    }

    #[test]
    fn input_control_rejects_invalid_output_slot_without_allocating() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        let cursor = machine.heap_cursor;
        machine.set_user_register(0, 0);
        machine.handle_ctrl_service(12);
        assert_eq!(machine.register(0), 0);
        assert_eq!(machine.heap_cursor, cursor);
        assert_eq!(machine.memory.r32(INPUT_HEAD), 0);
    }
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
