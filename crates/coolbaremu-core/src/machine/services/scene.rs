//! Scene maps exposed to native rendering callbacks.

use armv4t_emu::{Memory, Mode};

use super::super::CoolbarMachine;

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Option<&'a [u8]> {
        let end = self.position.checked_add(size)?;
        let bytes = self.bytes.get(self.position..end)?;
        self.position = end;
        Some(bytes)
    }

    fn short(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn integer(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn name(&mut self) -> Option<&'a [u8]> {
        let length = usize::from(*self.take(1)?.first()?);
        self.take(length)
    }
}

struct SceneMap {
    width: u16,
    height: u16,
    name: Vec<u8>,
    position: [i16; 2],
    tail: usize,
}

fn first_scene_map(bytes: &[u8]) -> Option<SceneMap> {
    let mut reader = Reader { bytes, position: 0 };
    if reader.take(4)? != b"SCE2" {
        return None;
    }
    let width = reader.short()?;
    let height = reader.short()?;
    let map_count = reader.short()?;
    if width == 0 || height == 0 || !(1..=16).contains(&map_count) {
        return None;
    }
    let name = reader.name()?.to_vec();
    let position = [reader.short()? as i16, reader.short()? as i16];
    reader.take(4)?;
    for _ in 1..map_count {
        reader.name()?;
        reader.take(8)?;
    }
    Some(SceneMap {
        width,
        height,
        name,
        position,
        tail: reader.position,
    })
}

struct TileMap {
    images: Vec<Vec<u8>>,
    pixel_width: u32,
    pixel_height: u32,
    tile_width: u32,
    tile_height: u32,
    columns: u32,
    rows: u32,
    cells: Vec<u32>,
}

fn parse_tile_map(bytes: &[u8]) -> Option<TileMap> {
    let mut reader = Reader { bytes, position: 0 };
    let image_count = reader.integer()?;
    if !(1..=16).contains(&image_count) {
        return None;
    }
    let mut images = Vec::new();
    for _ in 0..image_count {
        images.push(reader.name()?.to_vec());
    }
    let pixel_width = reader.integer()?;
    let pixel_height = reader.integer()?;
    let tile_width = reader.integer()?;
    let tile_height = reader.integer()?;
    if !(1..=4096).contains(&pixel_width)
        || !(1..=4096).contains(&pixel_height)
        || !(1..=4096).contains(&tile_width)
        || !(1..=4096).contains(&tile_height)
    {
        return None;
    }
    let columns = pixel_width.div_ceil(tile_width);
    let rows = pixel_height.div_ceil(tile_height);
    if columns.checked_mul(rows)? > 65536 {
        return None;
    }
    let mut cells = Vec::new();
    for _ in 0..columns * rows {
        let cell = reader.integer()?;
        if ((cell >> 24) & 15) >= image_count {
            return None;
        }
        cells.push(cell);
    }
    Some(TileMap {
        images,
        pixel_width,
        pixel_height,
        tile_width,
        tile_height,
        columns,
        rows,
        cells,
    })
}

/// Byte range of the method slots inside a native map object.  The map
/// data fields live below `NATIVE_MAP_METHOD_START`; the firmware exposes
/// the object's callable methods from there up to `NATIVE_MAP_METHOD_END`.
const NATIVE_MAP_METHOD_START: u32 = 0x28;
const NATIVE_MAP_METHOD_END: u32 = 0xbc;

impl CoolbarMachine {
    fn scene_resource_bytes(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        let name_ptr = self.allocate(name.len() as u32 + 1);
        if name_ptr == 0 {
            return None;
        }
        self.memory.write_bytes(name_ptr, name);
        self.memory.w8(name_ptr + name.len() as u32, 0);
        let source = self.resource_by_name(name_ptr);
        self.deallocate(name_ptr);
        let stream = self.decode_resource_stream(source);
        let size = self.allocation_size(stream)?;
        let region = self.memory.region(stream, size as usize)?;
        let offset = (stream - region.base) as usize;
        let bytes = region.data[offset..offset + size as usize].to_vec();
        self.deallocate(stream);
        Some(bytes)
    }

