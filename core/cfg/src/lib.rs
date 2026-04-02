mod builder;
mod decode;

use std::collections::HashMap;
use std::fmt;

use codegen::{CompiledBytecode, Opcode};
pub use decode::{
    ACC_REG, DecodedInst, DecodedSwitchCase, DecodedSwitchTable, decode_branch_target,
    decode_switch_table, decode_word,
};
use value::{JSValue, to_f64};

pub use builder::build_cfg;

pub type BlockId = usize;
pub type InstIdx = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareKind {
    Eq,
    Neq,
    Lt,
    Lte,
    LteFalse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Truthy {
        reg: u8,
        negate: bool,
    },
    Compare {
        kind: CompareKind,
        lhs: u8,
        rhs: u8,
        negate: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchCase {
    pub value: JSValue,
    pub target: BlockId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminator {
    None,
    Fallthrough {
        target: BlockId,
    },
    Jump {
        target: BlockId,
    },
    Branch {
        condition: Condition,
        target: BlockId,
        fallthrough: BlockId,
    },
    Switch {
        key: u8,
        cases: Vec<SwitchCase>,
        default_target: BlockId,
    },
    Try {
        handler: BlockId,
        fallthrough: BlockId,
    },
    ConditionalReturn {
        condition: Condition,
        value: u8,
        fallthrough: BlockId,
    },
    Return {
        value: Option<u8>,
    },
    Throw {
        value: u8,
    },
    TailCall {
        callee: u8,
        argc: u8,
    },
    CallReturn {
        callee: u8,
        argc: u8,
    },
}

impl Terminator {
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Self::None
            | Self::Return { .. }
            | Self::Throw { .. }
            | Self::TailCall { .. }
            | Self::CallReturn { .. } => Vec::new(),
            Self::Fallthrough { target } | Self::Jump { target } => vec![*target],
            Self::Branch {
                target,
                fallthrough,
                ..
            }
            | Self::Try {
                handler: target,
                fallthrough,
            } => vec![*target, *fallthrough],
            Self::Switch {
                cases,
                default_target,
                ..
            } => {
                let mut successors = vec![*default_target];
                successors.extend(cases.iter().map(|case| case.target));
                successors.sort_unstable();
                successors.dedup();
                successors
            }
            Self::ConditionalReturn { fallthrough, .. } => vec![*fallthrough],
        }
    }
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub start_pc: InstIdx,
    pub end_pc: InstIdx,
    pub instructions: Vec<DecodedInst>,
    pub predecessors: Vec<BlockId>,
    pub successors: Vec<BlockId>,
    pub terminator: Terminator,
    pub is_loop_header: bool,
    pub is_loop_latch: bool,
}

#[derive(Debug, Clone)]
pub struct CFG {
    pub blocks: Vec<BasicBlock>,
    pub entry: BlockId,
    pub exit_blocks: Vec<BlockId>,
    pub entry_pc: InstIdx,
    pub bytecode: Vec<u32>,
    pub constants: Vec<JSValue>,
    pc_to_block: HashMap<InstIdx, BlockId>,
}

impl CFG {
    pub fn from_compiled(compiled: &CompiledBytecode) -> Result<Self, CFGError> {
        Self::from_entry(compiled, 0)
    }

    pub fn from_entry(compiled: &CompiledBytecode, entry_pc: InstIdx) -> Result<Self, CFGError> {
        Self::from_parts(
            compiled.bytecode.clone(),
            compiled.constants.clone(),
            entry_pc,
        )
    }

    pub fn from_parts(
        bytecode: Vec<u32>,
        constants: Vec<JSValue>,
        entry_pc: InstIdx,
    ) -> Result<Self, CFGError> {
        build_cfg(bytecode, constants, entry_pc)
    }

    pub fn block_for_pc(&self, pc: InstIdx) -> Option<BlockId> {
        self.pc_to_block.get(&pc).copied()
    }

    pub fn block_containing_pc(&self, pc: InstIdx) -> Option<BlockId> {
        self.block_for_pc(pc)
    }

    pub fn function_entries(compiled: &CompiledBytecode) -> Vec<InstIdx> {
        let mut entries = vec![0];
        entries.extend(compiled.function_constants.iter().filter_map(|index| {
            compiled
                .constants
                .get(*index as usize)
                .and_then(|value| to_f64(*value))
                .and_then(|value| {
                    if value.is_finite() && value >= 0.0 {
                        Some(value as usize)
                    } else {
                        None
                    }
                })
        }));
        entries.sort_unstable();
        entries.dedup();
        entries
    }

    pub(crate) fn with_block_map(mut self, pc_to_block: HashMap<InstIdx, BlockId>) -> Self {
        self.pc_to_block = pc_to_block;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CFGError {
    InvalidEntry {
        entry_pc: InstIdx,
        len: usize,
    },
    InvalidBranchTarget {
        pc: InstIdx,
        target_pc: InstIdx,
        len: usize,
    },
    InvalidSwitchTable {
        pc: InstIdx,
        table_index: usize,
    },
    MissingBlockForPc {
        pc: InstIdx,
    },
}

impl fmt::Display for CFGError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEntry { entry_pc, len } => {
                write!(
                    f,
                    "invalid CFG entry pc {entry_pc} for bytecode length {len}"
                )
            }
            Self::InvalidBranchTarget { pc, target_pc, len } => {
                write!(
                    f,
                    "invalid branch target {target_pc} decoded at pc {pc} for bytecode length {len}"
                )
            }
            Self::InvalidSwitchTable { pc, table_index } => {
                write!(
                    f,
                    "invalid switch table {table_index} referenced at pc {pc}"
                )
            }
            Self::MissingBlockForPc { pc } => write!(f, "missing basic block for pc {pc}"),
        }
    }
}

impl std::error::Error for CFGError {}

pub(crate) fn branch_target_or_error(inst: &DecodedInst, len: usize) -> Result<usize, CFGError> {
    let Some(target_pc) = decode_branch_target(inst) else {
        return Err(CFGError::InvalidBranchTarget {
            pc: inst.pc,
            target_pc: len,
            len,
        });
    };
    if target_pc >= len {
        return Err(CFGError::InvalidBranchTarget {
            pc: inst.pc,
            target_pc,
            len,
        });
    }
    Ok(target_pc)
}

pub(crate) fn next_pc(inst: &DecodedInst, len: usize) -> Result<usize, CFGError> {
    let next = inst.pc + 1;
    if next >= len {
        return Err(CFGError::InvalidBranchTarget {
            pc: inst.pc,
            target_pc: next,
            len,
        });
    }
    Ok(next)
}

pub(crate) fn is_conditional_branch(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::JmpTrue
            | Opcode::JmpFalse
            | Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLtF64
            | Opcode::JmpLte
            | Opcode::JmpLteF64
            | Opcode::JmpLteFalse
            | Opcode::JmpLteFalseF64
            | Opcode::LoopIncJmp
            | Opcode::IncJmpFalseLoop
            | Opcode::EqJmpTrue
            | Opcode::LtJmp
            | Opcode::EqJmpFalse
            | Opcode::TestJmpTrue
            | Opcode::LteJmpLoop
            | Opcode::RetIfLteI
            | Opcode::JmpI32Fast
            | Opcode::CmpJmp
            | Opcode::LoadJfalse
            | Opcode::LoadCmpEqJfalse
            | Opcode::LoadCmpLtJfalse
    )
}
