//! Sparse guest memory regions with byte-order-aware access.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use armv4t_emu::Memory;
use serde::{Deserialize, Serialize};

/// Public snapshot of one mapped guest memory region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegionInfo {
    /// Guest base address of the region.
    pub base: u32,
    /// Size of the region in bytes.
    pub size: usize,
    /// Whether guest writes to the region are rejected.
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Region {
    pub(crate) base: u32,
    pub(crate) data: Vec<u8>,
    pub(crate) read_only: bool,
}

/// Read up to the first four bytes as a little-endian word, zero-padding the
/// tail. Watch records only need a comparable snapshot, not guest byte order.
fn little_endian_word(bytes: &[u8]) -> u32 {
    let mut word = [0u8; 4];
    let used = bytes.len().min(4);
    word[..used].copy_from_slice(&bytes[..used]);
    u32::from_le_bytes(word)
}

/// One recorded write into a watched guest memory range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteWatchEntry {
    /// Program counter of the instruction that stored, or the last executed
    /// instruction when the write came from a host service instead.
    pub pc: u32,
    /// Written guest address.
    pub address: u32,
    /// Written size in bytes (1, 2, or 4 for stores; more for bulk loads).
    pub size: usize,
    /// Bytes written, little-endian normalized into the low bytes.
    pub value: u32,
    /// Bytes that were at the address before the write.
    pub previous: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct WriteWatch {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) pc: u32,
    pub(crate) log: Vec<WriteWatchEntry>,
}

impl WriteWatch {
    const LOG_CAPACITY: usize = 8192;

    fn covers(&self, address: u32, size: usize) -> bool {
        address < self.end && address.wrapping_add(size as u32) > self.start
    }