    pub(crate) fn initialize_scene_resources(&mut self, scene: u32) -> anyhow::Result<()> {
        if scene == 0 || self.register(3) == 0 {
            return Ok(());
        }
        let filename = self.argument(4);
        let library = self.register(3);
        if filename == 0 {
            return Ok(());
        }
        let name = self.read_c_bytes(filename, 256);
        let Some(scene_bytes) = self.scene_resource_bytes(&name) else {
            return Ok(());
        };
        let Some(descriptor) = first_scene_map(&scene_bytes) else {
            return Ok(());
        };
        let Some(bytes) = self.scene_resource_bytes(&descriptor.name) else {
            return Ok(());
        };
        let Some(map) = parse_tile_map(&bytes) else {
            return Ok(());
        };
        let ids = self.allocate(map.images.len() as u32 * 2);
        if ids == 0 {
            return Ok(());
        }
        for (index, name) in map.images.iter().enumerate() {
            let name_ptr = self.allocate(name.len() as u32 + 1);
            self.memory.write_bytes(name_ptr, name);
            self.memory.w8(name_ptr + name.len() as u32, 0);
            let cpu = self.cpu;
            self.cpu.reg_set(Mode::User, 0, library);
            self.cpu.reg_set(Mode::User, 1, name_ptr);
            self.handle_picture_library_method(0x1c);
            let id = self.register(0);
            self.cpu = cpu;
            self.deallocate(name_ptr);
            if id == u32::MAX {
                self.deallocate(ids);
                return Ok(());
            }
            self.memory.w16(ids + index as u32 * 2, id as u16);
        }
        let table = self.allocate(map.columns * 4);
        let cells = self.allocate(map.columns * map.rows * 4);
        let object = self.allocate(0x48);
        if table == 0 || cells == 0 || object == 0 {
            for pointer in [ids, table, cells, object] {
                self.deallocate(pointer);
            }
            return Ok(());
        }
        self.memory.write_bytes(object, &[0; 0x48]);
        for column in 0..map.columns {
            self.memory
                .w32(table + column * 4, cells + column * map.rows * 4);
        }
        for (index, cell) in map.cells.iter().enumerate() {
            self.memory.w32(cells + index as u32 * 4, *cell);
        }
        for (offset, value) in [
            (0, map.pixel_width),
            (4, map.pixel_height),
            (8, map.tile_width),
            (12, map.tile_height),
            (16, map.columns),
            (20, map.rows),
            (24, table),
            (28, ids),
            (36, library),
        ] {
            self.memory.w32(object + offset, value);
        }
        self.memory
            .w16(object + 0x40, descriptor.position[0] as u16);
        self.memory
            .w16(object + 0x42, descriptor.position[1] as u16);
        self.memory.w16(scene, descriptor.width);
        self.memory.w16(scene + 2, descriptor.height);
        // The first map is the native scene's background render target.
        self.memory.w32(scene + 0x61c, object);
        self.memory.w32(scene + 0x640, self.register(1));
        self.memory.w32(scene + 0x644, self.register(2));
        // Native scene layouts expose the movement pair at +0x678 or +0x680.
        for (offset, method) in [
            (0x678, 0x680),
            (0x67c, 0x684),
            (0x680, 0x680),
            (0x684, 0x684),
        ] {
            self.memory.w32(
                scene + offset,
                Self::method_stub_address(super::super::METHOD_KIND_GAMEOLD, method),
            );
        }
        // Some engine builds place the window scroll pair one slot window
        // earlier: the absolute set at +0x670 and the relative adjust at
        // +0x674, with +0x668/+0x66c completing the same method block.
        // 032 士兵突袭V10 (issue #106) centres its camera through `bx r4`
        // with r4 loaded from scene+0x670 every frame and faulted on the
        // never-populated slot.  Leave non-zero slots alone: unlike the
        // fixed movement pair above, these offsets have no single
        // cross-title contract, so only empty method slots are claimable.
        for offset in [0x668u32, 0x66c, 0x670, 0x674] {
            let slot = scene + offset;
            if self.memory.r32(slot) == 0 {
                self.memory.w32(
                    slot,
                    Self::method_stub_address(super::super::METHOD_KIND_GAMEOLD, offset),
                );
            }
        }
        // Empty actor tables precede the application-owned scene payload.
        // The payload starts with the named-region count, which stays
        // non-zero for scenes whose regions build the actors: the
        // application callback owns that list and must run whenever the
        // built-in actor table is empty, receiving the stream cursor
        // positioned after both counts.
        let mut reader = Reader {
            bytes: &scene_bytes,
            position: descriptor.tail,
        };
        let empty_actor_table = reader.short() == Some(0) && reader.short().is_some();
        if empty_actor_table {
            let callback = self.argument(7);
            if callback != 0 {
                let stream = self.allocate(scene_bytes.len() as u32);
                let cursor = self.allocate(4);
                if stream == 0 || cursor == 0 {
                    self.deallocate(stream);
                    self.deallocate(cursor);
                    anyhow::bail!("cannot allocate scene callback data");
                }
                self.memory.write_bytes(stream, &scene_bytes);
                self.memory.w32(cursor, reader.position as u32);
                let cpu = self.cpu;
                let result = self.invoke_callback(
                    callback,
                    stream,
                    cursor,
                    0,
                    crate::DEFAULT_INSTRUCTION_LIMIT,
                );
                self.cpu = cpu;
                self.deallocate(stream);
                self.deallocate(cursor);
                result?;
            }
        }
        Ok(())
    }

