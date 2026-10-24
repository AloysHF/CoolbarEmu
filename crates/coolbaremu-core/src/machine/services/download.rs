//! Download, payment, and download-resource services (groups 22-26).

use armv4t_emu::Memory;

use super::super::{CoolbarMachine, DREAM_FACTORY_PACKAGE_SLOT, HEAP_SIZE};

impl CoolbarMachine {
    pub(crate) fn handle_download_service(&mut self, index: u32) {
        if index == 4 {
            self.set_result(u32::MAX);
        } else {
            self.set_result(0);
        }
    }

    pub(crate) fn handle_payment_service(&mut self, index: u32) {
        if index == 7 {
            let destination = self.register(0);
            let capacity = self.register(1);
            let identifier = b"111111111111111\0";
            let count = capacity.min(identifier.len() as u32);
            if destination != 0 && count != 0 {
                self.memory
                    .write_bytes(destination, &identifier[..count as usize]);
                self.memory.w8(destination + count - 1, 0);
            }
        }
        self.set_result(0);
    }

    pub(crate) fn handle_download_resource_service(&mut self, index: u32) {
        match index {
            0 => {
                let package = self.memory.r32(DREAM_FACTORY_PACKAGE_SLOT);
                self.set_result(package);
            }
            10 => {
                let result = self.create_image_from_stream(self.register(0), self.register(1));
                self.set_result(result);
            }
            _ => self.set_result(0),
        }
    }

    pub(crate) fn handle_download_image_service(&mut self, index: u32) {
        if index == 4 {
            let pointer = self.allocate(self.register(0));
            self.set_result(pointer);
        } else {
            self.set_result(0);
        }
    }

    pub(crate) fn read_length_prefixed_string(
        &mut self,
        buffer: u32,
        cursor: u32,
        wide_length: bool,
    ) -> u32 {
        let mut offset = self.memory.r32(cursor);
        let length = if wide_length {
            let address = buffer.wrapping_add(offset);
            // Stream lengths are little-endian; the guest cursor keeps native byte order.
            let length = u32::from_le_bytes([
                self.memory.r8(address),
                self.memory.r8(address.wrapping_add(1)),
                self.memory.r8(address.wrapping_add(2)),
                self.memory.r8(address.wrapping_add(3)),
            ]);
            offset = offset.wrapping_add(4);
            length
        } else {
            let length = self.memory.r8(buffer.wrapping_add(offset)) as u32;
            offset = offset.wrapping_add(1);
            length
        };
        if length > HEAP_SIZE as u32 {
            return 0;
        }
        let result = self.allocate(length.saturating_add(1).max(1));
        for index in 0..length {
            let byte = self.memory.r8(buffer.wrapping_add(offset + index));
            self.memory.w8(result + index, byte);
        }
        self.memory.w8(result + length, 0);
        self.memory.w32(cursor, offset.wrapping_add(length));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::memory::MachineMemory;

    #[test]
    fn string_stream_lengths_are_independent_of_guest_byte_order() {
        for big_endian in [false, true] {
            for wide_length in [false, true] {
                let mut machine = CoolbarMachine::new_blank_for_tests();
                let mut memory = MachineMemory::new(big_endian);
                memory.regions = std::mem::take(&mut machine.memory.regions);
                machine.memory = memory;
                let buffer = machine.allocate(64);
                let cursor = machine.allocate(4);
                let mut stream = vec![0xaa, 0xbb];
                if wide_length {
                    stream.extend_from_slice(&3u32.to_le_bytes());
                } else {
                    stream.push(3);
                }
                stream.extend_from_slice(b"abc");
                stream.extend_from_slice(&0x1234_5678u32.to_le_bytes());
                machine.memory.write_bytes(buffer, &stream);
                machine.memory.w32(cursor, 2);
                machine.set_user_register(0, buffer);
                machine.set_user_register(1, cursor);
                machine.handle_game_util_service(if wide_length { 29 } else { 25 });
                let result = machine.register(0);
                assert_ne!(result, 0, "stream length must not use guest byte order");
                assert_eq!(machine.read_c_bytes(result, 16), b"abc");
                let expected_cursor = if wide_length { 9 } else { 6 };
                assert_eq!(machine.memory.r32(cursor), expected_cursor);
                assert_eq!(machine.memory.r8(result + 3), 0);
                machine.set_user_register(0, buffer);
                machine.set_user_register(1, cursor);
                machine.handle_game_util_service(20);
                assert_eq!(machine.register(0), 0x1234_5678);
                assert_eq!(machine.memory.r32(cursor), expected_cursor + 4);
            }
        }
    }
}