    fn record(&mut self, pc: u32, address: u32, size: usize, value: u32, previous: u32) {
        if self.log.len() < Self::LOG_CAPACITY {
            self.log.push(WriteWatchEntry {
                pc,
                address,
                size,
                value,
                previous,
            });
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MachineMemory {
    pub(crate) regions: Vec<Region>,
    pub(crate) bad_accesses: BTreeSet<u32>,
    /// Debug watch over guest writes into one address range. Not serialized:
    /// it only exists while a diagnosis session is armed.
    #[serde(skip, default)]
    pub(crate) watch: Option<WriteWatch>,
    big_endian: bool,
}

impl MachineMemory {
    pub(crate) fn new(big_endian: bool) -> Self {
        Self {
            regions: Vec::new(),
            bad_accesses: BTreeSet::new(),
            watch: None,
            big_endian,
        }
    }

    /// Arm a write watch over `[start, end)`; the CPU loop feeds the current
    /// program counter through [`Self::set_watch_pc`] so recorded writes can
    /// name the storing instruction.
    pub(crate) fn arm_watch(&mut self, start: u32, end: u32) {
        self.watch = Some(WriteWatch {
            start,
            end,
            pc: 0,
            log: Vec::new(),
        });
    }

    pub(crate) fn disarm_watch(&mut self) {
        self.watch = None;
    }

    pub(crate) fn set_watch_pc(&mut self, pc: u32) {
        if let Some(watch) = &mut self.watch {
            watch.pc = pc;
        }
    }

    /// Take the recorded watch entries, leaving the watch armed.
    pub(crate) fn take_watch_log(&mut self) -> Vec<WriteWatchEntry> {
        match &mut self.watch {
            Some(watch) => std::mem::take(&mut watch.log),
            None => Vec::new(),
        }
    }

    pub(crate) fn map(&mut self, base: u32, size: usize, read_only: bool) {
        self.regions.push(Region {
            base,
            data: vec![0; size],
            read_only,
        });
    }

    /// Addresses of guest accesses (reads or writes) that landed outside any
    /// mapped region, in ascending order. Used to diagnose faults caused by
    /// guests dereferencing tables the host never populated.
    pub(crate) fn unmapped_accesses(&self) -> &BTreeSet<u32> {
        &self.bad_accesses
    }

    /// Whether guest multi-byte values are stored big-endian.
    pub(crate) fn is_big_endian(&self) -> bool {
        self.big_endian
    }

    /// Record an upcoming write into the watched range, if one is armed and
    /// covers the address. Called before the write lands so the previous
    /// bytes are captured.
    fn record_watch(&mut self, address: u32, data: &[u8]) {
        let covers = self
            .watch
            .as_ref()
            .is_some_and(|watch| watch.covers(address, data.len()));
        if !covers {
            return;
        }
        let previous = self
            .region(address, data.len())
            .map(|region| {
                let offset = (address - region.base) as usize;
                little_endian_word(&region.data[offset..offset + data.len()])
            })
            .unwrap_or(0);
        let value = little_endian_word(data);
        if let Some(watch) = &mut self.watch {
            let pc = watch.pc;
            watch.record(pc, address, data.len(), value, previous);
        }
    }

    pub(crate) fn load(&mut self, address: u32, data: &[u8]) -> Result<()> {
        self.record_watch(address, data);
        let region = self
            .region_mut(address, data.len())
            .context("load address is not mapped")?;
        let offset = (address - region.base) as usize;
        region.data[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    pub(crate) fn write_bytes(&mut self, address: u32, data: &[u8]) -> bool {
        self.record_watch(address, data);
        let Some(region) = self.region_mut(address, data.len()) else {
            self.bad_accesses.insert(address);
            return false;
        };
        if region.read_only {
            self.bad_accesses.insert(address);
            return false;
        }
        let offset = (address - region.base) as usize;
        region.data[offset..offset + data.len()].copy_from_slice(data);
        true
    }

    pub(crate) fn read_bytes(&mut self, address: u32, size: usize) -> Option<Vec<u8>> {
        let Some(region) = self.region(address, size) else {
            self.bad_accesses.insert(address);
            return None;
        };
        let offset = (address - region.base) as usize;
        Some(region.data[offset..offset + size].to_vec())
    }

    pub(crate) fn region(&self, address: u32, size: usize) -> Option<&Region> {
        self.regions.iter().find(|region| {
            let offset = address.wrapping_sub(region.base) as usize;
            address >= region.base
                && offset
                    .checked_add(size)
                    .is_some_and(|end| end <= region.data.len())
        })
    }

    pub(crate) fn region_mut(&mut self, address: u32, size: usize) -> Option<&mut Region> {
        self.regions.iter_mut().find(|region| {
            let offset = address.wrapping_sub(region.base) as usize;
            address >= region.base
                && offset
                    .checked_add(size)
                    .is_some_and(|end| end <= region.data.len())
        })
    }

    pub(crate) fn read(&mut self, address: u32, size: usize) -> u32 {
        if let Some(region) = self.region(address, size) {
            let offset = (address - region.base) as usize;
            let mut bytes = [0u8; 4];
            if self.big_endian {
                bytes[4 - size..].copy_from_slice(&region.data[offset..offset + size]);
                u32::from_be_bytes(bytes)
            } else {
                bytes[..size].copy_from_slice(&region.data[offset..offset + size]);
                u32::from_le_bytes(bytes)
            }
        } else {
            self.bad_accesses.insert(address);
            0
        }
    }

    pub(crate) fn write(&mut self, address: u32, value: u32, size: usize) {
        let big_endian = self.big_endian;
        let bytes = if big_endian {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        let source = if big_endian {
            &bytes[4 - size..]
        } else {
            &bytes[..size]
        };
        self.record_watch(address, source);
        if let Some(region) = self.region_mut(address, size) {
            if region.read_only {
                self.bad_accesses.insert(address);
                return;
            }
            let offset = (address - region.base) as usize;
            region.data[offset..offset + size].copy_from_slice(source);
        } else {
            self.bad_accesses.insert(address);
        }
    }
}

impl Memory for MachineMemory {
    fn r8(&mut self, addr: u32) -> u8 {
        self.read(addr, 1) as u8
    }
    fn r16(&mut self, addr: u32) -> u16 {
        self.read(addr, 2) as u16
    }
    fn r32(&mut self, addr: u32) -> u32 {
        self.read(addr, 4)
    }
    fn w8(&mut self, addr: u32, val: u8) {
        self.write(addr, val as u32, 1)
    }
    fn w16(&mut self, addr: u32, val: u16) {
        self.write(addr, val as u32, 2)
    }
    fn w32(&mut self, addr: u32, val: u32) {
        self.write(addr, val, 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_memory_honors_guest_endianness() {
        for big_endian in [false, true] {
            let mut memory = MachineMemory::new(big_endian);
            memory.map(0x1000, 16, false);
            memory.w8(0x1000, 0x12);
            memory.w16(0x1002, 0x3456);
            memory.w32(0x1004, 0x789a_bcde);
            assert_eq!(memory.r8(0x1000), 0x12);
            assert_eq!(memory.r16(0x1002), 0x3456);
            assert_eq!(memory.r32(0x1004), 0x789a_bcde);
        }
    }
}
