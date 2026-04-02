use std::collections::{HashMap, HashSet};
use std::fmt;

use cfg::{ACC_REG, BlockId, CompareKind, Condition, Terminator};
use codegen::Opcode;
use value::{
    JSValue, make_false, make_int32, make_null, make_number, make_true, make_undefined, to_f64,
};

use crate::builder::{PhiNode, SSAForm, SSAInstr, SSAValue};

const REG_SLOTS: usize = u8::MAX as usize + 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IRValue {
    Register(u8, usize),
    Constant(JSValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IRCondition {
    Truthy {
        value: IRValue,
        negate: bool,
    },
    Compare {
        kind: CompareKind,
        lhs: IRValue,
        rhs: IRValue,
        negate: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IRUnaryOp {
    Typeof,
    ToNum,
    ToStr,
    IsUndef,
    IsNull,
    Neg,
    Inc,
    Dec,
    ToPrimitive,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IRBinaryOp {
    Add,
    Sub,
    Mul,
    Eq,
    Lt,
    Lte,
    StrictEq,
    StrictNeq,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Ushr,
    Pow,
    LogicalAnd,
    LogicalOr,
    NullishCoalesce,
    In,
    Instanceof,
    AddStr,
    Mod,
    Div,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IRInst {
    Phi {
        dst: IRValue,
        incoming: Vec<(BlockId, IRValue)>,
    },
    Mov {
        dst: IRValue,
        src: IRValue,
    },
    LoadConst {
        dst: IRValue,
        value: JSValue,
    },
    Unary {
        dst: IRValue,
        op: IRUnaryOp,
        operand: IRValue,
    },
    Binary {
        dst: IRValue,
        op: IRBinaryOp,
        lhs: IRValue,
        rhs: IRValue,
    },
    Bytecode {
        inst: cfg::DecodedInst,
        uses: Vec<IRValue>,
        defs: Vec<IRValue>,
    },
    Nop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IRTerminator {
    None,
    Jump {
        target: BlockId,
    },
    Branch {
        condition: IRCondition,
        target: BlockId,
        fallthrough: BlockId,
    },
    Switch {
        key: IRValue,
        cases: Vec<(JSValue, BlockId)>,
        default_target: BlockId,
    },
    Try {
        handler: BlockId,
        fallthrough: BlockId,
    },
    ConditionalReturn {
        condition: IRCondition,
        value: IRValue,
        fallthrough: BlockId,
    },
    Return {
        value: Option<IRValue>,
    },
    Throw {
        value: IRValue,
    },
    TailCall {
        callee: IRValue,
        argc: u8,
    },
    CallReturn {
        callee: IRValue,
        argc: u8,
    },
}

#[derive(Debug, Clone)]
pub struct IRBlock {
    pub id: BlockId,
    pub instructions: Vec<IRInst>,
    pub terminator: IRTerminator,
    pub successors: Vec<BlockId>,
    pub predecessors: Vec<BlockId>,
}

#[derive(Debug, Clone)]
pub struct IRFunction {
    pub blocks: Vec<IRBlock>,
    pub entry: BlockId,
    pub exit_blocks: Vec<BlockId>,
    pub constants: Vec<JSValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BytecodeLoweringError {
    MissingBlock {
        block: BlockId,
    },
    UnsupportedTerminator {
        kind: &'static str,
    },
    UnsupportedValue {
        kind: &'static str,
    },
    RegisterPressure {
        required: usize,
        available: usize,
    },
    JumpOutOfRange {
        opcode: Opcode,
        from_pc: usize,
        to_pc: usize,
    },
    SwitchTableTooLarge {
        index: usize,
    },
}

impl fmt::Display for BytecodeLoweringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBlock { block } => write!(f, "missing IR block {block}"),
            Self::UnsupportedTerminator { kind } => {
                write!(f, "cannot lower IR terminator `{kind}` to bytecode")
            }
            Self::UnsupportedValue { kind } => {
                write!(f, "cannot lower IR value for `{kind}` to bytecode")
            }
            Self::RegisterPressure {
                required,
                available,
            } => write!(
                f,
                "bytecode lowering needs {required} temporary register(s) but only {available} are free"
            ),
            Self::JumpOutOfRange {
                opcode,
                from_pc,
                to_pc,
            } => write!(
                f,
                "jump for opcode {:?} from pc {from_pc} to pc {to_pc} does not fit the bytecode encoding",
                opcode
            ),
            Self::SwitchTableTooLarge { index } => {
                write!(f, "switch table index {index} does not fit in u8")
            }
        }
    }
}

impl std::error::Error for BytecodeLoweringError {}

impl SSAForm {
    pub fn to_ir(&self) -> IRFunction {
        let blocks = self
            .blocks
            .iter()
            .map(|block| IRBlock {
                id: block.id,
                instructions: lower_block_instructions(
                    block.phi_nodes.as_slice(),
                    &block.instructions,
                    self,
                ),
                terminator: lower_terminator(block.id, self),
                successors: block.successors.clone(),
                predecessors: block.predecessors.clone(),
            })
            .collect::<Vec<_>>();

        IRFunction {
            blocks,
            entry: self.entry,
            exit_blocks: self.exit_blocks.clone(),
            constants: self.cfg.constants.clone(),
        }
    }

    pub fn to_bytecodes(&self) -> Result<(Vec<u32>, Vec<JSValue>), BytecodeLoweringError> {
        self.to_ir().into_bytecodes()
    }

    pub fn into_bytecodes(self) -> Result<(Vec<u32>, Vec<JSValue>), BytecodeLoweringError> {
        self.to_bytecodes()
    }
}

impl IRFunction {
    pub fn to_bytecodes(&self) -> Result<(Vec<u32>, Vec<JSValue>), BytecodeLoweringError> {
        self.clone().into_bytecodes()
    }

    pub fn into_bytecodes(self) -> Result<(Vec<u32>, Vec<JSValue>), BytecodeLoweringError> {
        lower_ir_to_bytecode(self)
    }
}

fn lower_block_instructions(
    phi_nodes: &[PhiNode],
    instructions: &[SSAInstr],
    ssa: &SSAForm,
) -> Vec<IRInst> {
    let mut lowered = Vec::new();

    for phi in phi_nodes {
        lowered.push(IRInst::Phi {
            dst: IRValue::Register(phi.target_reg, phi.target_version),
            incoming: phi
                .incoming
                .iter()
                .map(|(block, (reg, version))| (*block, IRValue::Register(*reg, *version)))
                .collect(),
        });
    }

    for instruction in instructions {
        lowered.extend(lower_instruction(instruction, ssa));
    }

    lowered
}

fn lower_instruction(instruction: &SSAInstr, ssa: &SSAForm) -> Vec<IRInst> {
    let SSAInstr::Original {
        inst,
        uses,
        defined,
    } = instruction;

    if is_terminator_opcode(inst.opcode) {
        return Vec::new();
    }

    match inst.opcode {
        codegen::Opcode::Mov => lower_primary_with_copies(defined, |dst| IRInst::Mov {
            dst,
            src: use_value(uses, 0, inst.b),
        }),
        codegen::Opcode::LoadK => lower_primary_with_copies(defined, |dst| IRInst::LoadConst {
            dst,
            value: ssa
                .cfg
                .constants
                .get(inst.bx as usize)
                .copied()
                .unwrap_or_else(make_undefined),
        }),
        codegen::Opcode::LoadI => lower_primary_with_copies(defined, |dst| IRInst::LoadConst {
            dst,
            value: make_int32(inst.sbx as i32),
        }),
        codegen::Opcode::LoadNull => lower_primary_with_copies(defined, |dst| IRInst::LoadConst {
            dst,
            value: make_null(),
        }),
        codegen::Opcode::LoadTrue => lower_primary_with_copies(defined, |dst| IRInst::LoadConst {
            dst,
            value: make_true(),
        }),
        codegen::Opcode::LoadFalse => lower_primary_with_copies(defined, |dst| IRInst::LoadConst {
            dst,
            value: make_false(),
        }),
        codegen::Opcode::Load0 => lower_primary_with_copies(defined, |dst| IRInst::LoadConst {
            dst,
            value: make_int32(0),
        }),
        codegen::Opcode::Load1 => lower_primary_with_copies(defined, |dst| IRInst::LoadConst {
            dst,
            value: make_int32(1),
        }),
        codegen::Opcode::LoadAcc | codegen::Opcode::LoadThis => {
            lower_primary_with_copies(defined, |dst| IRInst::Mov {
                dst,
                src: use_value(uses, 0, inst.a),
            })
        }
        codegen::Opcode::Typeof => {
            lower_unary_with_copies(defined, IRUnaryOp::Typeof, uses, inst.b)
        }
        codegen::Opcode::ToNum => lower_unary_with_copies(defined, IRUnaryOp::ToNum, uses, inst.b),
        codegen::Opcode::ToStr => lower_unary_with_copies(defined, IRUnaryOp::ToStr, uses, inst.b),
        codegen::Opcode::IsUndef => {
            lower_unary_with_copies(defined, IRUnaryOp::IsUndef, uses, inst.b)
        }
        codegen::Opcode::IsNull => {
            lower_unary_with_copies(defined, IRUnaryOp::IsNull, uses, inst.b)
        }
        codegen::Opcode::Neg => lower_unary_with_copies(defined, IRUnaryOp::Neg, uses, inst.b),
        codegen::Opcode::Inc => lower_unary_with_copies(defined, IRUnaryOp::Inc, uses, inst.b),
        codegen::Opcode::Dec => lower_unary_with_copies(defined, IRUnaryOp::Dec, uses, inst.b),
        codegen::Opcode::ToPrimitive => {
            lower_unary_with_copies(defined, IRUnaryOp::ToPrimitive, uses, inst.b)
        }
        codegen::Opcode::BitNot => {
            lower_unary_with_copies(defined, IRUnaryOp::BitNot, uses, inst.b)
        }
        codegen::Opcode::Add | codegen::Opcode::AddI32 | codegen::Opcode::AddF64 => {
            lower_binary_with_copies(defined, IRBinaryOp::Add, uses, inst.b, inst.c)
        }
        codegen::Opcode::AddAcc => {
            lower_binary_with_copies(defined, IRBinaryOp::Add, uses, ACC_REG, inst.b)
        }
        codegen::Opcode::SubAcc => {
            lower_binary_with_copies(defined, IRBinaryOp::Sub, uses, ACC_REG, inst.b)
        }
        codegen::Opcode::MulAcc => {
            lower_binary_with_copies(defined, IRBinaryOp::Mul, uses, ACC_REG, inst.b)
        }
        codegen::Opcode::DivAcc => {
            lower_binary_with_copies(defined, IRBinaryOp::Div, uses, ACC_REG, inst.b)
        }
        codegen::Opcode::SubI32 | codegen::Opcode::SubF64 => {
            lower_binary_with_copies(defined, IRBinaryOp::Sub, uses, inst.b, inst.c)
        }
        codegen::Opcode::MulI32 | codegen::Opcode::MulF64 => {
            lower_binary_with_copies(defined, IRBinaryOp::Mul, uses, inst.b, inst.c)
        }
        codegen::Opcode::Eq | codegen::Opcode::EqI32Fast => {
            lower_binary_with_copies(defined, IRBinaryOp::Eq, uses, inst.b, inst.c)
        }
        codegen::Opcode::Lt | codegen::Opcode::LtI32Fast | codegen::Opcode::LtF64 => {
            lower_binary_with_copies(defined, IRBinaryOp::Lt, uses, inst.b, inst.c)
        }
        codegen::Opcode::Lte | codegen::Opcode::LteF64 => {
            lower_binary_with_copies(defined, IRBinaryOp::Lte, uses, inst.b, inst.c)
        }
        codegen::Opcode::StrictEq => {
            lower_binary_with_copies(defined, IRBinaryOp::StrictEq, uses, inst.b, inst.c)
        }
        codegen::Opcode::StrictNeq => {
            lower_binary_with_copies(defined, IRBinaryOp::StrictNeq, uses, inst.b, inst.c)
        }
        codegen::Opcode::BitAnd => {
            lower_binary_with_copies(defined, IRBinaryOp::BitAnd, uses, inst.b, inst.c)
        }
        codegen::Opcode::BitOr => {
            lower_binary_with_copies(defined, IRBinaryOp::BitOr, uses, inst.b, inst.c)
        }
        codegen::Opcode::BitXor => {
            lower_binary_with_copies(defined, IRBinaryOp::BitXor, uses, inst.b, inst.c)
        }
        codegen::Opcode::Shl => {
            lower_binary_with_copies(defined, IRBinaryOp::Shl, uses, inst.b, inst.c)
        }
        codegen::Opcode::Shr => {
            lower_binary_with_copies(defined, IRBinaryOp::Shr, uses, inst.b, inst.c)
        }
        codegen::Opcode::Ushr => {
            lower_binary_with_copies(defined, IRBinaryOp::Ushr, uses, inst.b, inst.c)
        }
        codegen::Opcode::Pow => {
            lower_binary_with_copies(defined, IRBinaryOp::Pow, uses, inst.b, inst.c)
        }
        codegen::Opcode::LogicalAnd => {
            lower_binary_with_copies(defined, IRBinaryOp::LogicalAnd, uses, inst.b, inst.c)
        }
        codegen::Opcode::LogicalOr => {
            lower_binary_with_copies(defined, IRBinaryOp::LogicalOr, uses, inst.b, inst.c)
        }
        codegen::Opcode::NullishCoalesce => {
            lower_binary_with_copies(defined, IRBinaryOp::NullishCoalesce, uses, inst.b, inst.c)
        }
        codegen::Opcode::In => {
            lower_binary_with_copies(defined, IRBinaryOp::In, uses, inst.b, inst.c)
        }
        codegen::Opcode::Instanceof => {
            lower_binary_with_copies(defined, IRBinaryOp::Instanceof, uses, inst.b, inst.c)
        }
        codegen::Opcode::AddStr => {
            lower_binary_with_copies(defined, IRBinaryOp::AddStr, uses, inst.b, inst.c)
        }
        codegen::Opcode::AddStrAcc => {
            lower_binary_with_copies(defined, IRBinaryOp::AddStr, uses, ACC_REG, inst.b)
        }
        codegen::Opcode::Mod => {
            lower_binary_with_copies(defined, IRBinaryOp::Mod, uses, inst.b, inst.c)
        }
        _ => vec![IRInst::Bytecode {
            inst: inst.clone(),
            uses: uses.iter().copied().map(value_for_def).collect(),
            defs: defined.iter().copied().map(value_for_def).collect(),
        }],
    }
}

fn is_terminator_opcode(opcode: codegen::Opcode) -> bool {
    matches!(
        opcode,
        codegen::Opcode::Jmp
            | codegen::Opcode::JmpTrue
            | codegen::Opcode::JmpFalse
            | codegen::Opcode::JmpEq
            | codegen::Opcode::JmpNeq
            | codegen::Opcode::JmpLt
            | codegen::Opcode::JmpLtF64
            | codegen::Opcode::JmpLte
            | codegen::Opcode::JmpLteF64
            | codegen::Opcode::LoopIncJmp
            | codegen::Opcode::Switch
            | codegen::Opcode::Ret
            | codegen::Opcode::RetU
            | codegen::Opcode::TailCall
            | codegen::Opcode::Throw
            | codegen::Opcode::Try
            | codegen::Opcode::IncJmpFalseLoop
            | codegen::Opcode::EqJmpTrue
            | codegen::Opcode::LtJmp
            | codegen::Opcode::EqJmpFalse
            | codegen::Opcode::IncAccJmp
            | codegen::Opcode::TestJmpTrue
            | codegen::Opcode::LteJmpLoop
            | codegen::Opcode::JmpLteFalse
            | codegen::Opcode::JmpLteFalseF64
            | codegen::Opcode::RetReg
            | codegen::Opcode::RetIfLteI
            | codegen::Opcode::CmpJmp
            | codegen::Opcode::CallRet
            | codegen::Opcode::JmpI32Fast
            | codegen::Opcode::LoadJfalse
            | codegen::Opcode::LoadCmpEqJfalse
            | codegen::Opcode::LoadCmpLtJfalse
    )
}

fn lower_terminator(block_id: BlockId, ssa: &SSAForm) -> IRTerminator {
    match &ssa.cfg.blocks[block_id].terminator {
        Terminator::None => IRTerminator::None,
        Terminator::Fallthrough { target } | Terminator::Jump { target } => {
            IRTerminator::Jump { target: *target }
        }
        Terminator::Branch {
            condition,
            target,
            fallthrough,
        } => IRTerminator::Branch {
            condition: lower_condition(*condition, block_id, ssa),
            target: *target,
            fallthrough: *fallthrough,
        },
        Terminator::Switch {
            key,
            cases,
            default_target,
        } => IRTerminator::Switch {
            key: IRValue::Register(*key, ssa.version_at_end(block_id, *key)),
            cases: cases.iter().map(|case| (case.value, case.target)).collect(),
            default_target: *default_target,
        },
        Terminator::Try {
            handler,
            fallthrough,
        } => IRTerminator::Try {
            handler: *handler,
            fallthrough: *fallthrough,
        },
        Terminator::ConditionalReturn {
            condition,
            value,
            fallthrough,
        } => IRTerminator::ConditionalReturn {
            condition: lower_condition(*condition, block_id, ssa),
            value: IRValue::Register(*value, ssa.version_at_end(block_id, *value)),
            fallthrough: *fallthrough,
        },
        Terminator::Return { value } => IRTerminator::Return {
            value: value.map(|reg| IRValue::Register(reg, ssa.version_at_end(block_id, reg))),
        },
        Terminator::Throw { value } => IRTerminator::Throw {
            value: IRValue::Register(*value, ssa.version_at_end(block_id, *value)),
        },
        Terminator::TailCall { callee, argc } => IRTerminator::TailCall {
            callee: IRValue::Register(*callee, ssa.version_at_end(block_id, *callee)),
            argc: *argc,
        },
        Terminator::CallReturn { callee, argc } => IRTerminator::CallReturn {
            callee: IRValue::Register(*callee, ssa.version_at_end(block_id, *callee)),
            argc: *argc,
        },
    }
}

fn lower_condition(condition: Condition, block_id: BlockId, ssa: &SSAForm) -> IRCondition {
    match condition {
        Condition::Truthy { reg, negate } => IRCondition::Truthy {
            value: IRValue::Register(reg, ssa.version_at_end(block_id, reg)),
            negate,
        },
        Condition::Compare {
            kind,
            lhs,
            rhs,
            negate,
        } => IRCondition::Compare {
            kind,
            lhs: IRValue::Register(lhs, ssa.version_at_end(block_id, lhs)),
            rhs: IRValue::Register(rhs, ssa.version_at_end(block_id, rhs)),
            negate,
        },
    }
}

fn primary_definition(defined: &[SSAValue]) -> Option<SSAValue> {
    defined.first().copied()
}

fn value_for_def((reg, version): SSAValue) -> IRValue {
    IRValue::Register(reg, version)
}

fn use_value(uses: &[SSAValue], index: usize, fallback_reg: u8) -> IRValue {
    uses.get(index)
        .copied()
        .map(value_for_def)
        .unwrap_or(IRValue::Register(fallback_reg, 0))
}

fn lower_primary_with_copies(
    defined: &[SSAValue],
    make_primary: impl FnOnce(IRValue) -> IRInst,
) -> Vec<IRInst> {
    let Some(primary) = primary_definition(defined) else {
        return Vec::new();
    };

    let primary = value_for_def(primary);
    let mut lowered = vec![make_primary(primary.clone())];
    for extra in defined.iter().skip(1).copied() {
        lowered.push(IRInst::Mov {
            dst: value_for_def(extra),
            src: primary.clone(),
        });
    }
    lowered
}

fn lower_unary_with_copies(
    defined: &[SSAValue],
    op: IRUnaryOp,
    uses: &[SSAValue],
    fallback_reg: u8,
) -> Vec<IRInst> {
    lower_primary_with_copies(defined, |dst| IRInst::Unary {
        dst,
        op,
        operand: use_value(uses, 0, fallback_reg),
    })
}

fn lower_binary_with_copies(
    defined: &[SSAValue],
    op: IRBinaryOp,
    uses: &[SSAValue],
    fallback_lhs: u8,
    fallback_rhs: u8,
) -> Vec<IRInst> {
    lower_primary_with_copies(defined, |dst| IRInst::Binary {
        dst,
        op,
        lhs: use_value(uses, 0, fallback_lhs),
        rhs: use_value(uses, 1, fallback_rhs),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Label {
    Block(BlockId),
    Stub(usize),
}

#[derive(Debug, Clone)]
enum LowerItem {
    Label(Label),
    Inst(PendingInst),
}

#[derive(Debug, Clone)]
enum PendingInst {
    Raw(u32),
    Jump {
        opcode: Opcode,
        a: u8,
        target: Label,
    },
    CompareJump {
        opcode: Opcode,
        lhs: u8,
        rhs: u8,
        target: Label,
    },
    Switch {
        key: u8,
        cases: Vec<(JSValue, Label)>,
        default_target: Label,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EdgeSource {
    Register(u8),
    Constant(JSValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EdgeAssignment {
    dst: u8,
    src: EdgeSource,
}

struct LoweringState {
    items: Vec<LowerItem>,
    constants: Vec<JSValue>,
    free_regs: Vec<u8>,
    next_stub: usize,
    edge_labels: HashMap<(BlockId, BlockId), Label>,
    pending_edge_stubs: Vec<(Label, Vec<EdgeAssignment>, BlockId)>,
    numeric_values: HashSet<IRValue>,
}

impl LoweringState {
    fn new(constants: Vec<JSValue>, free_regs: Vec<u8>, numeric_values: HashSet<IRValue>) -> Self {
        Self {
            items: Vec::new(),
            constants,
            free_regs,
            next_stub: 0,
            edge_labels: HashMap::new(),
            pending_edge_stubs: Vec::new(),
            numeric_values,
        }
    }

    fn fresh_stub(&mut self) -> Label {
        let label = Label::Stub(self.next_stub);
        self.next_stub += 1;
        label
    }

    fn temp(&self, reserved: &[u8], kind: &'static str) -> Result<u8, BytecodeLoweringError> {
        let _ = kind;
        self.free_regs
            .iter()
            .copied()
            .find(|reg| !reserved.contains(reg))
            .ok_or(BytecodeLoweringError::RegisterPressure {
                required: reserved.len() + 1,
                available: self.free_regs.len(),
            })
    }

    fn value_reg(
        &mut self,
        value: &IRValue,
        reserved: &[u8],
        kind: &'static str,
    ) -> Result<u8, BytecodeLoweringError> {
        match value {
            IRValue::Register(reg, _) => Ok(*reg),
            IRValue::Constant(value) => {
                let reg = self.temp(reserved, kind)?;
                emit_load_const(reg, *value, self);
                Ok(reg)
            }
        }
    }

    fn finish(self) -> Result<(Vec<u32>, Vec<JSValue>), BytecodeLoweringError> {
        let mut label_pcs = HashMap::<Label, usize>::new();
        let mut pc = 0usize;
        for item in &self.items {
            match item {
                LowerItem::Label(label) => {
                    label_pcs.insert(*label, pc);
                }
                LowerItem::Inst(_) => pc += 1,
            }
        }

        let mut bytecode = Vec::with_capacity(pc);
        let mut constants = self.constants;

        for item in self.items {
            let LowerItem::Inst(inst) = item else {
                continue;
            };

            let raw = match inst {
                PendingInst::Raw(raw) => raw,
                PendingInst::Jump { opcode, a, target } => {
                    let target_pc = label_pcs.get(&target).copied().ok_or(
                        BytecodeLoweringError::MissingBlock {
                            block: label_block_id(target),
                        },
                    )?;
                    encode_targeted_jump(opcode, a, bytecode.len(), target_pc)?
                }
                PendingInst::CompareJump {
                    opcode,
                    lhs,
                    rhs,
                    target,
                } => {
                    let target_pc = label_pcs.get(&target).copied().ok_or(
                        BytecodeLoweringError::MissingBlock {
                            block: label_block_id(target),
                        },
                    )?;
                    encode_compare_jump(opcode, lhs, rhs, bytecode.len(), target_pc)?
                }
                PendingInst::Switch {
                    key,
                    cases,
                    default_target,
                } => {
                    let table = add_switch_table(
                        &mut constants,
                        bytecode.len(),
                        &label_pcs,
                        default_target,
                        &cases,
                    )?;
                    encode_raw(Opcode::Switch, key, table, 0)
                }
            };

            bytecode.push(raw);
        }

        Ok((bytecode, constants))
    }

    fn enqueue_edge_stub(
        &mut self,
        label: Label,
        assignments: Vec<EdgeAssignment>,
        target: BlockId,
    ) {
        self.pending_edge_stubs.push((label, assignments, target));
    }

    fn emit_pending_edge_stubs(&mut self) -> Result<(), BytecodeLoweringError> {
        let pending = std::mem::take(&mut self.pending_edge_stubs);
        for (label, assignments, target) in pending {
            self.items.push(LowerItem::Label(label));
            emit_parallel_copies(assignments, self)?;
            self.items.push(LowerItem::Inst(PendingInst::Jump {
                opcode: Opcode::Jmp,
                a: 0,
                target: Label::Block(target),
            }));
        }
        Ok(())
    }
}

fn lower_ir_to_bytecode(ir: IRFunction) -> Result<(Vec<u32>, Vec<JSValue>), BytecodeLoweringError> {
    let edge_copies = collect_edge_copies(&ir.blocks);
    let numeric_values = infer_numeric_values(&ir);
    let mut lowering =
        LoweringState::new(ir.constants, collect_free_regs(&ir.blocks), numeric_values);

    for block in &ir.blocks {
        lowering
            .items
            .push(LowerItem::Label(Label::Block(block.id)));
        for inst in &block.instructions {
            lower_ir_instruction(inst, &mut lowering)?;
        }
        lower_ir_terminator(block, &edge_copies, &mut lowering)?;
    }

    lowering.emit_pending_edge_stubs()?;
    lowering.finish()
}

fn lower_ir_instruction(
    inst: &IRInst,
    lowering: &mut LoweringState,
) -> Result<(), BytecodeLoweringError> {
    match inst {
        IRInst::Phi { .. } | IRInst::Nop => Ok(()),
        IRInst::Mov { dst, src } => {
            let dst = register_dst(dst, "mov")?;
            emit_move_or_load(dst, src, lowering);
            Ok(())
        }
        IRInst::LoadConst { dst, value } => {
            let dst = register_dst(dst, "load_const")?;
            emit_load_const(dst, *value, lowering);
            Ok(())
        }
        IRInst::Unary { dst, op, operand } => lower_unary_op(dst, *op, operand, lowering),
        IRInst::Binary { dst, op, lhs, rhs } => lower_binary_inst(dst, *op, lhs, rhs, lowering),
        IRInst::Bytecode { inst, .. } => {
            lowering
                .items
                .push(LowerItem::Inst(PendingInst::Raw(inst.raw)));
            Ok(())
        }
    }
}

fn lower_binary_op(
    dst: &IRValue,
    lhs: &IRValue,
    rhs: &IRValue,
    opcode: Opcode,
    lowering: &mut LoweringState,
) -> Result<(), BytecodeLoweringError> {
    let dst = register_dst(dst, "binary_op")?;
    let lhs_reg = lowering.value_reg(lhs, &[], "binary_lhs")?;
    let rhs_reg = lowering.value_reg(rhs, &[lhs_reg], "binary_rhs")?;

    if matches!(opcode, Opcode::AddF64 | Opcode::SubF64 | Opcode::MulF64) {
        lowering
            .items
            .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                opcode, dst, lhs_reg, rhs_reg,
            ))));
        return Ok(());
    }

    lowering
        .items
        .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
            Opcode::LoadAcc,
            lhs_reg,
            0,
            0,
        ))));
    lowering
        .items
        .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
            opcode, 0, rhs_reg, 0,
        ))));
    if dst != ACC_REG {
        lowering
            .items
            .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                Opcode::Mov,
                dst,
                ACC_REG,
                0,
            ))));
    }
    Ok(())
}