    /// Firmware tile-map initialization into a caller-provided object
    /// (shared game service slot 72).  The guest passes the destination
    /// object in r0, the picture library that receives the map's tile
    /// images in r3, the map resource name through the fifth AAPCS
    /// argument and the tile capacity through the sixth.  The host parses
    /// the named map resource, registers each tile image in the library,
    /// lays out the column/cell tables the guest renderer walks and fills
    /// the object's method slots with callable stubs so later indirect
    /// calls stay off NULL.
    pub(crate) fn initialize_native_map(&mut self) {
        let object = self.register(0);
        let library = self.register(3);
        let filename = self.argument(4);
        // The method slots must stay callable even when the map cannot be
        // loaded: the generic constructor path filled these slots, and a
        // guest that calls a method on a half-built object must not jump
        // to NULL.
        if object != 0 {
            for offset in (NATIVE_MAP_METHOD_START..=NATIVE_MAP_METHOD_END).step_by(4) {
                self.memory.w32(
                    object + offset,
                    Self::method_stub_address(super::super::METHOD_KIND_AUTO, offset),
                );
            }
        }
        if object == 0 || filename == 0 {
            self.set_result(0);
            return;
        }
        let name = self.read_c_bytes(filename, 256);
        let Some(bytes) = self.scene_resource_bytes(&name) else {
            self.set_result(0);
            return;
        };
        let Some(map) = parse_tile_map(&bytes) else {
            self.set_result(0);
            return;
        };
        let ids = self.allocate(map.images.len() as u32 * 2);
        let table = self.allocate(map.columns * 4);
        let cells = self.allocate(map.columns * map.rows * 4);
        if ids == 0 || table == 0 || cells == 0 {
            for pointer in [ids, table, cells] {
                self.deallocate(pointer);
            }
            self.set_result(0);
            return;
        }
        for (index, name) in map.images.iter().enumerate() {
            let name_ptr = self.allocate(name.len() as u32 + 1);
            if name_ptr == 0 {
                for pointer in [ids, table, cells] {
                    self.deallocate(pointer);
                }
                self.set_result(0);
                return;
            }
            self.memory.write_bytes(name_ptr, name);
            self.memory.w8(name_ptr + name.len() as u32, 0);
            let cpu = self.cpu;
            self.cpu.reg_set(Mode::User, 0, library);
            self.cpu.reg_set(Mode::User, 1, name_ptr);
            self.handle_picture_library_method(0x1c);
            let id = self.register(0);
            self.cpu = cpu;
            self.deallocate(name_ptr);
            if id == u32::MAX {
                for pointer in [ids, table, cells] {
                    self.deallocate(pointer);
                }
                self.set_result(0);
                return;
            }
            self.memory.w16(ids + index as u32 * 2, id as u16);
        }
        for column in 0..map.columns {
            self.memory
                .w32(table + column * 4, cells + column * map.rows * 4);
        }
        for (index, cell) in map.cells.iter().enumerate() {
            self.memory.w32(cells + index as u32 * 4, *cell);
        }
        for (offset, value) in [
            (0, map.pixel_width),
            (4, map.pixel_height),
            (8, map.tile_width),
            (12, map.tile_height),
            (16, map.columns),
            (20, map.rows),
            (24, table),
            (28, ids),
            (36, library),
        ] {
            self.memory.w32(object + offset, value);
        }
        self.set_result(object);
    }

    pub(crate) fn render_scene_window(&mut self, window: u32) {
        let scene = window.wrapping_sub(0x628);
        let map = self.memory.r32(scene + 0x61c);
        if map == 0 {
            self.set_result(0);
            return;
        }
        let origin_x = self.memory.r16(scene + 0x640) as i16 as i32;
        let origin_y = self.memory.r16(scene + 0x642) as i16 as i32;
        let viewport_width = self.memory.r16(scene + 0x644) as i32;
        let viewport_height = self.memory.r16(scene + 0x646) as i32;
        let x = origin_x + self.memory.r16(map + 0x40) as i16 as i32;
        let y = origin_y + self.memory.r16(map + 0x42) as i16 as i32;
        let tile_width = self.memory.r32(map + 8) as i32;
        let tile_height = self.memory.r32(map + 12) as i32;
        let columns = self.memory.r32(map + 16);
        let rows = self.memory.r32(map + 20);
        let table = self.memory.r32(map + 24);
        let ids = self.memory.r32(map + 28);
        let library = self.memory.r32(map + 36);
        let pictures = self.memory.r32(library + 16);
        for column in 0..columns {
            let cells = self.memory.r32(table + column * 4);
            for row in 0..rows {
                let cell = self.memory.r32(cells + row * 4);
                let id = self.memory.r16(ids + ((cell >> 24) & 15) * 2) as u32;
                let image = self.memory.r32(pictures + id * 4);
                let (width, _) = self.image_dims(image);
                if width <= 0 || tile_width <= 0 || tile_height <= 0 {
                    continue;
                }
                let tiles_per_row = (width / tile_width).max(1) as u32;
                let tile = cell & 0xffffff;
                let dx = x + column as i32 * tile_width;
                let dy = y + row as i32 * tile_height;
                let left = dx.max(origin_x);
                let top = dy.max(origin_y);
                let right = (dx + tile_width).min(origin_x + viewport_width);
                let bottom = (dy + tile_height).min(origin_y + viewport_height);
                self.blit_image(
                    super::super::SCREEN_IMAGE_STRUCT,
                    image,
                    (tile % tiles_per_row) as i32 * tile_width + left - dx,
                    (tile / tiles_per_row) as i32 * tile_height + top - dy,
                    right - left,
                    bottom - top,
                    left,
                    top,
                    false,
                );
            }
        }
        self.set_result(0);
    }

