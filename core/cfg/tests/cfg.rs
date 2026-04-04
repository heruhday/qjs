use cfg::{CFG, CFGError, CompareKind, Condition, Terminator};
use codegen::{CompiledBytecode, Opcode};
use value::{make_int32, make_number, make_true};

fn encode_raw(opcode: Opcode, a: u8, b: u8, c: u8) -> u32 {
    ((c as u32) << 24) | ((b as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

fn encode_asbx(opcode: Opcode, a: u8, sbx: i16) -> u32 {
    (((sbx as u16) as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

#[test]
fn builds_if_else_cfg() {
    let bytecode = vec![
        encode_raw(Opcode::LoadTrue, 0, 0, 0),
        encode_asbx(Opcode::JmpFalse, 0, 2),
        encode_asbx(Opcode::LoadI, 1, 1),
        encode_asbx(Opcode::Jmp, 0, 1),
        encode_asbx(Opcode::LoadI, 1, 2),
        encode_raw(Opcode::RetReg, 1, 0, 0),
    ];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");

    assert_eq!(cfg.blocks.len(), 4);
    assert_eq!(cfg.blocks[0].start_pc, 0);
    assert_eq!(cfg.blocks[1].start_pc, 2);
    assert_eq!(cfg.blocks[2].start_pc, 4);
    assert_eq!(cfg.blocks[3].start_pc, 5);

    match &cfg.blocks[0].terminator {
        Terminator::Branch {
            condition,
            target,
            fallthrough,
        } => {
            assert_eq!(
                *condition,
                Condition::Truthy {
                    reg: 0,
                    negate: true,
                }
            );
            assert_eq!(cfg.blocks[*target].start_pc, 4);
            assert_eq!(cfg.blocks[*fallthrough].start_pc, 2);
        }
        other => panic!("expected branch terminator, got {other:?}"),
    }
}

#[test]
fn decodes_switch_tables_from_constants() {
    let bytecode = vec![
        encode_raw(Opcode::Switch, 0, 0, 0),
        encode_asbx(Opcode::LoadI, 1, 10),
        encode_raw(Opcode::RetReg, 1, 0, 0),
        encode_asbx(Opcode::LoadI, 1, 20),
        encode_raw(Opcode::RetReg, 1, 0, 0),
    ];
    let constants = vec![make_int32(1), make_int32(3), make_true(), make_int32(1)];

    let cfg = CFG::from_parts(bytecode, constants, 0).expect("cfg");

    assert_eq!(cfg.blocks.len(), 3);

    match &cfg.blocks[0].terminator {
        Terminator::Switch {
            key,
            cases,
            default_target,
        } => {
            assert_eq!(*key, 0);
            assert_eq!(cases.len(), 1);
            assert_eq!(cfg.blocks[cases[0].target].start_pc, 2);
            assert_eq!(cfg.blocks[*default_target].start_pc, 4);
        }
        other => panic!("expected switch terminator, got {other:?}"),
    }
}

#[test]
fn classifies_loop_inc_as_compare_branch() {
    let bytecode = vec![
        encode_asbx(Opcode::LoadI, 0, 0),
        encode_asbx(Opcode::LoadI, 255, 10),
        encode_asbx(Opcode::LoopIncJmp, 0, -2),
        encode_raw(Opcode::RetReg, 0, 0, 0),
    ];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");
    let loop_block = cfg.block_containing_pc(2).expect("loop block");

    match &cfg.blocks[loop_block].terminator {
        Terminator::Branch { condition, .. } => {
            assert_eq!(
                *condition,
                Condition::Compare {
                    kind: CompareKind::Lt,
                    lhs: 0,
                    rhs: 255,
                    negate: false,
                }
            );
        }
        other => panic!("expected compare branch, got {other:?}"),
    }
}

#[test]
fn block_for_pc_covers_every_instruction_in_block() {
    let bytecode = vec![
        encode_raw(Opcode::LoadTrue, 0, 0, 0),
        encode_asbx(Opcode::JmpFalse, 0, 2),
        encode_asbx(Opcode::LoadI, 1, 1),
        encode_asbx(Opcode::Jmp, 0, 1),
        encode_asbx(Opcode::LoadI, 1, 2),
        encode_raw(Opcode::RetReg, 1, 0, 0),
    ];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");

    assert_eq!(cfg.block_for_pc(0), Some(0));
    assert_eq!(cfg.block_for_pc(1), Some(0));
    assert_eq!(cfg.block_for_pc(2), Some(1));
    assert_eq!(cfg.block_for_pc(3), Some(1));
    assert_eq!(cfg.block_for_pc(4), Some(2));
    assert_eq!(cfg.block_for_pc(5), Some(3));
}

#[test]
fn dead_end_block_is_not_reported_as_exit() {
    let bytecode = vec![
        encode_asbx(Opcode::LoadI, 0, 7),
        encode_raw(Opcode::Mov, 1, 0, 0),
    ];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");

    assert_eq!(cfg.blocks.len(), 1);
    assert!(matches!(cfg.blocks[0].terminator, Terminator::None));
    assert!(cfg.exit_blocks.is_empty());
}

#[test]
fn decodes_try_terminator_and_successors() {
    let bytecode = vec![
        encode_asbx(Opcode::Try, 0, 1),
        encode_raw(Opcode::RetU, 0, 0, 0),
        encode_raw(Opcode::Throw, 0, 0, 0),
    ];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");

    assert_eq!(cfg.blocks.len(), 3);
    match &cfg.blocks[0].terminator {
        Terminator::Try {
            handler,
            fallthrough,
        } => {
            assert_eq!(cfg.blocks[*handler].start_pc, 2);
            assert_eq!(cfg.blocks[*fallthrough].start_pc, 1);
        }
        other => panic!("expected try terminator, got {other:?}"),
    }
}

#[test]
fn classifies_conditional_return_as_exit() {
    let bytecode = vec![
        encode_raw(Opcode::RetIfLteI, 1, 2, 3),
        encode_raw(Opcode::RetU, 0, 0, 0),
    ];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");

    assert_eq!(cfg.blocks.len(), 2);
    assert!(cfg.exit_blocks.contains(&0));
    assert!(cfg.exit_blocks.contains(&1));
    match &cfg.blocks[0].terminator {
        Terminator::ConditionalReturn {
            condition,
            value,
            fallthrough,
        } => {
            assert_eq!(
                *condition,
                Condition::Compare {
                    kind: CompareKind::Lte,
                    lhs: 1,
                    rhs: 2,
                    negate: false,
                }
            );
            assert_eq!(*value, 3);
            assert_eq!(cfg.blocks[*fallthrough].start_pc, 1);
        }
        other => panic!("expected conditional return, got {other:?}"),
    }
}

#[test]
fn classifies_explicit_compare_branch_kinds() {
    let neq_bytecode = vec![
        encode_raw(Opcode::JmpNeq, 0, 1, 1),
        encode_raw(Opcode::RetU, 0, 0, 0),
        encode_raw(Opcode::RetU, 0, 0, 0),
    ];
    let neq_cfg = CFG::from_parts(neq_bytecode, Vec::new(), 0).expect("neq cfg");

    assert!(matches!(
        neq_cfg.blocks[0].terminator,
        Terminator::Branch {
            condition: Condition::Compare {
                kind: CompareKind::Neq,
                lhs: 0,
                rhs: 1,
                negate: false,
            },
            ..
        }
    ));

    let lte_false_bytecode = vec![
        encode_raw(Opcode::JmpLteFalse, 1, 2, 1),
        encode_raw(Opcode::RetU, 0, 0, 0),
        encode_raw(Opcode::RetU, 0, 0, 0),
    ];
    let lte_false_cfg = CFG::from_parts(lte_false_bytecode, Vec::new(), 0).expect("lte false cfg");

    assert!(matches!(
        lte_false_cfg.blocks[0].terminator,
        Terminator::Branch {
            condition: Condition::Compare {
                kind: CompareKind::LteFalse,
                lhs: 1,
                rhs: 2,
                negate: false,
            },
            ..
        }
    ));
}

#[test]
fn classifies_call_return_as_exit() {
    let bytecode = vec![encode_raw(Opcode::CallRet, 4, 2, 0)];

    let cfg = CFG::from_parts(bytecode, Vec::new(), 0).expect("cfg");

    assert_eq!(cfg.blocks.len(), 1);
    assert_eq!(cfg.exit_blocks, vec![0]);
    assert!(matches!(
        cfg.blocks[0].terminator,
        Terminator::CallReturn { callee: 4, argc: 2 }
    ));
}

#[test]
fn reports_invalid_branch_targets() {
    let bytecode = vec![encode_asbx(Opcode::Jmp, 0, 5)];

    let error = CFG::from_parts(bytecode, Vec::new(), 0).expect_err("invalid branch");

    assert!(matches!(
        error,
        CFGError::InvalidBranchTarget {
            pc: 0,
            target_pc: 6,
            len: 1
        }
    ));
}

#[test]
fn function_entries_include_nested_function_offsets() {
    let compiled = CompiledBytecode {
        bytecode: vec![
            encode_raw(Opcode::RetU, 0, 0, 0),
            encode_raw(Opcode::RetU, 0, 0, 0),
            encode_raw(Opcode::RetU, 0, 0, 0),
            encode_raw(Opcode::RetU, 0, 0, 0),
            encode_raw(Opcode::RetU, 0, 0, 0),
        ],
        constants: vec![make_number(2.0), make_number(4.0)],
        string_constants: Vec::new(),
        atom_constants: Vec::new(),
        function_constants: vec![0, 1],
        names: Vec::new(),
        properties: Vec::new(),
        private_properties: Vec::new(),
    };

    assert_eq!(CFG::function_entries(&compiled), vec![0, 2, 4]);
}