fn lower_unary_op(
    dst: &IRValue,
    op: IRUnaryOp,
    operand: &IRValue,
    lowering: &mut LoweringState,
) -> Result<(), BytecodeLoweringError> {
    let dst = register_dst(dst, "unary_op")?;
    let operand = lowering.value_reg(operand, &[], "unary_operand")?;
    let opcode = unary_opcode(op);

    if unary_writes_to_acc(op) {
        lowering
            .items
            .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                opcode, 0, operand, 0,
            ))));
        if dst != ACC_REG {
            lowering
                .items
                .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                    Opcode::Mov,
                    dst,
                    ACC_REG,
                    0,
                ))));
        }
    } else {
        lowering
            .items
            .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                opcode, dst, operand, 0,
            ))));
    }

    Ok(())
}

fn lower_binary_inst(
    dst: &IRValue,
    op: IRBinaryOp,
    lhs: &IRValue,
    rhs: &IRValue,
    lowering: &mut LoweringState,
) -> Result<(), BytecodeLoweringError> {
    let numeric = value_is_numeric(lhs, &lowering.numeric_values)
        && value_is_numeric(rhs, &lowering.numeric_values);

    match op {
        IRBinaryOp::Add => lower_binary_op(
            dst,
            lhs,
            rhs,
            if numeric {
                Opcode::AddF64
            } else {
                Opcode::AddAcc
            },
            lowering,
        ),
        IRBinaryOp::Sub => lower_binary_op(
            dst,
            lhs,
            rhs,
            if numeric {
                Opcode::SubF64
            } else {
                Opcode::SubAcc
            },
            lowering,
        ),
        IRBinaryOp::Mul => lower_binary_op(
            dst,
            lhs,
            rhs,
            if numeric {
                Opcode::MulF64
            } else {
                Opcode::MulAcc
            },
            lowering,
        ),
        IRBinaryOp::Div => lower_binary_op(dst, lhs, rhs, Opcode::DivAcc, lowering),
        _ => {
            let dst = register_dst(dst, "binary")?;
            let lhs = lowering.value_reg(lhs, &[], "binary_lhs")?;
            let rhs = lowering.value_reg(rhs, &[lhs], "binary_rhs")?;
            let opcode = binary_opcode(op, numeric);
            lowering
                .items
                .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                    opcode,
                    if matches!(opcode, Opcode::LtF64 | Opcode::LteF64) {
                        dst
                    } else {
                        0
                    },
                    lhs,
                    rhs,
                ))));
            if dst != ACC_REG && !matches!(opcode, Opcode::LtF64 | Opcode::LteF64) {
                lowering
                    .items
                    .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                        Opcode::Mov,
                        dst,
                        ACC_REG,
                        0,
                    ))));
            }
            Ok(())
        }
    }
}