    pub(crate) fn repaint_scene_window(&mut self, window: u32) -> anyhow::Result<()> {
        self.render_scene_window(window);
        // A full background repaint also invalidates the foreground windows it covers.
        let size = self.framebuffer_size();
        super::screen::invalidate_panel(&mut self.memory, window, 2, window + 24, size);
        let child = self.memory.r32(window + 36);
        let sibling = self.memory.r32(window + 32);
        self.repaint_fixed_gameold_windows(child)?;
        self.repaint_fixed_gameold_windows(sibling)?;
        self.set_result(0);
        Ok(())
    }

    /// Firmware scene-window scroll methods that some engine builds expose
    /// at scene+0x670 (absolute) and scene+0x674 (relative): r1 carries the
    /// i16 x component and r2 the i16 y component.  The viewport origin the
    /// tile renderer reads lives at scene+0x640/0x642, so writing those
    /// fields is the whole effect; the guest's per-frame repaint then draws
    /// the map at the new offset.  Unimplemented here would leave the slot
    /// at zero and the guest's `bx r4` dispatch would jump to NULL.
    pub(crate) fn scroll_scene_window(&mut self, offset: u32) {
        let scene = self.register(0);
        if scene == 0 {
            self.set_result(0);
            return;
        }
        let dx = self.register(1) as u16 as i16 as i32;
        let dy = self.register(2) as u16 as i16 as i32;
        let (next_x, next_y) = if offset == 0x670 {
            (dx, dy)
        } else {
            let origin_x = self.memory.r16(scene + 0x640) as i16 as i32;
            let origin_y = self.memory.r16(scene + 0x642) as i16 as i32;
            (origin_x + dx, origin_y + dy)
        };
        self.memory.w16(scene + 0x640, next_x as u16);
        self.memory.w16(scene + 0x642, next_y as u16);
        self.set_result(0);
    }

