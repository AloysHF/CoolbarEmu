//! Scene maps exposed to native rendering callbacks.

use armv4t_emu::{Memory, Mode};

use super::super::NicaiMachine;

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
}

fn first_scene_map(bytes: &[u8]) -> Option<SceneMap> {
    let mut reader = Reader { bytes, position: 0 };
    if reader.take(4)? != b"SCE2" {
        return None;
    }
    let width = reader.short()?;
    let height = reader.short()?;
    if width == 0 || height == 0 || reader.short()? == 0 {
        return None;
    }
    let name = reader.name()?.to_vec();
    let position = [reader.short()? as i16, reader.short()? as i16];
    reader.take(4)?;
    Some(SceneMap {
        width,
        height,
        name,
        position,
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

impl NicaiMachine {
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

    pub(crate) fn initialize_scene_resources(&mut self, scene: u32) {
        if scene == 0 || self.register(3) == 0 {
            return;
        }
        let filename = self.argument(4);
        let library = self.register(3);
        if filename == 0 {
            return;
        }
        let name = self.read_c_bytes(filename, 256);
        let Some(bytes) = self.scene_resource_bytes(&name) else {
            return;
        };
        let Some(descriptor) = first_scene_map(&bytes) else {
            return;
        };
        let Some(bytes) = self.scene_resource_bytes(&descriptor.name) else {
            return;
        };
        let Some(map) = parse_tile_map(&bytes) else {
            return;
        };
        let ids = self.allocate(map.images.len() as u32 * 2);
        if ids == 0 {
            return;
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
                return;
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
            return;
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let mut machine = NicaiMachine::new_blank_for_tests();
            let mut memory = super::super::super::memory::MachineMemory::new(big_endian);
            memory.regions = std::mem::take(&mut machine.memory.regions);
            machine.memory = memory;
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                32,
                32,
                image::Rgba([255, 0, 0, 255]),
            ))
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
            machine.cpu.reg_set(Mode::User, 13, stack);
            machine.memory.w32(stack, name);
            machine.cpu.reg_set(Mode::User, 0, scene);
            machine.cpu.reg_set(Mode::User, 3, library);
            machine.initialize_scene_resources(scene);
            assert_eq!(machine.register(0), scene);
            assert_eq!(machine.register(3), library);
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
}
