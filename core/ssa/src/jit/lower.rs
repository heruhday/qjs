use crate::ir::{IRFunction, IRTerminator, IRValue};
use value::JSValue;

pub type MachineBlockId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MachineReg(pub u16);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineOperand {
    Reg(MachineReg),
    Value(IRValue),
    Immediate(JSValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineOpcode {
    Move,
    Spill,
    Reload,
    Call,
    Guard,
    Opaque(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineInst {
    pub opcode: MachineOpcode,
    pub defs: Vec<MachineReg>,
    pub uses: Vec<MachineOperand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineTerminator {
    Jump {
        target: MachineBlockId,
    },
    Branch {
        then_bb: MachineBlockId,
        else_bb: MachineBlockId,
    },
    Return {
        value: Option<MachineOperand>,
    },
    Trap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineBlock {
    pub id: MachineBlockId,
    pub instructions: Vec<MachineInst>,
    pub terminator: MachineTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineFunction {
    pub blocks: Vec<MachineBlock>,
    pub entry: MachineBlockId,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitLowering;

impl JitLowering {
    pub fn lower(&self, ir: &IRFunction) -> MachineFunction {
        MachineFunction {
            blocks: ir
                .blocks
                .iter()
                .map(|block| MachineBlock {
                    id: block.id,
                    instructions: Vec::new(),
                    terminator: lower_terminator(&block.terminator),
                })
                .collect(),
            entry: ir.entry,
        }
    }
}

fn lower_terminator(terminator: &IRTerminator) -> MachineTerminator {
    match terminator {
        IRTerminator::Jump { target } => MachineTerminator::Jump { target: *target },
        IRTerminator::Branch {
            target,
            fallthrough,
            ..
        } => MachineTerminator::Branch {
            then_bb: *target,
            else_bb: *fallthrough,
        },
        IRTerminator::ConditionalReturn { value, .. } => MachineTerminator::Return {
            value: Some(MachineOperand::Value(value.clone())),
        },
        IRTerminator::Return { value } => MachineTerminator::Return {
            value: value.clone().map(MachineOperand::Value),
        },
        IRTerminator::None
        | IRTerminator::Switch { .. }
        | IRTerminator::Try { .. }
        | IRTerminator::Throw { .. }
        | IRTerminator::TailCall { .. }
        | IRTerminator::CallReturn { .. } => MachineTerminator::Trap,
    }
}