    pub(crate) fn query_scene_movement(&mut self, offset: u32) {
        let scene = self.register(0);
        let map = self.memory.r32(scene + 0x61c);
        if map == 0 {
            self.set_result(0);
            return;
        }
        let x = self.register(1) as i16 as i32 - self.memory.r16(map + 0x40) as i16 as i32;
        let y = self.register(2) as i16 as i32 - self.memory.r16(map + 0x42) as i16 as i32;
        let delta = self.register(3) as i16 as i32;
        let tw = self.memory.r32(map + 8) as i32;
        let th = self.memory.r32(map + 12) as i32;
        let width = self.memory.r32(map) as i32;
        let height = self.memory.r32(map + 4) as i32;
        if tw <= 0 || th <= 0 || x < 0 || y < 0 || x >= width || y >= height {
            self.set_result(0);
            return;
        }
        let horizontal = offset == 0x680;
        let coordinate = if horizontal { x } else { y };
        let tile_size = if horizontal { tw } else { th };
        let extent = self.memory.r32(map + if horizontal { 0 } else { 4 }) as i32;
        let endpoint = coordinate + delta;
        let distance = if endpoint <= 0 {
            -coordinate
        } else if endpoint >= extent - 1 {
            extent - 1 - coordinate
        } else {
            let from = coordinate / tile_size;
            let to = endpoint / tile_size;
            let table = self.memory.r32(map + 24);
            let mut cell_at = |column: i32, row: i32| {
                let cells = self.memory.r32(table + column as u32 * 4);
                self.memory.r32(cells + row as u32 * 4) >> 28
            };
            let (first, last) = if horizontal {
                (cell_at(from, y / th), cell_at(to, y / th))
            } else {
                (cell_at(x / tw, from), cell_at(x / tw, to))
            };
            // Edge bits are left, up, right, down; inspect both crossed edges.
            let blocked = from != to
                && if horizontal {
                    if delta > 0 {
                        first & 4 != 0 || last & 1 != 0
                    } else {
                        first & 1 != 0 || last & 4 != 0
                    }
                } else if delta > 0 {
                    first & 8 != 0 || last & 2 != 0
                } else {
                    first & 2 != 0 || last & 8 != 0
                };
            if blocked {
                if delta > 0 {
                    to * tile_size - 1 - coordinate
                } else {
                    from * tile_size - coordinate
                }
            } else {
                delta
            }
        };
        self.set_result(distance as i16 as i32 as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window scroll pair sits one slot block earlier than the movement
    /// pair on some engine builds: +0x670 sets the viewport origin outright
    /// and +0x674 adjusts it, both taking a signed i16 x/y in r1/r2.  032
    /// 士兵突袭V10 loads these slots into `r4` and dispatches through
    /// `bx r4` every frame, so an unpopulated slot faulted on NULL (#106).
    #[test]
    fn scene_window_scroll_methods_move_the_viewport_origin() {
        let mut machine = CoolbarMachine::new_blank_for_tests();
        let scene = machine.allocate(0x700);
        machine.cpu.reg_set(Mode::User, 0, scene);
        machine.cpu.reg_set(Mode::User, 1, 40);
        machine.cpu.reg_set(Mode::User, 2, 24);
        machine.scroll_scene_window(0x670);
        assert_eq!(machine.memory.r16(scene + 0x640), 40);
        assert_eq!(machine.memory.r16(scene + 0x642), 24);

        // The method writes its result to r0, so the caller restores the scene.
        machine.cpu.reg_set(Mode::User, 0, scene);
        machine.cpu.reg_set(Mode::User, 1, (-8i32) as u32);
        machine.cpu.reg_set(Mode::User, 2, 4);
        machine.scroll_scene_window(0x674);
        assert_eq!(machine.memory.r16(scene + 0x640) as i16, 32);
        assert_eq!(machine.memory.r16(scene + 0x642) as i16, 28);
        assert_eq!(machine.register(0), 0, "the method reports success");
    }

    #[test]
    fn scene_window_scroll_ignores_a_null_scene() {
        let mut machine = CoolbarMachine::new_blank_for_tests();
        machine.cpu.reg_set(Mode::User, 0, 0);
        machine.scroll_scene_window(0x670);
        assert_eq!(machine.register(0), 0);
    }

    #[test]
    fn scene_initialization_exposes_real_map_cells_and_picture_library() {
        fn packed(bytes: &[u8]) -> Vec<u8> {
            let mut payload = vec![2];
            payload.extend_from_slice(&(bytes.len() as u32 + 1).to_be_bytes());
            payload.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            payload.push(0x80 | bytes.len() as u8);
            payload.extend_from_slice(bytes);
            payload
        }
        for big_endian in [false, true] {
            let mut machine = CoolbarMachine::new_blank_for_tests();
            let mut memory = super::super::super::memory::MachineMemory::new(big_endian);
            memory.regions = std::mem::take(&mut machine.memory.regions);
            machine.memory = memory;
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(32, 32, |x, y| {
                image::Rgba(match (x < 16, y < 16) {
                    (true, true) => [255, 0, 0, 255],
                    (false, true) => [0, 255, 0, 255],
                    (true, false) => [0, 0, 255, 255],
                    _ => [255, 255, 0, 255],
                })
            }))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
            let mut map = 1u32.to_le_bytes().to_vec();
            map.push(5);
            map.extend_from_slice(b"a.png");
            for value in [32u32, 32, 16, 16, 0, 1, 2, 3] {
                map.extend_from_slice(&value.to_le_bytes());
            }
            let mut scene_bytes = b"SCE2".to_vec();
            for value in [32u16, 32, 1] {
                scene_bytes.extend_from_slice(&value.to_le_bytes());
            }
            scene_bytes.push(5);
            scene_bytes.extend_from_slice(b"a.map");
            for value in [4u16, 8, 1, 1] {
                scene_bytes.extend_from_slice(&value.to_le_bytes());
            }
            scene_bytes.extend_from_slice(&[0, 0, 0, 0, 42, 0]);
            for (name, data) in [
                ("a.sce", packed(&scene_bytes)),
                ("a.map", packed(&map)),
                ("a.png", png.into_inner()),
            ] {
                machine
                    .resources
                    .push(super::super::super::packages::HostResource {
                        name: name.into(),
                        data,
                    });
            }
            let package = machine.allocate(104);
            machine.initialize_data_package(package, 5);
            machine.load_main_resource_package(package);
            let scene = machine.allocate(0x700);
            let library = machine.allocate(0x60);
            let ids = machine.allocate(8);
            let images = machine.allocate(16);
            machine.memory.w16(library + 8, 4);
            machine.memory.w32(library + 12, ids);
            machine.memory.w32(library + 16, images);
            let name = machine.allocate(6);
            machine.memory.write_bytes(name, b"a.sce\0");
            let stack = machine.allocate(32);
            let marker = machine.allocate(4);
            let callback = machine.allocate(16);
            for (index, instruction) in [0x6808, 0x4a02, 0x6010, 0x2000, 0x4770, 0x46c0]
                .into_iter()
                .enumerate()
            {
                machine.memory.w16(callback + index as u32 * 2, instruction);
            }
            machine.memory.w32(callback + 12, marker);
            machine.cpu.reg_set(Mode::User, 13, stack);
            machine.memory.w32(stack, name);
            machine.memory.w32(stack + 12, callback | 1);
            machine.cpu.reg_set(Mode::User, 0, scene);
            machine.cpu.reg_set(Mode::User, 1, 0x000a000a);
            machine.cpu.reg_set(Mode::User, armv4t_emu::reg::CPSR, 0x30);
            machine.cpu.reg_set(Mode::User, 2, 0x00200020);
            machine.cpu.reg_set(Mode::User, 3, library);
            machine.initialize_scene_resources(scene).unwrap();
            assert_eq!(machine.register(0), scene);
            assert_eq!(machine.register(3), library);
            assert_eq!(machine.register(13), stack);
            assert_eq!(machine.memory.r32(marker), 28);
            for (offset, method) in [
                (0x678, 0x680),
                (0x67c, 0x684),
                (0x680, 0x680),
                (0x684, 0x684),
            ] {
                assert_eq!(
                    machine.memory.r32(scene + offset),
                    CoolbarMachine::method_stub_address(
                        super::super::super::METHOD_KIND_GAMEOLD,
                        method
                    )
                );
            }
            let object = machine.memory.r32(scene + 0x61c);
            assert_ne!(object, 0);
            assert_eq!(machine.memory.r32(object + 8), 16);
            assert_eq!(machine.memory.r32(object + 12), 16);
            assert_eq!(machine.memory.r32(object + 36), library);
            assert_eq!(machine.memory.r16(object + 0x40), 4);
            assert_eq!(machine.memory.r16(object + 0x42), 8);
            let columns = machine.memory.r32(object + 24);
            for column in 0..2 {
                let cells = machine.memory.r32(columns + column * 4);
                for row in 0..2 {
                    assert_eq!(machine.memory.r32(cells + row * 4), column * 2 + row);
                }
            }
            assert_eq!(machine.memory.r16(library + 20), 1);
            let repaint = CoolbarMachine::method_stub_address(
                super::super::super::METHOD_KIND_GAMEOLD,
                0x660,
            );
            machine
                .invoke_callback(
                    repaint,
                    scene + 0x628,
                    0,
                    0,
                    crate::DEFAULT_INSTRUCTION_LIMIT,
                )
                .unwrap();
            let stride = machine.display_surface.stride();
            let pixel = |x: u32, y: u32| super::super::super::SCREEN_IMAGE + (y * stride + x) * 2;
            assert_eq!(machine.memory.r16(pixel(14, 18)), 0xf800);
            assert_eq!(machine.memory.r16(pixel(30, 18)), 0x001f);
            assert_eq!(machine.memory.r16(pixel(14, 34)), 0x07e0);
            assert_eq!(machine.memory.r16(pixel(30, 34)), 0xffe0);
            assert_eq!(machine.memory.r16(pixel(42, 18)), 0);
            assert_eq!(machine.memory.r16(pixel(14, 42)), 0);

            let painter = machine.allocate(16);
            for (index, instruction) in [0x6801, 0x3101, 0x6001, 0x6841, 0x6882, 0x8011, 0x4770]
                .into_iter()
                .enumerate()
            {
                machine.memory.w16(painter + index as u32 * 2, instruction);
            }
            let mut panels = Vec::new();
            let mut contexts = Vec::new();
            for _ in 0..3 {
                let panel = machine.allocate(72);
                let entries = machine.allocate(4);
                let rectangle = machine.allocate(8);
                let context = machine.allocate(12);
                machine.memory.w32(panel + 8, 1);
                machine.memory.w32(panel + 12, entries);
                machine.memory.w32(entries, rectangle);
                machine.memory.w32(panel + 48, painter | 1);
                machine.memory.w32(panel + 20, context);
                machine.memory.w32(context + 4, 0xffff);
                machine.memory.w32(context + 8, pixel(14, 18));
                panels.push(panel);
                contexts.push(context);
            }
            machine.executable.big_endian = true;
            machine.executable.preferred_code_address = 0x0400_0000;
            machine.cpu.reg_set(Mode::User, 0, scene);
            machine.handle_game_service(77);
            let add_child = machine.memory.r32(scene + 0x650);
            machine
                .invoke_callback(
                    add_child,
                    scene + 0x628,
                    panels[0],
                    1,
                    crate::DEFAULT_INSTRUCTION_LIMIT,
                )
                .unwrap();
            assert_eq!(machine.memory.r32(scene + 0x628 + 36), panels[0]);
            machine.memory.w32(panels[0] + 36, panels[1]);
            machine.memory.w32(scene + 0x628 + 32, panels[2]);
            for count in 1..=3 {
                machine
                    .invoke_callback(
                        repaint,
                        scene + 0x628,
                        0,
                        0,
                        crate::DEFAULT_INSTRUCTION_LIMIT,
                    )
                    .unwrap();
                assert_eq!(machine.memory.r16(pixel(14, 18)), 0xffff);
                for (&panel, &context) in panels.iter().zip(&contexts) {
                    assert_eq!(machine.memory.r32(context), count);
                    assert_eq!(machine.memory.r32(panel + 4), 0);
                }
                assert_eq!(machine.register(13), stack);
            }
            machine.memory.w32(panels[0] + 48, 0x4000_0001);
            assert!(machine.repaint_scene_window(scene + 0x628).is_err());
            assert_eq!(machine.register(13), stack);
            machine.cpu.reg_set(Mode::User, 0, scene);
            machine.memory.w32(stack + 12, 0x4000_0001);
            assert!(machine.initialize_scene_resources(scene).is_err());
            assert_eq!(machine.register(0), scene);
            assert_eq!(machine.register(3), library);
            assert_eq!(machine.register(13), stack);
        }
    }

    #[test]
    fn scene_movement_checks_directional_edges_and_signed_distances() {
        for big_endian in [false, true] {
            let mut machine = CoolbarMachine::new_blank_for_tests();
            let mut memory = super::super::super::memory::MachineMemory::new(big_endian);
            memory.regions = std::mem::take(&mut machine.memory.regions);
            machine.memory = memory;
            let scene = machine.allocate(0x700);
            let map = machine.allocate(0x48);
            let table = machine.allocate(16);
            let cells = machine.allocate(64);
            machine.memory.w32(scene + 0x61c, map);
            for (offset, value) in [
                (0, 64),
                (4, 64),
                (8, 16),
                (12, 16),
                (16, 4),
                (20, 4),
                (24, table),
            ] {
                machine.memory.w32(map + offset, value);
            }
            machine.memory.w16(map + 0x40, 4);
            machine.memory.w16(map + 0x42, 8);
            for column in 0..4 {
                machine.memory.w32(table + column * 4, cells + column * 16);
            }
            for (offset, delta, from_flag, to_flag, expected, destination) in [
                (0x680, 16i32, 4, 1, 11i32, 9),
                (0x680, -16, 1, 4, -4, 1),
                (0x684, 16, 8, 2, 11, 6),
                (0x684, -16, 2, 8, -4, 4),
            ] {
                for (cell, flag) in [(5, from_flag), (destination, to_flag)] {
                    machine.memory.write_bytes(cells, &[0; 64]);
                    machine.memory.w32(cells + cell * 4, flag << 28);
                    machine.cpu.reg_set(Mode::User, 0, scene);
                    machine.cpu.reg_set(Mode::User, 1, 24);
                    machine.cpu.reg_set(Mode::User, 2, 28);
                    machine.cpu.reg_set(Mode::User, 3, delta as u32);
                    machine.handle_method_stub(
                        CoolbarMachine::method_stub_address(
                            super::super::super::METHOD_KIND_GAMEOLD,
                            offset,
                        ) & !1,
                    );
                    assert_eq!(machine.register(0) as i32, expected);
                }
                machine.memory.write_bytes(cells, &[0; 64]);
                machine.cpu.reg_set(Mode::User, 0, scene);
                machine.query_scene_movement(offset);
                assert_eq!(machine.register(0) as i32, delta);
            }
        }
    }

    #[test]
    #[ignore = "requires local CBE game assets (set COOLBAR_GAME_DIR)"]
    fn real_content_scene_start_and_repeated_taps_remain_ready() {
        let directory = std::env::var_os("COOLBAR_GAME_DIR").expect("COOLBAR_GAME_DIR is not set");
        for game in ["魔塔V10.CBE", "魔塔(新品).CBE"] {
            let archive =
                crate::CbeArchive::load(&std::path::PathBuf::from(&directory).join(game)).unwrap();
            let mut machine = CoolbarMachine::new(&archive).unwrap();
            machine.boot(crate::DEFAULT_INSTRUCTION_LIMIT).unwrap();
            for frame in 0..2000 {
                if [20, 60, 100, 140, 180].contains(&frame) {
                    let (x, y) = if game == "魔塔(新品).CBE" && frame == 60 {
                        (20, 310)
                    } else {
                        (120, 183)
                    };
                    machine.set_pointer(x, y, true);
                }
                machine.run_frame(crate::DEFAULT_INSTRUCTION_LIMIT).unwrap();
                machine.set_pointer(120, 183, false);
            }
            assert_eq!(machine.state(), crate::MachineState::Ready, "{game}");
            assert_eq!(machine.display_size(), (240, 320), "{game}");
        }
    }

    #[test]
    fn scene_map_keeps_canvas_name_and_signed_placement() {
        let mut bytes = b"SCE2".to_vec();
        for value in [240u16, 320, 1] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.push(5);
        bytes.extend_from_slice(b"a.map");
        for value in [(-8i16) as u16, 12, 1, 1] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let map = first_scene_map(&bytes).unwrap();
        assert_eq!((map.width, map.height), (240, 320));
        assert_eq!(map.name, b"a.map");
        assert_eq!(map.position, [-8, 12]);
        bytes.pop();
        assert!(first_scene_map(&bytes).is_none());
    }

    #[test]
    fn tile_map_preserves_column_order_image_slots_and_dimensions() {
        let mut bytes = 2u32.to_le_bytes().to_vec();
        for name in [b"a.gif", b"b.gif"] {
            bytes.push(name.len() as u8);
            bytes.extend_from_slice(name);
        }
        for value in [
            32u32, 48, 16, 16, 0, 1, 2, 0x01000003, 0x01000004, 0x01000005,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let map = parse_tile_map(&bytes).unwrap();
        assert_eq!(map.images, [b"a.gif".to_vec(), b"b.gif".to_vec()]);
        assert_eq!((map.columns, map.rows), (2, 3));
        assert_eq!((map.tile_width, map.tile_height), (16, 16));
        assert_eq!(map.cells, [0, 1, 2, 0x01000003, 0x01000004, 0x01000005]);
        bytes.pop();
        assert!(parse_tile_map(&bytes).is_none());
        let mut invalid = bytes.clone();
        invalid[20..24].copy_from_slice(&0u32.to_le_bytes());
        assert!(parse_tile_map(&invalid).is_none());
    }

    #[test]
    fn native_map_initialization_fills_caller_object_cells_and_library() {
        fn packed(bytes: &[u8]) -> Vec<u8> {
            let mut payload = vec![2];
            payload.extend_from_slice(&(bytes.len() as u32 + 1).to_be_bytes());
            payload.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            payload.push(0x80 | bytes.len() as u8);
            payload.extend_from_slice(bytes);
            payload
        }
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(32, 32, |x, y| {
            image::Rgba(match (x < 16, y < 16) {
                (true, true) => [255, 0, 0, 255],
                (false, true) => [0, 255, 0, 255],
                (true, false) => [0, 0, 255, 255],
                _ => [255, 255, 0, 255],
            })
        }))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        // One tile image over a 32x32 pixel map of 16x16 tiles: 2x2 cells.
        let mut map = 1u32.to_le_bytes().to_vec();
        map.push(5);
        map.extend_from_slice(b"a.png");
        for value in [32u32, 32, 16, 16, 0, 1, 2, 3] {
            map.extend_from_slice(&value.to_le_bytes());
        }
        let mut machine = CoolbarMachine::new_blank_for_tests();
        let mut memory = super::super::super::memory::MachineMemory::new(true);
        memory.regions = std::mem::take(&mut machine.memory.regions);
        machine.memory = memory;
        // The fixed-manager ABI is what routes shared game service slot 72
        // to the tile-map builder; the real guests are big-endian.
        machine.executable.big_endian = true;
        machine.executable.preferred_code_address = 0x0010_0000;
        machine.executable.code_image_size = 0x2000;
        machine.executable.code_size = 4;
        for (name, data) in [
            ("a.map", packed(&map)),
            ("a.png", png.into_inner()),
            ("b.map", packed(&map)),
        ] {
            machine
                .resources
                .push(super::super::super::packages::HostResource {
                    name: name.into(),
                    data,
                });
        }
        let package = machine.allocate(104);
        machine.initialize_data_package(package, 5);
        machine.load_main_resource_package(package);
        let object = machine.allocate(0x100);
        let library = machine.allocate(0x60);
        let ids = machine.allocate(8);
        let images = machine.allocate(16);
        machine.memory.w16(library + 8, 4);
        machine.memory.w32(library + 12, ids);
        machine.memory.w32(library + 16, images);
        let name = machine.allocate(6);
        machine.memory.write_bytes(name, b"a.map\0");
        let missing_name = machine.allocate(6);
        machine.memory.write_bytes(missing_name, b"z.map\0");
        let stack = machine.allocate(32);
        machine.cpu.reg_set(Mode::User, 13, stack);
        machine.memory.w32(stack, name);
        machine.memory.w32(stack + 4, 20);
        machine.cpu.reg_set(Mode::User, 0, object);
        machine.cpu.reg_set(Mode::User, 3, library);
        // Dispatching through the shared service table (not the helper
        // directly) keeps the slot-72 wiring itself under test.
        machine.handle_game_service(72);
        // The caller's object comes back as the result with the map laid out.
        assert_eq!(machine.register(0), object);
        assert_eq!(machine.memory.r32(object), 32);
        assert_eq!(machine.memory.r32(object + 4), 32);
        assert_eq!(machine.memory.r32(object + 8), 16);
        assert_eq!(machine.memory.r32(object + 12), 16);
        assert_eq!(machine.memory.r32(object + 16), 2);
        assert_eq!(machine.memory.r32(object + 20), 2);
        assert_eq!(machine.memory.r32(object + 36), library);
        let table = machine.memory.r32(object + 24);
        let cell_ids = machine.memory.r32(object + 28);
        assert_ne!(table, 0);
        assert_ne!(cell_ids, 0);
        for column in 0..2 {
            let cells = machine.memory.r32(table + column * 4);
            for row in 0..2 {
                assert_eq!(machine.memory.r32(cells + row * 4), column * 2 + row);
            }
        }
        // The map's tile image is registered in the caller's library.
        assert_eq!(machine.memory.r16(library + 20), 1);
        assert_ne!(machine.memory.r32(images), 0);
        // Method slots stay callable so later indirect calls land on stubs.
        let stub = CoolbarMachine::method_stub_address(super::super::super::METHOD_KIND_AUTO, 0x6c);
        assert_eq!(machine.memory.r32(object + 0x6c), stub);
        // A missing map resource still leaves the slots callable and reports failure.
        let missing = machine.allocate(0x100);
        machine.cpu.reg_set(Mode::User, 0, missing);
        machine.cpu.reg_set(Mode::User, 3, library);
        machine.memory.w32(stack, missing_name);
        machine.handle_game_service(72);
        assert_eq!(machine.register(0), 0);
        assert_eq!(machine.memory.r32(missing + 0x6c), stub);
    }
}