fn lower_ir_terminator(
    block: &IRBlock,
    edge_copies: &HashMap<(BlockId, BlockId), Vec<EdgeAssignment>>,
    lowering: &mut LoweringState,
) -> Result<(), BytecodeLoweringError> {
    match &block.terminator {
        IRTerminator::None => Ok(()),
        IRTerminator::Jump { target } => {
            let label = edge_label_for(block.id, *target, edge_copies, lowering)?;
            lowering.items.push(LowerItem::Inst(PendingInst::Jump {
                opcode: Opcode::Jmp,
                a: 0,
                target: label,
            }));
            Ok(())
        }
        IRTerminator::Branch {
            condition,
            target,
            fallthrough,
        } => {
            let lowered = lower_branch(condition, *target, *fallthrough, lowering)?;
            let jump_label = edge_label_for(block.id, lowered.jump_target, edge_copies, lowering)?;
            let other_label =
                edge_label_for(block.id, lowered.other_target, edge_copies, lowering)?;
            emit_branch_jump(lowered.kind, jump_label, lowering);
            lowering.items.push(LowerItem::Inst(PendingInst::Jump {
                opcode: Opcode::Jmp,
                a: 0,
                target: other_label,
            }));
            Ok(())
        }
        IRTerminator::Switch {
            key,
            cases,
            default_target,
        } => {
            let key = lowering.value_reg(key, &[], "switch_key")?;
            let default_target = edge_label_for(block.id, *default_target, edge_copies, lowering)?;
            let mut lowered_cases = Vec::with_capacity(cases.len());
            for (value, target) in cases {
                lowered_cases.push((
                    *value,
                    edge_label_for(block.id, *target, edge_copies, lowering)?,
                ));
            }
            lowering.items.push(LowerItem::Inst(PendingInst::Switch {
                key,
                cases: lowered_cases,
                default_target,
            }));
            Ok(())
        }
        IRTerminator::Return { value } => {
            emit_return(value.as_ref(), lowering)?;
            Ok(())
        }
        IRTerminator::Throw { value } => {
            let reg = lowering.value_reg(value, &[], "throw")?;
            lowering
                .items
                .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                    Opcode::Throw,
                    reg,
                    0,
                    0,
                ))));
            Ok(())
        }
        IRTerminator::TailCall { callee, argc } => {
            let reg = lowering.value_reg(callee, &[], "tail_call")?;
            lowering
                .items
                .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                    Opcode::TailCall,
                    reg,
                    *argc,
                    0,
                ))));
            Ok(())
        }
        IRTerminator::CallReturn { callee, argc } => {
            let reg = lowering.value_reg(callee, &[], "call_return")?;
            lowering
                .items
                .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                    Opcode::CallRet,
                    reg,
                    *argc,
                    0,
                ))));
            Ok(())
        }
        IRTerminator::ConditionalReturn {
            condition,
            value,
            fallthrough,
        } => {
            if let IRCondition::Compare {
                kind: CompareKind::Lte,
                lhs,
                rhs,
                negate: false,
            } = condition
            {
                let lhs = lowering.value_reg(lhs, &[], "conditional_return_lhs")?;
                let rhs = lowering.value_reg(rhs, &[lhs], "conditional_return_rhs")?;
                let value = lowering.value_reg(value, &[lhs, rhs], "conditional_return_value")?;
                lowering
                    .items
                    .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                        Opcode::RetIfLteI,
                        lhs,
                        rhs,
                        value,
                    ))));
                return Ok(());
            }

            let fallthrough_label = edge_label_for(block.id, *fallthrough, edge_copies, lowering)?;
            let negated = negate_condition(condition);
            emit_condition_jump(&negated, fallthrough_label, lowering)?;
            emit_return(Some(value), lowering)
        }
        IRTerminator::Try { .. } => {
            Err(BytecodeLoweringError::UnsupportedTerminator { kind: "Try" })
        }
    }
}

