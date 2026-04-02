use std::collections::HashMap;

use super::lower::{MachineFunction, MachineReg};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpillSlot(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RegAllocMapping {
    pub registers: HashMap<MachineReg, MachineReg>,
    pub spills: HashMap<MachineReg, SpillSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RegAllocResult {
    pub mapping: RegAllocMapping,
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterAllocator {
    pub max_registers: u16,
}

impl RegisterAllocator {
    pub const fn new(max_registers: u16) -> Self {
        Self { max_registers }
    }

    pub fn allocate(&self, _function: &mut MachineFunction) -> RegAllocResult {
        let _ = self.max_registers;
        RegAllocResult::default()
    }
}

impl Default for RegisterAllocator {
    fn default() -> Self {
        Self::new(16)
    }
}
