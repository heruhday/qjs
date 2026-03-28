use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use codegen::Opcode;
use value::JSValue;

use crate::{
    ACC_REG, BasicBlock, BlockId, CFG, CFGError, CompareKind, Condition, DecodedInst, SwitchCase,
    Terminator, branch_target_or_error, decode_switch_table, decode_word, is_conditional_branch,
    next_pc,
};

#[derive(Debug, Clone)]
enum PendingTerminator {
    None,
    Fallthrough {
        target_pc: usize,
    },
    Jump {
        target_pc: usize,
    },
    Branch {
        condition: Condition,
        target_pc: usize,
        fallthrough_pc: usize,
    },
    Switch {
        key: u8,
        cases: Vec<(JSValue, usize)>,
        default_target_pc: usize,
    },
    Try {
        handler_pc: usize,
        fallthrough_pc: usize,
    },
    ConditionalReturn {
        condition: Condition,
        value: u8,
        fallthrough_pc: usize,
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

impl PendingTerminator {
    fn successor_pcs(&self) -> Vec<usize> {
        match self {
            Self::None
            | Self::Return { .. }
            | Self::Throw { .. }
            | Self::TailCall { .. }
            | Self::CallReturn { .. } => Vec::new(),
            Self::Fallthrough { target_pc } | Self::Jump { target_pc } => vec![*target_pc],
            Self::Branch {
                target_pc,
                fallthrough_pc,
                ..
            }
            | Self::Try {
                handler_pc: target_pc,
                fallthrough_pc,
            } => vec![*target_pc, *fallthrough_pc],
            Self::Switch {
                cases,
                default_target_pc,
                ..
            } => {
                let mut successors = vec![*default_target_pc];
                successors.extend(cases.iter().map(|(_, target_pc)| *target_pc));
                successors.sort_unstable();
                successors.dedup();
                successors
            }
            Self::ConditionalReturn { fallthrough_pc, .. } => vec![*fallthrough_pc],
        }
    }
}

#[derive(Debug, Clone)]
struct PendingBlock {
    start_pc: usize,
    end_pc: usize,
    instructions: Vec<DecodedInst>,
    terminator: PendingTerminator,
}

pub fn build_cfg(
    bytecode: Vec<u32>,
    constants: Vec<JSValue>,
    entry_pc: usize,
) -> Result<CFG, CFGError> {
    if bytecode.is_empty() || entry_pc >= bytecode.len() {
        return Err(CFGError::InvalidEntry {
            entry_pc,
            len: bytecode.len(),
        });
    }

    let (leaders, reachable) = collect_reachable_leaders(&bytecode, &constants, entry_pc)?;
    let pending_blocks = build_pending_blocks(&bytecode, &constants, &leaders, &reachable)?;

    let mut pc_to_block = HashMap::new();
    for (id, block) in pending_blocks.iter().enumerate() {
        for pc in block.start_pc..=block.end_pc {
            pc_to_block.insert(pc, id);
        }
    }

    let mut blocks = Vec::with_capacity(pending_blocks.len());
    for (id, pending) in pending_blocks.into_iter().enumerate() {
        let terminator = lower_terminator(&pending.terminator, &pc_to_block)?;
        let successors = terminator.successors();
        blocks.push(BasicBlock {
            id,
            start_pc: pending.start_pc,
            end_pc: pending.end_pc,
            instructions: pending.instructions,
            predecessors: Vec::new(),
            successors,
            terminator,
            is_loop_header: false,
            is_loop_latch: false,
        });
    }

    let mut predecessor_sets = vec![BTreeSet::<BlockId>::new(); blocks.len()];
    for (block_id, block) in blocks.iter().enumerate() {
        for &successor in &block.successors {
            predecessor_sets[successor].insert(block_id);
        }
    }
    for (block, predecessors) in blocks.iter_mut().zip(predecessor_sets) {
        block.predecessors = predecessors.into_iter().collect();
    }

    for block_id in 0..blocks.len() {
        let start_pc = blocks[block_id].start_pc;
        let successors = blocks[block_id].successors.clone();
        for successor in successors {
            if blocks[successor].start_pc <= start_pc {
                blocks[successor].is_loop_header = true;
                blocks[block_id].is_loop_latch = true;
            }
        }
    }

    let exit_blocks = blocks
        .iter()
        .filter(|block| {
            matches!(
                block.terminator,
                Terminator::ConditionalReturn { .. }
                    | Terminator::Return { .. }
                    | Terminator::Throw { .. }
                    | Terminator::TailCall { .. }
                    | Terminator::CallReturn { .. }
            )
        })
        .map(|block| block.id)
        .collect::<Vec<_>>();

    Ok(CFG {
        blocks,
        entry: 0,
        exit_blocks,
        entry_pc,
        bytecode,
        constants,
        pc_to_block: HashMap::new(),
    }
    .with_block_map(pc_to_block))
}

fn collect_reachable_leaders(
    bytecode: &[u32],
    constants: &[JSValue],
    entry_pc: usize,
) -> Result<(BTreeSet<usize>, HashSet<usize>), CFGError> {
    let mut leaders = BTreeSet::new();
    let mut reachable = HashSet::new();
    let mut worklist = VecDeque::from([entry_pc]);

    leaders.insert(entry_pc);

    while let Some(start_pc) = worklist.pop_front() {
        let mut pc = start_pc;
        while pc < bytecode.len() {
            if !reachable.insert(pc) {
                break;
            }

            let inst = decode_word(pc, bytecode[pc]);
            let pending = classify_terminator(&inst, bytecode.len(), constants)?;
            let successors = pending.successor_pcs();

            if !matches!(pending, PendingTerminator::None) {
                for target_pc in successors {
                    validate_target(inst.pc, target_pc, bytecode.len())?;
                    if leaders.insert(target_pc) {
                        worklist.push_back(target_pc);
                    }
                }
                break;
            }

            pc += 1;
        }
    }

    Ok((leaders, reachable))
}

fn build_pending_blocks(
    bytecode: &[u32],
    constants: &[JSValue],
    leaders: &BTreeSet<usize>,
    reachable: &HashSet<usize>,
) -> Result<Vec<PendingBlock>, CFGError> {
    let ordered_leaders = leaders
        .iter()
        .copied()
        .filter(|leader| reachable.contains(leader))
        .collect::<Vec<_>>();
    let mut blocks = Vec::with_capacity(ordered_leaders.len());

    for (index, start_pc) in ordered_leaders.iter().copied().enumerate() {
        let next_leader = ordered_leaders.get(index + 1).copied();
        let mut instructions = Vec::new();
        let mut pc = start_pc;

        loop {
            if !reachable.contains(&pc) {
                break;
            }

            let inst = decode_word(pc, bytecode[pc]);
            let pending = classify_terminator(&inst, bytecode.len(), constants)?;
            instructions.push(inst.clone());

            if !matches!(pending, PendingTerminator::None) {
                blocks.push(PendingBlock {
                    start_pc,
                    end_pc: pc,
                    instructions,
                    terminator: pending,
                });
                break;
            }

            let next_pc = pc + 1;
            if let Some(next_leader_pc) = next_leader {
                if next_pc == next_leader_pc {
                    blocks.push(PendingBlock {
                        start_pc,
                        end_pc: pc,
                        instructions,
                        terminator: PendingTerminator::Fallthrough {
                            target_pc: next_leader_pc,
                        },
                    });
                    break;
                }
            }

            if next_pc >= bytecode.len() || !reachable.contains(&next_pc) {
                blocks.push(PendingBlock {
                    start_pc,
                    end_pc: pc,
                    instructions,
                    terminator: PendingTerminator::None,
                });
                break;
            }

            pc = next_pc;
        }
    }

    Ok(blocks)
}

fn classify_terminator(
    inst: &DecodedInst,
    len: usize,
    constants: &[JSValue],
) -> Result<PendingTerminator, CFGError> {
    match inst.opcode {
        Opcode::Jmp | Opcode::IncAccJmp => jump_terminator(inst, len),
        Opcode::JmpTrue | Opcode::TestJmpTrue => truthy_branch(inst, len, inst.a, false),
        Opcode::JmpFalse | Opcode::LoadJfalse | Opcode::IncJmpFalseLoop => {
            truthy_branch(inst, len, inst.a, true)
        }
        Opcode::JmpEq | Opcode::LoadCmpEqJfalse => {
            compare_branch(inst, len, CompareKind::Eq, inst.a, inst.b, false)
        }
        Opcode::JmpNeq => compare_branch(inst, len, CompareKind::Eq, inst.a, inst.b, true),
        Opcode::JmpLt
        | Opcode::JmpLtF64
        | Opcode::LoadCmpLtJfalse
        | Opcode::JmpI32Fast
        | Opcode::CmpJmp => compare_branch(inst, len, CompareKind::Lt, inst.a, inst.b, false),
        Opcode::JmpLte | Opcode::JmpLteF64 => {
            compare_branch(inst, len, CompareKind::Lte, inst.a, inst.b, false)
        }
        Opcode::JmpLteFalse | Opcode::JmpLteFalseF64 => {
            compare_branch(inst, len, CompareKind::Lte, inst.a, inst.b, true)
        }
        Opcode::EqJmpTrue => compare_branch(inst, len, CompareKind::Eq, inst.b, inst.c, false),
        Opcode::EqJmpFalse => compare_branch(inst, len, CompareKind::Eq, inst.b, inst.c, true),
        Opcode::LtJmp => compare_branch(inst, len, CompareKind::Lt, inst.b, inst.c, false),
        Opcode::LteJmpLoop => compare_branch(inst, len, CompareKind::Lte, inst.b, inst.c, false),
        Opcode::LoopIncJmp => compare_branch(inst, len, CompareKind::Lt, inst.a, ACC_REG, false),
        Opcode::Switch => {
            let Some(table) = decode_switch_table(constants, inst.b as usize, inst.pc) else {
                return Err(CFGError::InvalidSwitchTable {
                    pc: inst.pc,
                    table_index: inst.b as usize,
                });
            };
            validate_target(inst.pc, table.default_target_pc, len)?;
            let mut cases = Vec::with_capacity(table.cases.len());
            for case in table.cases {
                validate_target(inst.pc, case.target_pc, len)?;
                cases.push((case.value, case.target_pc));
            }
            Ok(PendingTerminator::Switch {
                key: inst.a,
                cases,
                default_target_pc: table.default_target_pc,
            })
        }
        Opcode::Try => Ok(PendingTerminator::Try {
            handler_pc: branch_target_or_error(inst, len)?,
            fallthrough_pc: next_pc(inst, len)?,
        }),
        Opcode::Ret => Ok(PendingTerminator::Return {
            value: Some(ACC_REG),
        }),
        Opcode::RetReg => Ok(PendingTerminator::Return {
            value: Some(inst.a),
        }),
        Opcode::RetU => Ok(PendingTerminator::Return { value: None }),
        Opcode::Throw => Ok(PendingTerminator::Throw { value: inst.a }),
        Opcode::TailCall => Ok(PendingTerminator::TailCall {
            callee: inst.a,
            argc: inst.b,
        }),
        Opcode::CallRet => Ok(PendingTerminator::CallReturn {
            callee: inst.a,
            argc: inst.b,
        }),
        Opcode::RetIfLteI => Ok(PendingTerminator::ConditionalReturn {
            condition: Condition::Compare {
                kind: CompareKind::Lte,
                lhs: inst.a,
                rhs: inst.b,
                negate: false,
            },
            value: inst.c,
            fallthrough_pc: next_pc(inst, len)?,
        }),
        opcode if is_conditional_branch(opcode) => truthy_branch(inst, len, inst.a, false),
        _ => Ok(PendingTerminator::None),
    }
}

fn jump_terminator(inst: &DecodedInst, len: usize) -> Result<PendingTerminator, CFGError> {
    Ok(PendingTerminator::Jump {
        target_pc: branch_target_or_error(inst, len)?,
    })
}

fn truthy_branch(
    inst: &DecodedInst,
    len: usize,
    reg: u8,
    negate: bool,
) -> Result<PendingTerminator, CFGError> {
    Ok(PendingTerminator::Branch {
        condition: Condition::Truthy { reg, negate },
        target_pc: branch_target_or_error(inst, len)?,
        fallthrough_pc: next_pc(inst, len)?,
    })
}

fn compare_branch(
    inst: &DecodedInst,
    len: usize,
    kind: CompareKind,
    lhs: u8,
    rhs: u8,
    negate: bool,
) -> Result<PendingTerminator, CFGError> {
    Ok(PendingTerminator::Branch {
        condition: Condition::Compare {
            kind,
            lhs,
            rhs,
            negate,
        },
        target_pc: branch_target_or_error(inst, len)?,
        fallthrough_pc: next_pc(inst, len)?,
    })
}

fn lower_terminator(
    pending: &PendingTerminator,
    pc_to_block: &HashMap<usize, BlockId>,
) -> Result<Terminator, CFGError> {
    let block_for = |pc| {
        pc_to_block
            .get(&pc)
            .copied()
            .ok_or(CFGError::MissingBlockForPc { pc })
    };

    match pending {
        PendingTerminator::None => Ok(Terminator::None),
        PendingTerminator::Fallthrough { target_pc } => Ok(Terminator::Fallthrough {
            target: block_for(*target_pc)?,
        }),
        PendingTerminator::Jump { target_pc } => Ok(Terminator::Jump {
            target: block_for(*target_pc)?,
        }),
        PendingTerminator::Branch {
            condition,
            target_pc,
            fallthrough_pc,
        } => Ok(Terminator::Branch {
            condition: *condition,
            target: block_for(*target_pc)?,
            fallthrough: block_for(*fallthrough_pc)?,
        }),
        PendingTerminator::Switch {
            key,
            cases,
            default_target_pc,
        } => Ok(Terminator::Switch {
            key: *key,
            cases: cases
                .iter()
                .map(|(value, target_pc)| {
                    Ok(SwitchCase {
                        value: *value,
                        target: block_for(*target_pc)?,
                    })
                })
                .collect::<Result<Vec<_>, CFGError>>()?,
            default_target: block_for(*default_target_pc)?,
        }),
        PendingTerminator::Try {
            handler_pc,
            fallthrough_pc,
        } => Ok(Terminator::Try {
            handler: block_for(*handler_pc)?,
            fallthrough: block_for(*fallthrough_pc)?,
        }),
        PendingTerminator::ConditionalReturn {
            condition,
            value,
            fallthrough_pc,
        } => Ok(Terminator::ConditionalReturn {
            condition: *condition,
            value: *value,
            fallthrough: block_for(*fallthrough_pc)?,
        }),
        PendingTerminator::Return { value } => Ok(Terminator::Return { value: *value }),
        PendingTerminator::Throw { value } => Ok(Terminator::Throw { value: *value }),
        PendingTerminator::TailCall { callee, argc } => Ok(Terminator::TailCall {
            callee: *callee,
            argc: *argc,
        }),
        PendingTerminator::CallReturn { callee, argc } => Ok(Terminator::CallReturn {
            callee: *callee,
            argc: *argc,
        }),
    }
}

fn validate_target(pc: usize, target_pc: usize, len: usize) -> Result<(), CFGError> {
    if target_pc < len {
        Ok(())
    } else {
        Err(CFGError::InvalidBranchTarget { pc, target_pc, len })
    }
}