struct LoweredBranch {
    kind: BranchKind,
    jump_target: BlockId,
    other_target: BlockId,
}

enum BranchKind {
    Truthy { reg: u8, opcode: Opcode },
    Compare { lhs: u8, rhs: u8, opcode: Opcode },
}

fn lower_branch(
    condition: &IRCondition,
    target: BlockId,
    fallthrough: BlockId,
    lowering: &mut LoweringState,
) -> Result<LoweredBranch, BytecodeLoweringError> {
    match condition {
        IRCondition::Truthy { value, negate } => Ok(LoweredBranch {
            kind: BranchKind::Truthy {
                reg: lowering.value_reg(value, &[], "branch_truthy")?,
                opcode: if *negate {
                    Opcode::JmpFalse
                } else {
                    Opcode::JmpTrue
                },
            },
            jump_target: target,
            other_target: fallthrough,
        }),
        IRCondition::Compare {
            kind,
            lhs,
            rhs,
            negate,
        } => {
            let numeric = value_is_numeric(lhs, &lowering.numeric_values)
                && value_is_numeric(rhs, &lowering.numeric_values);
            let lhs = lowering.value_reg(lhs, &[], "branch_lhs")?;
            let rhs = lowering.value_reg(rhs, &[lhs], "branch_rhs")?;
            let (opcode, jump_target, other_target) =
                compare_branch_opcode(*kind, *negate, numeric, target, fallthrough);
            Ok(LoweredBranch {
                kind: BranchKind::Compare { lhs, rhs, opcode },
                jump_target,
                other_target,
            })
        }
    }
}

