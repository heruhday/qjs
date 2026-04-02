use super::lower::MachineFunction;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MachineCode {
    pub bytes: Vec<u8>,
    pub entry_offset: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MachineCodeEmitter;

impl MachineCodeEmitter {
    pub fn emit(&self, _function: &MachineFunction) -> MachineCode {
        MachineCode::default()
    }
}
