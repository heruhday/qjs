use std::cell::Cell;
use std::rc::Rc;

use cfg::{ACC_REG, CFG, decode_word};
use codegen::Opcode;
use ssa::{
    CfgSimplification, GlobalValueNumbering, IRBinaryOp, IRBlock, IRCondition, IRFunction, IRInst,
    IRTerminator, IRUnaryOp, IRValue, LoopInvariantCodeMotion, Pass, PassManager,
    SparseConditionalConstantPropagation, ValueRangePropagation, build_ssa, coalesce_registers,
    fold_temporary_checks, optimize_bytecode, optimize_ir, optimize_superinstructions,
    optimize_to_bytecode, reuse_registers_linear_scan,
};
use value::{make_false, make_int32, make_null, make_true};

fn encode_raw(opcode: Opcode, a: u8, b: u8, c: u8) -> u32 {
    ((c as u32) << 24) | ((b as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

fn encode_asbx(opcode: Opcode, a: u8, sbx: i16) -> u32 {
    (((sbx as u16) as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

fn sample_branch_bytecode() -> Vec<u32> {
    vec![
        encode_asbx(Opcode::LoadI, 0, 1),
        encode_asbx(Opcode::JmpFalse, 0, 2),
        encode_asbx(Opcode::LoadI, 1, 10),
        encode_asbx(Opcode::Jmp, 0, 1),
        encode_asbx(Opcode::LoadI, 1, 20),
        encode_raw(Opcode::RetReg, 1, 0, 0),
    ]
}

#[test]
fn inserts_phi_nodes_at_merge_points() {
    let cfg = CFG::from_parts(sample_branch_bytecode(), Vec::new(), 0).expect("cfg");
    let ssa = build_ssa(cfg, 256);
    let merge = ssa.cfg.block_containing_pc(5).expect("merge block");

    let phi = ssa.phi_for(merge, 1).expect("phi for r1");
    assert_eq!(phi.incoming.len(), 2);
    assert!(
        phi.incoming
            .iter()
            .any(|(block, _)| ssa.cfg.blocks[*block].start_pc == 2)
    );
    assert!(
        phi.incoming
            .iter()
            .any(|(block, _)| ssa.cfg.blocks[*block].start_pc == 4)
    );
}

#[test]
fn lowers_phi_and_return_to_ir() {
    let cfg = CFG::from_parts(sample_branch_bytecode(), Vec::new(), 0).expect("cfg");
    let ssa = build_ssa(cfg, 256);
    let ir = ssa.to_ir();
    let merge = ssa.cfg.block_containing_pc(5).expect("merge block");
    let block = &ir.blocks[merge];

    assert!(
        block
            .instructions
            .iter()
            .any(|inst| matches!(inst, IRInst::Phi { .. }))
    );
    assert!(matches!(
        block.terminator,
        IRTerminator::Return { value: Some(_) }
    ));
}

#[test]
fn preserves_unlowered_bytecode_as_generic_ir() {
    let bytecode = vec![
        encode_raw(Opcode::GetProp2Ic, 1, 2, 3),
        encode_raw(Opcode::RetU, 0, 0, 0),
    ];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");
    let ssa = build_ssa(cfg, 256);
    let ir = ssa.to_ir();

    assert!(matches!(
        ir.blocks[0].instructions.first(),
        Some(IRInst::Bytecode { .. })
    ));
}

#[test]
fn ir_round_trips_back_to_valid_bytecode() {
    let cfg = CFG::from_parts(sample_branch_bytecode(), Vec::new(), 0).expect("cfg");
    let ssa = build_ssa(cfg, 256);
    let ir = ssa.to_ir();

    let (bytecode, constants) = ir.into_bytecodes().expect("bytecode");
    assert!(constants.is_empty());

    let lowered_cfg = CFG::from_parts(bytecode.clone(), constants, 0).expect("lowered cfg");
    assert!(!lowered_cfg.blocks.is_empty());

    let last_pc = bytecode.len() - 1;
    let last = decode_word(last_pc, bytecode[last_pc]);
    assert!(matches!(
        last.opcode,
        Opcode::Ret | Opcode::RetReg | Opcode::RetU
    ));
}

#[test]
fn ssa_into_bytecodes_matches_ir_helper() {
    let cfg = CFG::from_parts(sample_branch_bytecode(), Vec::new(), 0).expect("cfg");
    let ssa = build_ssa(cfg.clone(), 256);
    let (from_ssa, from_ssa_consts) = ssa.into_bytecodes().expect("ssa bytecode");

    let ssa = build_ssa(cfg, 256);
    let (from_ir, from_ir_consts) = ssa.to_ir().into_bytecodes().expect("ir bytecode");

    assert_eq!(from_ssa, from_ir);
    assert_eq!(from_ssa_consts, from_ir_consts);
}

#[test]
fn lowers_common_unary_and_binary_ops_into_structured_ir() {
    let bytecode = vec![
        encode_asbx(Opcode::LoadI, 1, 3),
        encode_asbx(Opcode::LoadI, 2, 4),
        encode_raw(Opcode::Eq, 0, 1, 2),
        encode_raw(Opcode::Neg, 0, 1, 0),
        encode_raw(Opcode::RetReg, ACC_REG, 0, 0),
    ];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");
    let ssa = build_ssa(cfg, 256);
    let ir = ssa.to_ir();
    let block = &ir.blocks[0];

    assert!(block.instructions.iter().any(|inst| matches!(
        inst,
        IRInst::Binary {
            op: IRBinaryOp::Eq,
            ..
        }
    )));
    assert!(block.instructions.iter().any(|inst| matches!(
        inst,
        IRInst::Unary {
            op: IRUnaryOp::Neg,
            ..
        }
    )));
}

#[test]
fn lowers_multi_def_acc_bytecode_into_ir_op_plus_copy() {
    let bytecode = vec![
        encode_raw(Opcode::LoadTrue, 7, 0, 0),
        encode_raw(Opcode::RetReg, 7, 0, 0),
    ];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");
    let ssa = build_ssa(cfg, 256);
    let ir = ssa.to_ir();
    let block = &ir.blocks[0];

    assert!(matches!(
        block.instructions.first(),
        Some(IRInst::LoadConst { .. })
    ));
    assert!(
        block
            .instructions
            .iter()
            .any(|inst| matches!(inst, IRInst::Mov { .. }))
    );
}

#[test]
fn builds_ssa_for_cfg_without_explicit_exit() {
    let bytecode = vec![
        encode_asbx(Opcode::LoadI, 0, 7),
        encode_raw(Opcode::Mov, 1, 0, 0),
    ];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");
    assert!(cfg.exit_blocks.is_empty());

    let ssa = build_ssa(cfg, 256);
    assert_eq!(ssa.blocks.len(), 1);
    assert_eq!(ssa.blocks[0].instructions.len(), 2);
}

#[test]
fn gvn_reuses_dominating_expression_across_blocks() {
    let lhs = IRValue::Register(1, 0);
    let rhs = IRValue::Register(2, 0);
    let first = IRValue::Register(3, 0);
    let second = IRValue::Register(4, 0);

    let mut ir = IRFunction {
        blocks: vec![
            IRBlock {
                id: 0,
                instructions: vec![
                    IRInst::LoadConst {
                        dst: lhs.clone(),
                        value: make_int32(4),
                    },
                    IRInst::LoadConst {
                        dst: rhs.clone(),
                        value: make_int32(5),
                    },
                    IRInst::Binary {
                        dst: first.clone(),
                        op: IRBinaryOp::Add,
                        lhs: lhs.clone(),
                        rhs: rhs.clone(),
                    },
                ],
                terminator: IRTerminator::Jump { target: 1 },
                successors: vec![1],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 1,
                instructions: vec![IRInst::Binary {
                    dst: second.clone(),
                    op: IRBinaryOp::Add,
                    lhs: lhs.clone(),
                    rhs: rhs.clone(),
                }],
                terminator: IRTerminator::Return {
                    value: Some(second.clone()),
                },
                successors: Vec::new(),
                predecessors: vec![0],
            },
        ],
        entry: 0,
        exit_blocks: vec![1],
        constants: Vec::new(),
    };

    let changed = GlobalValueNumbering.run(&mut ir);

    assert!(changed);
    assert!(matches!(
        ir.blocks[1].instructions.first(),
        Some(IRInst::Mov { dst, src }) if *dst == second && *src == first
    ));
}

#[test]
fn gvn_does_not_reuse_non_dominating_branch_expression() {
    let condition = IRValue::Register(9, 0);
    let lhs = IRValue::Register(1, 0);
    let rhs = IRValue::Register(2, 0);
    let branch_value = IRValue::Register(3, 0);
    let merge_value = IRValue::Register(4, 0);

    let mut ir = IRFunction {
        blocks: vec![
            IRBlock {
                id: 0,
                instructions: vec![
                    IRInst::LoadConst {
                        dst: lhs.clone(),
                        value: make_int32(4),
                    },
                    IRInst::LoadConst {
                        dst: rhs.clone(),
                        value: make_int32(5),
                    },
                ],
                terminator: IRTerminator::Branch {
                    condition: IRCondition::Truthy {
                        value: condition,
                        negate: false,
                    },
                    target: 1,
                    fallthrough: 2,
                },
                successors: vec![1, 2],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 1,
                instructions: vec![IRInst::Binary {
                    dst: branch_value.clone(),
                    op: IRBinaryOp::Add,
                    lhs: lhs.clone(),
                    rhs: rhs.clone(),
                }],
                terminator: IRTerminator::Jump { target: 3 },
                successors: vec![3],
                predecessors: vec![0],
            },
            IRBlock {
                id: 2,
                instructions: vec![IRInst::LoadConst {
                    dst: IRValue::Register(8, 0),
                    value: make_false(),
                }],
                terminator: IRTerminator::Jump { target: 3 },
                successors: vec![3],
                predecessors: vec![0],
            },
            IRBlock {
                id: 3,
                instructions: vec![IRInst::Binary {
                    dst: merge_value.clone(),
                    op: IRBinaryOp::Add,
                    lhs: lhs.clone(),
                    rhs: rhs.clone(),
                }],
                terminator: IRTerminator::Return {
                    value: Some(merge_value),
                },
                successors: Vec::new(),
                predecessors: vec![1, 2],
            },
        ],
        entry: 0,
        exit_blocks: vec![3],
        constants: Vec::new(),
    };

    let changed = GlobalValueNumbering.run(&mut ir);

    assert!(!changed);
    assert!(matches!(
        ir.blocks[3].instructions.first(),
        Some(IRInst::Binary {
            op: IRBinaryOp::Add,
            ..
        })
    ));
}

#[test]
fn vrp_simplifies_branch_from_dominating_range_check() {
    let x = IRValue::Register(0, 0);

    let mut ir = IRFunction {
        blocks: vec![
            IRBlock {
                id: 0,
                instructions: Vec::new(),
                terminator: IRTerminator::Branch {
                    condition: IRCondition::Compare {
                        kind: cfg::CompareKind::Lt,
                        lhs: x.clone(),
                        rhs: IRValue::Constant(make_int32(10)),
                        negate: false,
                    },
                    target: 1,
                    fallthrough: 2,
                },
                successors: vec![1, 2],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 1,
                instructions: Vec::new(),
                terminator: IRTerminator::Branch {
                    condition: IRCondition::Compare {
                        kind: cfg::CompareKind::Lt,
                        lhs: x,
                        rhs: IRValue::Constant(make_int32(20)),
                        negate: false,
                    },
                    target: 3,
                    fallthrough: 4,
                },
                successors: vec![3, 4],
                predecessors: vec![0],
            },
            IRBlock {
                id: 2,
                instructions: Vec::new(),
                terminator: IRTerminator::Return {
                    value: Some(IRValue::Constant(make_false())),
                },
                successors: Vec::new(),
                predecessors: vec![0],
            },
            IRBlock {
                id: 3,
                instructions: Vec::new(),
                terminator: IRTerminator::Return {
                    value: Some(IRValue::Constant(make_true())),
                },
                successors: Vec::new(),
                predecessors: vec![1],
            },
            IRBlock {
                id: 4,
                instructions: Vec::new(),
                terminator: IRTerminator::Return {
                    value: Some(IRValue::Constant(make_false())),
                },
                successors: Vec::new(),
                predecessors: vec![1],
            },
        ],
        entry: 0,
        exit_blocks: vec![2, 3, 4],
        constants: Vec::new(),
    };

    let changed = ValueRangePropagation.run(&mut ir);

    assert!(changed);
    assert!(matches!(
        ir.blocks[1].terminator,
        IRTerminator::Jump { target: 3 }
    ));
    assert_eq!(ir.blocks[4].predecessors, Vec::<usize>::new());
}

#[test]
fn vrp_rewrites_constant_phi_to_load_const() {
    let phi = IRValue::Register(5, 0);

    let mut ir = IRFunction {
        blocks: vec![
            IRBlock {
                id: 0,
                instructions: Vec::new(),
                terminator: IRTerminator::Branch {
                    condition: IRCondition::Truthy {
                        value: IRValue::Register(0, 0),
                        negate: false,
                    },
                    target: 1,
                    fallthrough: 2,
                },
                successors: vec![1, 2],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 1,
                instructions: Vec::new(),
                terminator: IRTerminator::Jump { target: 3 },
                successors: vec![3],
                predecessors: vec![0],
            },
            IRBlock {
                id: 2,
                instructions: Vec::new(),
                terminator: IRTerminator::Jump { target: 3 },
                successors: vec![3],
                predecessors: vec![0],
            },
            IRBlock {
                id: 3,
                instructions: vec![IRInst::Phi {
                    dst: phi.clone(),
                    incoming: vec![
                        (1, IRValue::Constant(make_int32(7))),
                        (2, IRValue::Constant(make_int32(7))),
                    ],
                }],
                terminator: IRTerminator::Return {
                    value: Some(phi.clone()),
                },
                successors: Vec::new(),
                predecessors: vec![1, 2],
            },
        ],
        entry: 0,
        exit_blocks: vec![3],
        constants: Vec::new(),
    };

    let changed = ValueRangePropagation.run(&mut ir);

    assert!(changed);
    assert!(matches!(
        ir.blocks[3].instructions.first(),
        Some(IRInst::LoadConst { dst, value }) if *dst == phi && *value == make_int32(7)
    ));
}

#[test]
fn sccp_prunes_dead_branch_and_folds_constants() {
    let cond = IRValue::Register(0, 0);
    let result = IRValue::Register(1, 0);

    let mut ir = IRFunction {
        blocks: vec![
            IRBlock {
                id: 0,
                instructions: vec![IRInst::LoadConst {
                    dst: cond.clone(),
                    value: make_true(),
                }],
                terminator: IRTerminator::Branch {
                    condition: IRCondition::Truthy {
                        value: cond,
                        negate: false,
                    },
                    target: 1,
                    fallthrough: 2,
                },
                successors: vec![1, 2],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 1,
                instructions: vec![IRInst::LoadConst {
                    dst: result.clone(),
                    value: make_int32(7),
                }],
                terminator: IRTerminator::Return {
                    value: Some(result.clone()),
                },
                successors: Vec::new(),
                predecessors: vec![0],
            },
            IRBlock {
                id: 2,
                instructions: vec![IRInst::LoadConst {
                    dst: result.clone(),
                    value: make_int32(9),
                }],
                terminator: IRTerminator::Return {
                    value: Some(result.clone()),
                },
                successors: Vec::new(),
                predecessors: vec![0],
            },
        ],
        entry: 0,
        exit_blocks: vec![1, 2],
        constants: Vec::new(),
    };

    let changed = SparseConditionalConstantPropagation.run(&mut ir);

    assert!(changed);
    assert_eq!(ir.blocks.len(), 1);
    assert!(ir.blocks[0].instructions.iter().any(|inst| matches!(
        inst,
        IRInst::LoadConst { dst, value } if *dst == result && *value == make_int32(7)
    )));
    assert!(matches!(
        ir.blocks[0].terminator,
        IRTerminator::Return {
            value: Some(IRValue::Register(1, 0))
        }
    ));
}

#[test]
fn cfg_simplification_removes_unreachable_blocks_and_reduces_phi() {
    let phi = IRValue::Register(5, 0);

    let mut ir = IRFunction {
        blocks: vec![
            IRBlock {
                id: 0,
                instructions: Vec::new(),
                terminator: IRTerminator::Jump { target: 1 },
                successors: vec![1],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 1,
                instructions: Vec::new(),
                terminator: IRTerminator::Jump { target: 3 },
                successors: vec![3],
                predecessors: vec![0],
            },
            IRBlock {
                id: 2,
                instructions: vec![IRInst::LoadConst {
                    dst: IRValue::Register(9, 0),
                    value: make_false(),
                }],
                terminator: IRTerminator::Jump { target: 3 },
                successors: vec![3],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 3,
                instructions: vec![IRInst::Phi {
                    dst: phi.clone(),
                    incoming: vec![
                        (1, IRValue::Constant(make_int32(7))),
                        (2, IRValue::Constant(make_int32(7))),
                    ],
                }],
                terminator: IRTerminator::Return {
                    value: Some(phi.clone()),
                },
                successors: Vec::new(),
                predecessors: vec![1, 2],
            },
        ],
        entry: 0,
        exit_blocks: vec![3],
        constants: Vec::new(),
    };

    let changed = CfgSimplification.run(&mut ir);

    assert!(changed);
    assert_eq!(ir.blocks.len(), 1);
    assert!(
        !ir.blocks[0]
            .instructions
            .iter()
            .any(|inst| matches!(inst, IRInst::Phi { .. }))
    );
    assert!(ir.blocks[0].instructions.iter().any(|inst| matches!(
        inst,
        IRInst::LoadConst { dst, value } if *dst == phi && *value == make_int32(7)
    )));
    assert!(matches!(
        ir.blocks[0].terminator,
        IRTerminator::Return {
            value: Some(IRValue::Register(5, 0))
        }
    ));
}

#[test]
fn pass_manager_restarts_after_cfg_simplification_changes() {
    struct StructuralPass {
        changed_once: Rc<Cell<bool>>,
    }

    impl Pass for StructuralPass {
        fn name(&self) -> &'static str {
            "CfgSimplification"
        }

        fn is_structural(&self) -> bool {
            true
        }

        fn run(&self, ir: &mut IRFunction) -> bool {
            if self.changed_once.get() {
                return false;
            }

            self.changed_once.set(true);
            ir.constants.push(make_int32(1));
            true
        }
    }

    struct ObserverPass {
        calls: Rc<Cell<usize>>,
    }

    impl Pass for ObserverPass {
        fn name(&self) -> &'static str {
            "ObserverPass"
        }

        fn run(&self, _ir: &mut IRFunction) -> bool {
            self.calls.set(self.calls.get() + 1);
            false
        }
    }

    let changed_once = Rc::new(Cell::new(false));
    let observer_calls = Rc::new(Cell::new(0));

    let mut manager = PassManager::new();
    manager.set_max_iterations(4);
    manager.add_pass(StructuralPass {
        changed_once: changed_once.clone(),
    });
    manager.add_pass(ObserverPass {
        calls: observer_calls.clone(),
    });

    let mut ir = IRFunction {
        blocks: vec![IRBlock {
            id: 0,
            instructions: Vec::new(),
            terminator: IRTerminator::Return { value: None },
            successors: Vec::new(),
            predecessors: Vec::new(),
        }],
        entry: 0,
        exit_blocks: vec![0],
        constants: Vec::new(),
    };

    let changed = manager.run(&mut ir);

    assert!(changed);
    assert_eq!(observer_calls.get(), 1);
    assert_eq!(ir.constants, vec![make_int32(1)]);
}

#[test]
fn fold_temporary_checks_folds_known_null_checks() {
    let input = IRValue::Register(0, 0);
    let check = IRValue::Register(1, 0);

    let mut ir = IRFunction {
        blocks: vec![IRBlock {
            id: 0,
            instructions: vec![
                IRInst::LoadConst {
                    dst: input.clone(),
                    value: make_null(),
                },
                IRInst::Unary {
                    dst: check.clone(),
                    op: IRUnaryOp::IsNull,
                    operand: input,
                },
            ],
            terminator: IRTerminator::Return {
                value: Some(check.clone()),
            },
            successors: Vec::new(),
            predecessors: Vec::new(),
        }],
        entry: 0,
        exit_blocks: vec![0],
        constants: Vec::new(),
    };

    let changed = fold_temporary_checks(&mut ir);

    assert!(changed);
    assert!(matches!(
        ir.blocks[0].instructions.get(1),
        Some(IRInst::LoadConst { dst, value }) if *dst == check && *value == make_true()
    ));
}

#[test]
fn optimize_superinstructions_sinks_terminal_constant_into_return() {
    let value = IRValue::Register(3, 0);

    let mut ir = IRFunction {
        blocks: vec![IRBlock {
            id: 0,
            instructions: vec![IRInst::LoadConst {
                dst: value.clone(),
                value: make_int32(7),
            }],
            terminator: IRTerminator::Return { value: Some(value) },
            successors: Vec::new(),
            predecessors: Vec::new(),
        }],
        entry: 0,
        exit_blocks: vec![0],
        constants: Vec::new(),
    };

    let changed = optimize_superinstructions(&mut ir);

    assert!(changed);
    assert!(ir.blocks[0].instructions.is_empty());
    assert!(matches!(
        ir.blocks[0].terminator,
        IRTerminator::Return {
            value: Some(IRValue::Constant(value))
        } if value == make_int32(7)
    ));
}

#[test]
fn coalesce_registers_removes_copy_chains() {
    let source = IRValue::Register(0, 0);

    let mut ir = IRFunction {
        blocks: vec![IRBlock {
            id: 0,
            instructions: vec![
                IRInst::LoadConst {
                    dst: source.clone(),
                    value: make_int32(7),
                },
                IRInst::Mov {
                    dst: IRValue::Register(1, 0),
                    src: source.clone(),
                },
                IRInst::Mov {
                    dst: IRValue::Register(2, 0),
                    src: IRValue::Register(1, 0),
                },
            ],
            terminator: IRTerminator::Return {
                value: Some(IRValue::Register(2, 0)),
            },
            successors: Vec::new(),
            predecessors: Vec::new(),
        }],
        entry: 0,
        exit_blocks: vec![0],
        constants: Vec::new(),
    };

    let changed = coalesce_registers(&mut ir);

    assert!(changed);
    assert!(
        !ir.blocks[0]
            .instructions
            .iter()
            .any(|inst| matches!(inst, IRInst::Mov { .. }))
    );
    assert!(matches!(
        ir.blocks[0].terminator,
        IRTerminator::Return {
            value: Some(IRValue::Register(0, 0))
        } | IRTerminator::Return {
            value: Some(IRValue::Constant(_))
        }
    ));
}

#[test]
fn licm_hoists_loop_invariants_into_existing_preheader() {
    let entry_cond = IRValue::Register(0, 0);
    let lhs = IRValue::Register(1, 0);
    let rhs = IRValue::Register(2, 0);
    let invariant = IRValue::Register(3, 0);
    let carried = IRValue::Register(4, 0);

    let mut ir = IRFunction {
        blocks: vec![
            IRBlock {
                id: 0,
                instructions: vec![
                    IRInst::LoadConst {
                        dst: entry_cond.clone(),
                        value: make_true(),
                    },
                    IRInst::LoadConst {
                        dst: lhs.clone(),
                        value: make_int32(4),
                    },
                    IRInst::LoadConst {
                        dst: rhs.clone(),
                        value: make_int32(5),
                    },
                ],
                terminator: IRTerminator::Jump { target: 1 },
                successors: vec![1],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 1,
                instructions: Vec::new(),
                terminator: IRTerminator::Branch {
                    condition: IRCondition::Truthy {
                        value: entry_cond,
                        negate: false,
                    },
                    target: 2,
                    fallthrough: 3,
                },
                successors: vec![2, 3],
                predecessors: vec![0, 2],
            },
            IRBlock {
                id: 2,
                instructions: vec![
                    IRInst::Binary {
                        dst: invariant.clone(),
                        op: IRBinaryOp::Add,
                        lhs: lhs.clone(),
                        rhs: rhs.clone(),
                    },
                    IRInst::Mov {
                        dst: carried.clone(),
                        src: invariant.clone(),
                    },
                ],
                terminator: IRTerminator::Jump { target: 1 },
                successors: vec![1],
                predecessors: vec![1],
            },
            IRBlock {
                id: 3,
                instructions: Vec::new(),
                terminator: IRTerminator::Return { value: None },
                successors: Vec::new(),
                predecessors: vec![1],
            },
        ],
        entry: 0,
        exit_blocks: vec![3],
        constants: Vec::new(),
    };

    let changed = LoopInvariantCodeMotion.run(&mut ir);

    assert!(changed);
    assert!(ir.blocks[2].instructions.is_empty());
    assert!(ir.blocks[0].instructions.iter().any(|inst| matches!(
        inst,
        IRInst::Binary {
            dst,
            op: IRBinaryOp::Add,
            lhs: binary_lhs,
            rhs: binary_rhs,
        } if *dst == invariant && *binary_lhs == lhs && *binary_rhs == rhs
    )));
    assert!(ir.blocks[0].instructions.iter().any(|inst| matches!(
        inst,
        IRInst::Mov { dst, src } if *dst == carried && *src == invariant
    )));
}

#[test]
fn licm_splits_unique_entry_edge_and_rewrites_header_phi() {
    let entry_cond = IRValue::Register(0, 0);
    let loop_cond = IRValue::Register(1, 0);
    let start = IRValue::Register(2, 0);
    let phi = IRValue::Register(3, 0);
    let invariant = IRValue::Register(4, 0);
    let next = IRValue::Register(5, 0);

    let mut ir = IRFunction {
        blocks: vec![
            IRBlock {
                id: 0,
                instructions: vec![
                    IRInst::LoadConst {
                        dst: entry_cond.clone(),
                        value: make_true(),
                    },
                    IRInst::LoadConst {
                        dst: loop_cond.clone(),
                        value: make_true(),
                    },
                    IRInst::LoadConst {
                        dst: start.clone(),
                        value: make_int32(1),
                    },
                ],
                terminator: IRTerminator::Branch {
                    condition: IRCondition::Truthy {
                        value: entry_cond,
                        negate: false,
                    },
                    target: 1,
                    fallthrough: 4,
                },
                successors: vec![1, 4],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 1,
                instructions: vec![IRInst::Phi {
                    dst: phi.clone(),
                    incoming: vec![(0, start.clone()), (2, next.clone())],
                }],
                terminator: IRTerminator::Branch {
                    condition: IRCondition::Truthy {
                        value: loop_cond,
                        negate: false,
                    },
                    target: 2,
                    fallthrough: 3,
                },
                successors: vec![2, 3],
                predecessors: vec![0, 2],
            },
            IRBlock {
                id: 2,
                instructions: vec![
                    IRInst::LoadConst {
                        dst: invariant.clone(),
                        value: make_int32(7),
                    },
                    IRInst::Mov {
                        dst: next.clone(),
                        src: invariant.clone(),
                    },
                ],
                terminator: IRTerminator::Jump { target: 1 },
                successors: vec![1],
                predecessors: vec![1],
            },
            IRBlock {
                id: 3,
                instructions: Vec::new(),
                terminator: IRTerminator::Return {
                    value: Some(phi.clone()),
                },
                successors: Vec::new(),
                predecessors: vec![1],
            },
            IRBlock {
                id: 4,
                instructions: Vec::new(),
                terminator: IRTerminator::Return { value: None },
                successors: Vec::new(),
                predecessors: vec![0],
            },
        ],
        entry: 0,
        exit_blocks: vec![3, 4],
        constants: Vec::new(),
    };

    let changed = LoopInvariantCodeMotion.run(&mut ir);

    assert!(changed);
    assert_eq!(ir.blocks.len(), 6);
    assert!(matches!(
        ir.blocks[0].terminator,
        IRTerminator::Branch {
            target: 5,
            fallthrough: 4,
            ..
        }
    ));
    assert!(matches!(
        ir.blocks[5].terminator,
        IRTerminator::Jump { target: 1 }
    ));
    assert!(ir.blocks[5].instructions.iter().any(|inst| matches!(
        inst,
        IRInst::LoadConst { dst, value } if *dst == invariant && *value == make_int32(7)
    )));
    assert!(ir.blocks[5].instructions.iter().any(|inst| matches!(
        inst,
        IRInst::Mov { dst, src } if *dst == next && *src == invariant
    )));

    let phi_incoming = match ir.blocks[1].instructions.first() {
        Some(IRInst::Phi { incoming, .. }) => incoming.clone(),
        other => panic!("expected phi at loop header, got {other:?}"),
    };
    assert!(phi_incoming.contains(&(5, start)));
    assert!(phi_incoming.contains(&(2, next)));
}

#[test]
fn licm_does_not_hoist_opaque_bytecode() {
    let cond = IRValue::Register(0, 0);
    let used = IRValue::Register(1, 0);
    let defined = IRValue::Register(2, 0);

    let mut ir = IRFunction {
        blocks: vec![
            IRBlock {
                id: 0,
                instructions: vec![IRInst::LoadConst {
                    dst: cond.clone(),
                    value: make_true(),
                }],
                terminator: IRTerminator::Jump { target: 1 },
                successors: vec![1],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 1,
                instructions: Vec::new(),
                terminator: IRTerminator::Branch {
                    condition: IRCondition::Truthy {
                        value: cond,
                        negate: false,
                    },
                    target: 2,
                    fallthrough: 3,
                },
                successors: vec![2, 3],
                predecessors: vec![0, 2],
            },
            IRBlock {
                id: 2,
                instructions: vec![IRInst::Bytecode {
                    inst: decode_word(0, encode_raw(Opcode::GetProp2Ic, 1, 2, 3)),
                    uses: vec![used],
                    defs: vec![defined],
                }],
                terminator: IRTerminator::Jump { target: 1 },
                successors: vec![1],
                predecessors: vec![1],
            },
            IRBlock {
                id: 3,
                instructions: Vec::new(),
                terminator: IRTerminator::Return { value: None },
                successors: Vec::new(),
                predecessors: vec![1],
            },
        ],
        entry: 0,
        exit_blocks: vec![3],
        constants: Vec::new(),
    };

    let changed = LoopInvariantCodeMotion.run(&mut ir);

    assert!(!changed);
    assert!(matches!(
        ir.blocks[2].instructions.first(),
        Some(IRInst::Bytecode { .. })
    ));
}

#[test]
fn optimize_ir_runs_fixpoint_pipeline() {
    let input = IRValue::Register(0, 0);
    let check = IRValue::Register(1, 0);
    let result = IRValue::Register(2, 0);

    let mut ir = IRFunction {
        blocks: vec![
            IRBlock {
                id: 0,
                instructions: vec![
                    IRInst::LoadConst {
                        dst: input.clone(),
                        value: make_null(),
                    },
                    IRInst::Unary {
                        dst: check.clone(),
                        op: IRUnaryOp::IsNull,
                        operand: input,
                    },
                ],
                terminator: IRTerminator::Branch {
                    condition: IRCondition::Truthy {
                        value: check,
                        negate: false,
                    },
                    target: 1,
                    fallthrough: 2,
                },
                successors: vec![1, 2],
                predecessors: Vec::new(),
            },
            IRBlock {
                id: 1,
                instructions: vec![IRInst::LoadConst {
                    dst: result.clone(),
                    value: make_int32(1),
                }],
                terminator: IRTerminator::Return {
                    value: Some(result.clone()),
                },
                successors: Vec::new(),
                predecessors: vec![0],
            },
            IRBlock {
                id: 2,
                instructions: vec![IRInst::LoadConst {
                    dst: result.clone(),
                    value: make_int32(0),
                }],
                terminator: IRTerminator::Return {
                    value: Some(result.clone()),
                },
                successors: Vec::new(),
                predecessors: vec![0],
            },
        ],
        entry: 0,
        exit_blocks: vec![1, 2],
        constants: Vec::new(),
    };

    let changed = optimize_ir(&mut ir);

    assert!(changed);
    assert_eq!(ir.blocks.len(), 1);
    assert!(matches!(
        ir.blocks[0].terminator,
        IRTerminator::Return {
            value: Some(IRValue::Register(2, 0))
        } | IRTerminator::Return {
            value: Some(IRValue::Constant(_))
        }
    ));
}

#[test]
fn optimize_bytecode_runs_pure_ssa_pipeline() {
    let flag = IRValue::Register(0, 0);
    let copy = IRValue::Register(1, 0);

    let mut ir = IRFunction {
        blocks: vec![IRBlock {
            id: 0,
            instructions: vec![
                IRInst::LoadConst {
                    dst: flag.clone(),
                    value: make_int32(7),
                },
                IRInst::Mov {
                    dst: copy.clone(),
                    src: flag,
                },
            ],
            terminator: IRTerminator::Return { value: Some(copy) },
            successors: Vec::new(),
            predecessors: Vec::new(),
        }],
        entry: 0,
        exit_blocks: vec![0],
        constants: Vec::new(),
    };

    let changed = optimize_bytecode(&mut ir);

    assert!(changed);
    assert!(ir.blocks[0].instructions.len() <= 1);
}

#[test]
fn reuse_registers_linear_scan_compacts_register_ids() {
    fn max_register(ir: &IRFunction) -> u8 {
        let mut max_reg = 0;

        for block in &ir.blocks {
            for inst in &block.instructions {
                match inst {
                    IRInst::Phi { dst, incoming } => {
                        if let IRValue::Register(reg, _) = dst {
                            max_reg = max_reg.max(*reg);
                        }
                        for (_, value) in incoming {
                            if let IRValue::Register(reg, _) = value {
                                max_reg = max_reg.max(*reg);
                            }
                        }
                    }
                    IRInst::Mov { dst, src } => {
                        if let IRValue::Register(reg, _) = dst {
                            max_reg = max_reg.max(*reg);
                        }
                        if let IRValue::Register(reg, _) = src {
                            max_reg = max_reg.max(*reg);
                        }
                    }
                    IRInst::LoadConst { dst, .. } => {
                        if let IRValue::Register(reg, _) = dst {
                            max_reg = max_reg.max(*reg);
                        }
                    }
                    IRInst::Unary { dst, operand, .. } => {
                        if let IRValue::Register(reg, _) = dst {
                            max_reg = max_reg.max(*reg);
                        }
                        if let IRValue::Register(reg, _) = operand {
                            max_reg = max_reg.max(*reg);
                        }
                    }
                    IRInst::Binary { dst, lhs, rhs, .. } => {
                        if let IRValue::Register(reg, _) = dst {
                            max_reg = max_reg.max(*reg);
                        }
                        if let IRValue::Register(reg, _) = lhs {
                            max_reg = max_reg.max(*reg);
                        }
                        if let IRValue::Register(reg, _) = rhs {
                            max_reg = max_reg.max(*reg);
                        }
                    }
                    IRInst::Bytecode { .. } | IRInst::Nop => {}
                }
            }

            if let IRTerminator::Return {
                value: Some(IRValue::Register(reg, _)),
            } = &block.terminator
            {
                max_reg = max_reg.max(*reg);
            }
        }

        max_reg
    }

    let mut ir = IRFunction {
        blocks: vec![IRBlock {
            id: 0,
            instructions: vec![
                IRInst::LoadConst {
                    dst: IRValue::Register(10, 0),
                    value: make_int32(1),
                },
                IRInst::LoadConst {
                    dst: IRValue::Register(20, 0),
                    value: make_int32(2),
                },
                IRInst::Binary {
                    dst: IRValue::Register(30, 0),
                    op: IRBinaryOp::Add,
                    lhs: IRValue::Register(10, 0),
                    rhs: IRValue::Register(20, 0),
                },
            ],
            terminator: IRTerminator::Return {
                value: Some(IRValue::Register(30, 0)),
            },
            successors: Vec::new(),
            predecessors: Vec::new(),
        }],
        entry: 0,
        exit_blocks: vec![0],
        constants: Vec::new(),
    };

    let before = max_register(&ir);
    let changed = reuse_registers_linear_scan(&mut ir);
    let after = max_register(&ir);

    assert!(changed);
    assert!(after < before);
}

#[test]
fn optimize_to_bytecode_runs_ir_pipeline_before_lowering() {
    let flag = IRValue::Register(0, 0);
    let copy = IRValue::Register(1, 0);

    let ir = IRFunction {
        blocks: vec![IRBlock {
            id: 0,
            instructions: vec![
                IRInst::LoadConst {
                    dst: flag.clone(),
                    value: make_int32(7),
                },
                IRInst::Mov {
                    dst: copy.clone(),
                    src: flag,
                },
            ],
            terminator: IRTerminator::Return { value: Some(copy) },
            successors: Vec::new(),
            predecessors: Vec::new(),
        }],
        entry: 0,
        exit_blocks: vec![0],
        constants: Vec::new(),
    };

    let (baseline, baseline_constants) = ir.clone().into_bytecodes().expect("baseline bytecode");
    let (optimized, optimized_constants) = optimize_to_bytecode(&ir).expect("optimized bytecode");

    assert!(optimized.len() <= baseline.len());
    assert!(optimized_constants.len() <= baseline_constants.len());

    let last_pc = optimized.len() - 1;
    let last = decode_word(last_pc, optimized[last_pc]);
    assert!(matches!(
        last.opcode,
        Opcode::Ret | Opcode::RetReg | Opcode::RetU
    ));
}