fn emit_branch_jump(kind: BranchKind, target: Label, lowering: &mut LoweringState) {
    match kind {
        BranchKind::Truthy { reg, opcode } => {
            lowering.items.push(LowerItem::Inst(PendingInst::Jump {
                opcode,
                a: reg,
                target,
            }));
        }
        BranchKind::Compare { lhs, rhs, opcode } => {
            lowering
                .items
                .push(LowerItem::Inst(PendingInst::CompareJump {
                    opcode,
                    lhs,
                    rhs,
                    target,
                }));
        }
    }
}

fn emit_condition_jump(
    condition: &IRCondition,
    target: Label,
    lowering: &mut LoweringState,
) -> Result<(), BytecodeLoweringError> {
    match condition {
        IRCondition::Truthy { value, negate } => {
            let reg = lowering.value_reg(value, &[], "conditional_jump_truthy")?;
            lowering.items.push(LowerItem::Inst(PendingInst::Jump {
                opcode: if *negate {
                    Opcode::JmpFalse
                } else {
                    Opcode::JmpTrue
                },
                a: reg,
                target,
            }));
        }
        IRCondition::Compare {
            kind,
            lhs,
            rhs,
            negate,
        } => {
            let numeric = value_is_numeric(lhs, &lowering.numeric_values)
                && value_is_numeric(rhs, &lowering.numeric_values);
            let lhs = lowering.value_reg(lhs, &[], "conditional_jump_lhs")?;
            let rhs = lowering.value_reg(rhs, &[lhs], "conditional_jump_rhs")?;
            let opcode = compare_branch_opcode(*kind, *negate, numeric, 0, 0).0;
            lowering
                .items
                .push(LowerItem::Inst(PendingInst::CompareJump {
                    opcode,
                    lhs,
                    rhs,
                    target,
                }));
        }
    }

    Ok(())
}

