//! Active LCD geometry, independent of frontend presentation orientation.

use armv4t_emu::Memory;

use super::{DisplayOrientation, NicaiMachine, FRAME_HEIGHT, FRAME_WIDTH, SCREEN_IMAGE};

#[derive(Debug, Clone, Copy)]
pub(super) struct DisplaySurface {
    pub width: u32,
    pub height: u32,
    clip_hint: bool,
    scanline_clear: Option<(i32, i32)>,
    drawing_bounds: Option<(i32, i32)>,
}

impl Default for DisplaySurface {
    fn default() -> Self {
        Self {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
            clip_hint: false,
            scanline_clear: None,
            drawing_bounds: None,
        }
    }
}

impl DisplaySurface {
    pub fn stride(self) -> u32 {
        (self.width + 3) & !3
    }

    fn supports(width: u32, height: u32) -> bool {
        matches!(
            (width, height),
            (176, 220)
                | (220, 176)
                | (240, 320)
                | (320, 240)
                | (240, 352)
                | (352, 240)
                | (240, 400)
                | (400, 240)
        )
    }

    pub fn unrotate(self, orientation: DisplayOrientation, x: i32, y: i32) -> (i32, i32) {
        orientation.unrotate(x, y, self.width)
    }
}

impl NicaiMachine {
    /// Startup full-surface clips identify supported LCD modes. Explicit clips
    /// take precedence over inferred fills; later drawing cannot resize the LCD.
    pub(super) fn observe_display_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.observe_display_mode(x, y, width, height, true);
    }

    pub(super) fn observe_display_fill(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.observe_display_mode(x, y, width, height, false);
    }

    /// A contiguous full-width scanline clear carries the same mode evidence
    /// as a rectangle fill. Commit only when the sequence ends, not at an
    /// intermediate supported height of a taller display.
    pub(super) fn observe_display_scanline(&mut self, row: Option<(i32, i32)>) {
        if self.frame_count >= 4
            || self.display_surface.clip_hint
            || self.framebuffer_size() != (FRAME_WIDTH, FRAME_HEIGHT)
        {
            self.display_surface.scanline_clear = None;
            return;
        }
        let previous = self.display_surface.scanline_clear.take();
        if let (Some((width, next_y)), Some((row_width, y))) = (previous, row) {
            if width == row_width && y == next_y {
                self.display_surface.scanline_clear = Some((width, y + 1));
                return;
            }
        }
        if let Some((width, height)) = previous {
            self.observe_display_fill(0, 0, width, height);
        }
        if let Some((width, 0)) = row {
            self.display_surface.scanline_clear = Some((width, 1));
        }
    }

    /// Accumulate startup drawing extents; a later larger backdrop cancels a
    /// small splash image. Ignore glyph cells and vertical sprite strips; leave
    /// up to one text row for an unused softkey band.
    pub(super) fn observe_display_drawing(&mut self, x: i32, y: i32, width: i32, height: i32) {
        if self.frame_count >= 4
            || self.framebuffer_size() != (FRAME_WIDTH, FRAME_HEIGHT)
            || x < 0
            || y < 0
            || width <= 0
            || height <= 0
        {
            return;
        }
        let (right, bottom) = self.display_surface.drawing_bounds.unwrap_or((0, 0));
        self.display_surface.drawing_bounds = Some((right.max(x + width), bottom.max(y + height)));
    }

    pub(super) fn finish_display_discovery(&mut self) {
        if self.frame_count != 3 {
            return;
        }
        let Some((width, bottom)) = self.display_surface.drawing_bounds else {
            return;
        };
        for height in bottom..=bottom.saturating_add(16) {
            if DisplaySurface::supports(width as u32, height as u32) {
                self.observe_display_fill(0, 0, width, height);
                break;
            }
        }
        self.display_surface.drawing_bounds = None;
    }

    fn observe_display_mode(&mut self, x: i32, y: i32, width: i32, height: i32, clip_hint: bool) {
        if self.frame_count >= 4
            || x != 0
            || y != 0
            || !DisplaySurface::supports(width as u32, height as u32)
            || self.display_surface.clip_hint
            || (!clip_hint && self.framebuffer_size() != (FRAME_WIDTH, FRAME_HEIGHT))
        {
            return;
        }
        if (width as u32, height as u32) == self.framebuffer_size() {
            if clip_hint && self.framebuffer_size() != (FRAME_WIDTH, FRAME_HEIGHT) {
                self.display_surface.clip_hint = true;
                self.memory.w32(super::SCREEN_IMAGE_STRUCT + 20, 1);
            }
            return;
        }
        let old = self.display_surface;
        let next = DisplaySurface {
            width: width as u32,
            height: height as u32,
            clip_hint,
            scanline_clear: None,
            drawing_bounds: None,
        };
        // Preserve existing pixels when changing the row stride.
        let mut pixels = Vec::with_capacity((old.width * old.height) as usize);
        for y in 0..old.height {
            for x in 0..old.width {
                pixels.push(self.memory.r16(SCREEN_IMAGE + (y * old.stride() + x) * 2));
            }
        }
        for i in 0..FRAME_WIDTH * FRAME_HEIGHT {
            self.memory.w16(SCREEN_IMAGE + i * 2, 0);
        }
        for y in 0..old.height.min(next.height) {
            for x in 0..old.width.min(next.width) {
                self.memory.w16(
                    SCREEN_IMAGE + (y * next.stride() + x) * 2,
                    pixels[(y * old.width + x) as usize],
                );
            }
        }
        self.display_surface = next;
        self.memory
            .w32(super::SCREEN_IMAGE_STRUCT + 20, u32::from(clip_hint));
        self.oldlib_clip = [0, 0, width, height];
        self.write_screen_header();
    }

    /// Native LCD dimensions before presentation rotation.
    pub fn framebuffer_size(&self) -> (u32, u32) {
        (self.display_surface.width, self.display_surface.height)
    }

    pub(super) fn restore_display_surface(&mut self) {
        let (width, height) = self.image_dims(super::SCREEN_IMAGE_STRUCT);
        self.display_surface = if DisplaySurface::supports(width as u32, height as u32) {
            DisplaySurface {
                width: width as u32,
                height: height as u32,
                clip_hint: self.memory.r32(super::SCREEN_IMAGE_STRUCT + 20) != 0,
                scanline_clear: None,
                drawing_bounds: None,
            }
        } else {
            DisplaySurface::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::super::SCREEN_IMAGE_STRUCT;
    use super::*;

    #[test]
    fn startup_backdrops_resolve_geometry_without_cropping_a_later_splash() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        machine.observe_display_drawing(0, 0, 176, 205);
        machine.observe_display_drawing(10, 201, 28, 12);
        machine.frame_count = 2;
        machine.finish_display_discovery();
        assert_eq!(machine.framebuffer_size(), (240, 400));
        machine.frame_count = 3;
        machine.finish_display_discovery();
        assert_eq!(machine.framebuffer_size(), (176, 220));

        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        machine.observe_display_drawing(0, 0, 176, 220);
        machine.observe_display_drawing(0, 0, 240, 320);
        machine.frame_count = 3;
        machine.finish_display_discovery();
        assert_eq!(machine.framebuffer_size(), (240, 320));

        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        machine.observe_display_drawing(0, 0, 176, 180);
        machine.frame_count = 3;
        machine.finish_display_discovery();
        assert_eq!(machine.framebuffer_size(), (240, 400));
    }

    #[test]
    fn scanline_clear_commits_only_after_the_complete_sequence() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        for y in 0..400 {
            machine.observe_display_scanline(Some((240, y)));
            assert_eq!(machine.framebuffer_size(), (240, 400));
        }
        machine.observe_display_scanline(None);
        assert_eq!(machine.framebuffer_size(), (240, 400));
        for y in 0..220 {
            machine.observe_display_scanline(Some((176, y)));
        }
        machine.observe_display_scanline(None);
        assert_eq!(machine.framebuffer_size(), (176, 220));
    }

    #[test]
    fn partial_scanlines_and_explicit_clips_do_not_change_geometry() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        for y in 0..100 {
            machine.observe_display_scanline(Some((176, y)));
        }
        for y in 101..220 {
            machine.observe_display_scanline(Some((176, y)));
        }
        machine.observe_display_scanline(None);
        assert_eq!(machine.framebuffer_size(), (240, 400));
        machine.observe_display_rect(0, 0, 240, 320);
        for y in 0..220 {
            machine.observe_display_scanline(Some((176, y)));
        }
        machine.observe_display_scanline(None);
        assert_eq!(machine.framebuffer_size(), (240, 320));
    }

    #[test]
    fn startup_clip_resolves_geometry_and_preserves_rows() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        machine
            .memory
            .w16(SCREEN_IMAGE + (219 * 240 + 175) * 2, 0xf800);
        machine.set_user_register(0, 0);
        machine.set_user_register(1, 0);
        machine.set_user_register(2, 176);
        machine.set_user_register(3, 220);
        machine.set_oldlib_clip();
        assert_eq!(machine.display_size(), (176, 220));
        assert_eq!(machine.memory.r16(SCREEN_IMAGE_STRUCT + 8), 176);
        assert_eq!(machine.frame_pixels()[219 * 176 + 175], 0xff0000);
        machine.fill_screen_rect(175, 219, 2, 2, 0x07e0);
        assert_eq!(machine.frame_pixels()[219 * 176 + 175], 0x00ff00);
        assert_eq!(machine.memory.r16(SCREEN_IMAGE + 176 * 220 * 2), 0);
    }

    #[test]
    fn startup_mode_detection_ignores_partial_and_late_rectangles() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        machine.observe_display_rect(0, 0, 100, 100);
        machine.observe_display_rect(10, 0, 176, 220);
        assert_eq!(machine.framebuffer_size(), (240, 400));
        machine.observe_display_rect(0, 0, 240, 320);
        machine.observe_display_rect(0, 0, 320, 240);
        assert_eq!(machine.framebuffer_size(), (240, 320));
        machine.frame_count = 4;
        machine.observe_display_rect(0, 0, 240, 400);
        assert_eq!(machine.framebuffer_size(), (240, 320));
    }

    #[test]
    fn legacy_and_native_clip_services_resolve_the_same_lcd_mode() {
        for legacy in [false, true] {
            let mut machine = NicaiMachine::new_blank_for_tests();
            machine.initialize_screen();
            for (register, value) in [0, 0, 240, 320].into_iter().enumerate() {
                machine.set_user_register(register as u8, value);
            }
            if legacy {
                machine.handle_game_lcd_service(12);
            } else {
                machine.set_oldlib_clip();
            }
            assert_eq!(machine.framebuffer_size(), (240, 320));
            assert_eq!(machine.image_dims(SCREEN_IMAGE_STRUCT), (240, 320));
        }
    }

    #[test]
    fn packed_window_bounds_use_guest_byte_order() {
        for big_endian in [false, true] {
            let mut machine = NicaiMachine::new_blank_for_tests();
            let mut memory = super::super::memory::MachineMemory::new(big_endian);
            memory.regions = std::mem::take(&mut machine.memory.regions);
            machine.memory = memory;
            machine.initialize_screen();
            let stack = super::super::STACK_BASE + 0x100;
            machine.set_user_register(armv4t_emu::reg::SP, stack);
            machine.memory.w32(stack + 4, 1);
            machine.set_user_register(0, super::super::HEAP_BASE + 0x1000);
            machine.set_user_register(1, 0);
            machine.set_user_register(
                2,
                if big_endian {
                    240 << 16 | 320
                } else {
                    320 << 16 | 240
                },
            );
            machine.set_user_register(3, 0);
            machine.initialize_fixed_gameold_region();
            assert_eq!(machine.framebuffer_size(), (240, 320));
        }
    }

    #[test]
    fn landscape_pixels_text_and_pointer_share_active_geometry() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        machine.observe_display_rect(0, 0, 176, 220);
        machine.set_orientation(DisplayOrientation::Landscape);
        machine.fill_screen_rect(175, 219, 1, 1, 0xf800);
        assert_eq!(machine.display_size(), (220, 176));
        assert_eq!(machine.display_to_framebuffer(219, 0), (175, 219));
        assert_eq!(machine.frame_pixels()[219], 0xff0000);
        machine
            .memory
            .write_bytes(super::super::HEAP_BASE + 0x100, b"A\0");
        machine.set_user_register(0, super::super::HEAP_BASE + 0x100);
        machine.set_user_register(1, 200);
        machine.set_user_register(2, 150);
        machine.set_user_register(3, 0x07e0);
        machine.handle_lcd_service(9);
        let pixels = machine.frame_pixels();
        assert!(pixels.iter().enumerate().any(|(i, p)| *p == 0x00ff00
            && (200..208).contains(&(i % 220))
            && (150..166).contains(&(i / 220))));
    }

    #[test]
    fn native_landscape_is_not_rotated_twice_in_auto_mode() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        machine.orientation = DisplayOrientation::Auto;
        machine.effective_orientation = DisplayOrientation::Landscape;
        machine.observe_display_rect(0, 0, 220, 176);
        assert_eq!(machine.display_size(), (220, 176));
        assert_eq!(machine.display_to_framebuffer(219, 175), (219, 175));
    }

    #[test]
    fn scalar_lcd_fill_accepts_native_landscape_width() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        machine.set_user_register(0, 0);
        machine.set_user_register(1, 0);
        machine.set_user_register(2, 320);
        machine.set_user_register(3, 240);
        let stack = super::super::STACK_BASE + 0x100;
        machine.set_user_register(armv4t_emu::reg::SP, stack);
        machine.memory.w32(stack, 0xf800);
        machine.handle_lcd_service(19);
        assert_eq!(machine.display_size(), (320, 240));
        assert_eq!(machine.frame_pixels()[239 * 320 + 319], 0xff0000);
    }

    #[test]
    fn explicit_clip_overrides_a_transposed_startup_clear() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        machine.fill_screen_rect(0, 0, 320, 240, 0);
        assert_eq!(machine.framebuffer_size(), (320, 240));
        machine.observe_display_rect(0, 0, 240, 320);
        machine.fill_screen_rect(0, 0, 320, 240, 0);
        assert_eq!(machine.framebuffer_size(), (240, 320));
    }

    #[test]
    fn save_state_restores_active_geometry_in_both_image_abis() {
        for wide in [false, true] {
            let mut machine = NicaiMachine::new_blank_for_tests();
            machine.wide_images = wide;
            machine.initialize_screen();
            machine.observe_display_rect(0, 0, 240, 320);
            machine.fill_screen_rect(239, 319, 1, 1, 0xf800);
            let mut data = vec![0; crate::save_state::SERIALIZED_SIZE];
            crate::save_state::encode_machine(&machine, 123, &mut data).unwrap();
            let mut restored = crate::save_state::decode_machine(&data, 123).unwrap();
            assert_eq!(restored.display_size(), (240, 320));
            assert_eq!(restored.frame_pixels()[319 * 240 + 239], 0xff0000);
        }
    }

    #[test]
    fn save_state_preserves_provisional_fill_geometry() {
        let mut machine = NicaiMachine::new_blank_for_tests();
        machine.initialize_screen();
        machine.fill_screen_rect(0, 0, 220, 176, 0);
        let mut data = vec![0; crate::save_state::SERIALIZED_SIZE];
        crate::save_state::encode_machine(&machine, 123, &mut data).unwrap();
        let mut restored = crate::save_state::decode_machine(&data, 123).unwrap();
        restored.observe_display_rect(0, 0, 176, 220);
        assert_eq!(restored.framebuffer_size(), (176, 220));
    }
}