fn negate_condition(condition: &IRCondition) -> IRCondition {
    match condition {
        IRCondition::Truthy { value, negate } => IRCondition::Truthy {
            value: value.clone(),
            negate: !*negate,
        },
        IRCondition::Compare {
            kind,
            lhs,
            rhs,
            negate,
        } => IRCondition::Compare {
            kind: *kind,
            lhs: lhs.clone(),
            rhs: rhs.clone(),
            negate: !*negate,
        },
    }
}

fn emit_return(
    value: Option<&IRValue>,
    lowering: &mut LoweringState,
) -> Result<(), BytecodeLoweringError> {
    if let Some(value) = value {
        let reg = lowering.value_reg(value, &[], "return")?;
        lowering
            .items
            .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                Opcode::RetReg,
                reg,
                0,
                0,
            ))));
    } else {
        lowering
            .items
            .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                Opcode::RetU,
                0,
                0,
                0,
            ))));
    }
    Ok(())
}

fn edge_label_for(
    from: BlockId,
    to: BlockId,
    edge_copies: &HashMap<(BlockId, BlockId), Vec<EdgeAssignment>>,
    lowering: &mut LoweringState,
) -> Result<Label, BytecodeLoweringError> {
    let assignments = edge_copies
        .get(&(from, to))
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|assignment| match assignment.src {
            EdgeSource::Register(src) => src != assignment.dst,
            EdgeSource::Constant(_) => true,
        })
        .collect::<Vec<_>>();
    if assignments.is_empty() {
        return Ok(Label::Block(to));
    }
    if let Some(label) = lowering.edge_labels.get(&(from, to)).copied() {
        return Ok(label);
    }

    let label = lowering.fresh_stub();
    lowering.edge_labels.insert((from, to), label);
    lowering.enqueue_edge_stub(label, assignments, to);
    Ok(label)
}

fn emit_parallel_copies(
    mut assignments: Vec<EdgeAssignment>,
    lowering: &mut LoweringState,
) -> Result<(), BytecodeLoweringError> {
    assignments.retain(|assignment| match assignment.src {
        EdgeSource::Register(src) => src != assignment.dst,
        EdgeSource::Constant(_) => true,
    });

    while !assignments.is_empty() {
        let mut sources = Vec::new();
        for assignment in &assignments {
            if let EdgeSource::Register(src) = assignment.src {
                if src != assignment.dst && !sources.contains(&src) {
                    sources.push(src);
                }
            }
        }

        if let Some(index) = assignments
            .iter()
            .position(|assignment| !sources.contains(&assignment.dst))
        {
            let assignment = assignments.remove(index);
            match assignment.src {
                EdgeSource::Register(src) => {
                    lowering
                        .items
                        .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                            Opcode::Mov,
                            assignment.dst,
                            src,
                            0,
                        ))))
                }
                EdgeSource::Constant(value) => emit_load_const(assignment.dst, value, lowering),
            }
            continue;
        }

        let scratch = lowering.temp(&[], "phi_cycle")?;
        let EdgeSource::Register(src) = assignments[0].src.clone() else {
            return Err(BytecodeLoweringError::UnsupportedValue { kind: "phi_cycle" });
        };
        lowering
            .items
            .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                Opcode::Mov,
                scratch,
                src,
                0,
            ))));
        assignments[0].src = EdgeSource::Register(scratch);
    }

    Ok(())
}

fn collect_edge_copies(blocks: &[IRBlock]) -> HashMap<(BlockId, BlockId), Vec<EdgeAssignment>> {
    let mut edge_copies = HashMap::<(BlockId, BlockId), Vec<EdgeAssignment>>::new();
    for block in blocks {
        for inst in &block.instructions {
            let IRInst::Phi { dst, incoming } = inst else {
                continue;
            };
            let Ok(dst) = register_dst(dst, "phi") else {
                continue;
            };
            for (pred, value) in incoming {
                edge_copies
                    .entry((*pred, block.id))
                    .or_default()
                    .push(EdgeAssignment {
                        dst,
                        src: match value {
                            IRValue::Register(reg, _) => EdgeSource::Register(*reg),
                            IRValue::Constant(value) => EdgeSource::Constant(*value),
                        },
                    });
            }
        }
    }
    edge_copies
}

fn collect_free_regs(blocks: &[IRBlock]) -> Vec<u8> {
    let mut used = [false; REG_SLOTS];
    for block in blocks {
        for inst in &block.instructions {
            mark_instruction_regs(inst, &mut used);
        }
        mark_terminator_regs(&block.terminator, &mut used);
    }
    (0..=u8::MAX)
        .filter(|&reg| !used[reg as usize])
        .collect::<Vec<_>>()
}

fn infer_numeric_values(ir: &IRFunction) -> HashSet<IRValue> {
    let mut numeric = HashSet::new();

    loop {
        let mut changed = false;

        for block in &ir.blocks {
            for inst in &block.instructions {
                let Some(dst) = defined_value(inst) else {
                    continue;
                };

                let is_numeric = match inst {
                    IRInst::Phi { incoming, .. } => {
                        !incoming.is_empty()
                            && incoming
                                .iter()
                                .all(|(_, value)| value_is_numeric(value, &numeric))
                    }
                    IRInst::Mov { src, .. } => value_is_numeric(src, &numeric),
                    IRInst::LoadConst { value, .. } => to_f64(*value).is_some(),
                    IRInst::Unary { op, .. } => matches!(
                        op,
                        IRUnaryOp::ToNum
                            | IRUnaryOp::Neg
                            | IRUnaryOp::Inc
                            | IRUnaryOp::Dec
                            | IRUnaryOp::BitNot
                    ),
                    IRInst::Binary { op, lhs, rhs, .. } => match op {
                        IRBinaryOp::Add => {
                            value_is_numeric(lhs, &numeric) && value_is_numeric(rhs, &numeric)
                        }
                        IRBinaryOp::Sub
                        | IRBinaryOp::Mul
                        | IRBinaryOp::Div
                        | IRBinaryOp::Mod
                        | IRBinaryOp::Pow
                        | IRBinaryOp::BitAnd
                        | IRBinaryOp::BitOr
                        | IRBinaryOp::BitXor
                        | IRBinaryOp::Shl
                        | IRBinaryOp::Shr
                        | IRBinaryOp::Ushr => true,
                        IRBinaryOp::Eq
                        | IRBinaryOp::Lt
                        | IRBinaryOp::Lte
                        | IRBinaryOp::StrictEq
                        | IRBinaryOp::StrictNeq
                        | IRBinaryOp::LogicalAnd
                        | IRBinaryOp::LogicalOr
                        | IRBinaryOp::NullishCoalesce
                        | IRBinaryOp::In
                        | IRBinaryOp::Instanceof
                        | IRBinaryOp::AddStr => false,
                    },
                    IRInst::Bytecode { inst, .. } => matches!(
                        inst.opcode,
                        Opcode::LoadI
                            | Opcode::AddI32
                            | Opcode::AddF64
                            | Opcode::SubI32
                            | Opcode::SubF64
                            | Opcode::MulI32
                            | Opcode::MulF64
                            | Opcode::AddI32Fast
                            | Opcode::AddF64Fast
                            | Opcode::SubI32Fast
                            | Opcode::MulI32Fast
                    ),
                    IRInst::Nop => false,
                };

                if is_numeric && numeric.insert(dst) {
                    changed = true;
                }
            }
        }

        if !changed {
            break;
        }
    }

    numeric
}

fn value_is_numeric(value: &IRValue, numeric: &HashSet<IRValue>) -> bool {
    match value {
        IRValue::Constant(value) => to_f64(*value).is_some(),
        IRValue::Register(_, _) => numeric.contains(value),
    }
}

fn defined_value(inst: &IRInst) -> Option<IRValue> {
    match inst {
        IRInst::Phi { dst, .. }
        | IRInst::Mov { dst, .. }
        | IRInst::LoadConst { dst, .. }
        | IRInst::Unary { dst, .. }
        | IRInst::Binary { dst, .. } => Some(dst.clone()),
        IRInst::Bytecode { .. } | IRInst::Nop => None,
    }
}

fn register_dst(value: &IRValue, kind: &'static str) -> Result<u8, BytecodeLoweringError> {
    match value {
        IRValue::Register(reg, _) => Ok(*reg),
        IRValue::Constant(_) => Err(BytecodeLoweringError::UnsupportedValue { kind }),
    }
}

fn emit_move_or_load(dst: u8, src: &IRValue, lowering: &mut LoweringState) {
    match src {
        IRValue::Register(src, _) if *src == dst => {}
        IRValue::Register(src, _) => {
            lowering
                .items
                .push(LowerItem::Inst(PendingInst::Raw(encode_raw(
                    Opcode::Mov,
                    dst,
                    *src,
                    0,
                ))))
        }
        IRValue::Constant(value) => emit_load_const(dst, *value, lowering),
    }
}

fn emit_load_const(dst: u8, value: JSValue, lowering: &mut LoweringState) {
    let raw = if let Some(number) = to_f64(value)
        && number.fract() == 0.0
        && number >= i16::MIN as f64
        && number <= i16::MAX as f64
    {
        encode_asbx(Opcode::LoadI, dst, number as i16)
    } else {
        let index = constant_index(&mut lowering.constants, value);
        encode_abx(Opcode::LoadK, dst, index)
    };
    lowering.items.push(LowerItem::Inst(PendingInst::Raw(raw)));
}

fn constant_index(constants: &mut Vec<JSValue>, value: JSValue) -> u16 {
    if let Some(index) = constants.iter().position(|candidate| *candidate == value) {
        return index as u16;
    }
    let index = constants.len() as u16;
    constants.push(value);
    index
}

fn add_switch_table(
    constants: &mut Vec<JSValue>,
    pc: usize,
    labels: &HashMap<Label, usize>,
    default_target: Label,
    cases: &[(JSValue, Label)],
) -> Result<u8, BytecodeLoweringError> {
    let index = constants.len();
    if index > u8::MAX as usize {
        return Err(BytecodeLoweringError::SwitchTableTooLarge { index });
    }

    let default_pc =
        labels
            .get(&default_target)
            .copied()
            .ok_or(BytecodeLoweringError::MissingBlock {
                block: label_block_id(default_target),
            })?;
    let default_offset = i16::try_from(default_pc as isize - (pc as isize + 1)).map_err(|_| {
        BytecodeLoweringError::JumpOutOfRange {
            opcode: Opcode::Switch,
            from_pc: pc,
            to_pc: default_pc,
        }
    })?;

    constants.push(make_number(cases.len() as f64));
    constants.push(make_number(default_offset as f64));
    for (value, target) in cases {
        let target_pc = labels
            .get(target)
            .copied()
            .ok_or(BytecodeLoweringError::MissingBlock {
                block: label_block_id(*target),
            })?;
        let offset = i16::try_from(target_pc as isize - (pc as isize + 1)).map_err(|_| {
            BytecodeLoweringError::JumpOutOfRange {
                opcode: Opcode::Switch,
                from_pc: pc,
                to_pc: target_pc,
            }
        })?;
        constants.push(*value);
        constants.push(make_number(offset as f64));
    }

    Ok(index as u8)
}

fn unary_opcode(op: IRUnaryOp) -> Opcode {
    match op {
        IRUnaryOp::Typeof => Opcode::Typeof,
        IRUnaryOp::ToNum => Opcode::ToNum,
        IRUnaryOp::ToStr => Opcode::ToStr,
        IRUnaryOp::IsUndef => Opcode::IsUndef,
        IRUnaryOp::IsNull => Opcode::IsNull,
        IRUnaryOp::Neg => Opcode::Neg,
        IRUnaryOp::Inc => Opcode::Inc,
        IRUnaryOp::Dec => Opcode::Dec,
        IRUnaryOp::ToPrimitive => Opcode::ToPrimitive,
        IRUnaryOp::BitNot => Opcode::BitNot,
    }
}

fn unary_writes_to_acc(op: IRUnaryOp) -> bool {
    matches!(
        op,
        IRUnaryOp::Neg
            | IRUnaryOp::Inc
            | IRUnaryOp::Dec
            | IRUnaryOp::ToPrimitive
            | IRUnaryOp::BitNot
    )
}

fn compare_branch_opcode(
    kind: CompareKind,
    negate: bool,
    numeric: bool,
    target: BlockId,
    fallthrough: BlockId,
) -> (Opcode, BlockId, BlockId) {
    match (kind, negate, numeric) {
        (CompareKind::Eq, false, _) => (Opcode::JmpEq, target, fallthrough),
        (CompareKind::Eq, true, _) => (Opcode::JmpNeq, target, fallthrough),
        (CompareKind::Neq, false, _) => (Opcode::JmpNeq, target, fallthrough),
        (CompareKind::Neq, true, _) => (Opcode::JmpEq, target, fallthrough),
        (CompareKind::Lt, false, true) => (Opcode::JmpLtF64, target, fallthrough),
        (CompareKind::Lt, true, true) => (Opcode::JmpLtF64, fallthrough, target),
        (CompareKind::Lt, false, false) => (Opcode::JmpLt, target, fallthrough),
        (CompareKind::Lt, true, false) => (Opcode::JmpLt, fallthrough, target),
        (CompareKind::Lte, false, true) => (Opcode::JmpLteF64, target, fallthrough),
        (CompareKind::Lte, true, true) => (Opcode::JmpLteFalseF64, target, fallthrough),
        (CompareKind::Lte, false, false) => (Opcode::JmpLte, target, fallthrough),
        (CompareKind::Lte, true, false) => (Opcode::JmpLteFalse, target, fallthrough),
        (CompareKind::LteFalse, false, true) => (Opcode::JmpLteFalseF64, target, fallthrough),
        (CompareKind::LteFalse, true, true) => (Opcode::JmpLteF64, target, fallthrough),
        (CompareKind::LteFalse, false, false) => (Opcode::JmpLteFalse, target, fallthrough),
        (CompareKind::LteFalse, true, false) => (Opcode::JmpLte, target, fallthrough),
    }
}

fn binary_opcode(op: IRBinaryOp, numeric: bool) -> Opcode {
    match op {
        IRBinaryOp::Add => {
            if numeric {
                Opcode::AddF64
            } else {
                Opcode::Add
            }
        }
        IRBinaryOp::Eq => Opcode::Eq,
        IRBinaryOp::Lt => {
            if numeric {
                Opcode::LtF64
            } else {
                Opcode::Lt
            }
        }
        IRBinaryOp::Lte => {
            if numeric {
                Opcode::LteF64
            } else {
                Opcode::Lte
            }
        }
        IRBinaryOp::StrictEq => Opcode::StrictEq,
        IRBinaryOp::StrictNeq => Opcode::StrictNeq,
        IRBinaryOp::BitAnd => Opcode::BitAnd,
        IRBinaryOp::BitOr => Opcode::BitOr,
        IRBinaryOp::BitXor => Opcode::BitXor,
        IRBinaryOp::Shl => Opcode::Shl,
        IRBinaryOp::Shr => Opcode::Shr,
        IRBinaryOp::Ushr => Opcode::Ushr,
        IRBinaryOp::Pow => Opcode::Pow,
        IRBinaryOp::LogicalAnd => Opcode::LogicalAnd,
        IRBinaryOp::LogicalOr => Opcode::LogicalOr,
        IRBinaryOp::NullishCoalesce => Opcode::NullishCoalesce,
        IRBinaryOp::In => Opcode::In,
        IRBinaryOp::Instanceof => Opcode::Instanceof,
        IRBinaryOp::AddStr => Opcode::AddStr,
        IRBinaryOp::Mod => Opcode::Mod,
        IRBinaryOp::Div => Opcode::DivAcc,
        IRBinaryOp::Sub | IRBinaryOp::Mul => unreachable!("sub/mul use accumulator lowering"),
    }
}

fn encode_raw(opcode: Opcode, a: u8, b: u8, c: u8) -> u32 {
    ((c as u32) << 24) | ((b as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

fn encode_abx(opcode: Opcode, a: u8, bx: u16) -> u32 {
    ((bx as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

fn encode_asbx(opcode: Opcode, a: u8, sbx: i16) -> u32 {
    (((sbx as u16) as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

fn encode_targeted_jump(
    opcode: Opcode,
    a: u8,
    from_pc: usize,
    to_pc: usize,
) -> Result<u32, BytecodeLoweringError> {
    let offset = i16::try_from(to_pc as isize - (from_pc as isize + 1)).map_err(|_| {
        BytecodeLoweringError::JumpOutOfRange {
            opcode,
            from_pc,
            to_pc,
        }
    })?;
    Ok(encode_asbx(opcode, a, offset))
}

fn encode_compare_jump(
    opcode: Opcode,
    lhs: u8,
    rhs: u8,
    from_pc: usize,
    to_pc: usize,
) -> Result<u32, BytecodeLoweringError> {
    let offset = i8::try_from(to_pc as isize - (from_pc as isize + 1)).map_err(|_| {
        BytecodeLoweringError::JumpOutOfRange {
            opcode,
            from_pc,
            to_pc,
        }
    })?;
    Ok(encode_raw(opcode, lhs, rhs, offset as u8))
}

fn label_block_id(label: Label) -> BlockId {
    match label {
        Label::Block(block) => block,
        Label::Stub(_) => 0,
    }
}

fn mark_instruction_regs(inst: &IRInst, used: &mut [bool; REG_SLOTS]) {
    match inst {
        IRInst::Phi { dst, incoming } => {
            mark_value_reg(dst, used);
            for (_, value) in incoming {
                mark_value_reg(value, used);
            }
        }
        IRInst::Mov { dst, src } => {
            mark_value_reg(dst, used);
            mark_value_reg(src, used);
        }
        IRInst::LoadConst { dst, .. } => {
            mark_value_reg(dst, used);
        }
        IRInst::Unary { dst, operand, .. } => {
            mark_value_reg(dst, used);
            mark_value_reg(operand, used);
        }
        IRInst::Binary { dst, lhs, rhs, .. } => {
            mark_value_reg(dst, used);
            mark_value_reg(lhs, used);
            mark_value_reg(rhs, used);
        }
        IRInst::Bytecode { uses, defs, .. } => {
            for value in uses {
                mark_value_reg(value, used);
            }
            for value in defs {
                mark_value_reg(value, used);
            }
        }
        IRInst::Nop => {}
    }
}

fn mark_terminator_regs(terminator: &IRTerminator, used: &mut [bool; REG_SLOTS]) {
    match terminator {
        IRTerminator::Branch { condition, .. } => mark_condition_regs(condition, used),
        IRTerminator::Switch { key, .. }
        | IRTerminator::Throw { value: key }
        | IRTerminator::TailCall { callee: key, .. }
        | IRTerminator::CallReturn { callee: key, .. } => mark_value_reg(key, used),
        IRTerminator::ConditionalReturn {
            condition, value, ..
        } => {
            mark_condition_regs(condition, used);
            mark_value_reg(value, used);
        }
        IRTerminator::Return { value } => {
            if let Some(value) = value {
                mark_value_reg(value, used);
            }
        }
        IRTerminator::Jump { .. } | IRTerminator::Try { .. } | IRTerminator::None => {}
    }
}

fn mark_condition_regs(condition: &IRCondition, used: &mut [bool; REG_SLOTS]) {
    match condition {
        IRCondition::Truthy { value, .. } => mark_value_reg(value, used),
        IRCondition::Compare { lhs, rhs, .. } => {
            mark_value_reg(lhs, used);
            mark_value_reg(rhs, used);
        }
    }
}

fn mark_value_reg(value: &IRValue, used: &mut [bool; REG_SLOTS]) {
    if let IRValue::Register(reg, _) = value {
        used[*reg as usize] = true;
    }
}
