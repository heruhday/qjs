use codegen::{Opcode, compile_source};
use direct_optimization::{eliminate_dead_code, optimize_bytecode, optimize_compiled};
use disasm::disassemble_clean;
use value::make_number;

fn encode_abc(opcode: Opcode, a: u8, b: u8, c: u8) -> u32 {
    ((c as u32) << 24) | ((b as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

fn encode_abx(opcode: Opcode, a: u8, bx: u16) -> u32 {
    ((bx as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

fn encode_asbx(opcode: Opcode, a: u8, sbx: i16) -> u32 {
    encode_abx(opcode, a, sbx as u16)
}

#[test]
fn optimizes_bytecode_from_codegen() {
    let compiled = compile_source("1 + 2;").expect("compile");
    let (bytecode, constants) = optimize_bytecode(compiled.bytecode.clone(), compiled.constants);
    assert!(!bytecode.is_empty());
    assert!(constants.len() <= compiled.bytecode.len());
}

#[test]
fn optimizes_compiled_program() {
    let compiled = compile_source("let x = 1; x + 2;").expect("compile");
    let optimized = optimize_compiled(compiled);
    assert!(!optimized.bytecode.is_empty());
}

#[test]
fn optimize_bytecode_reuses_name_write_value_for_zero_arg_call() {
    let bytecode = vec![
        encode_abx(Opcode::NewFunc, 1, 0),
        encode_abc(Opcode::SetUpval, 1, 0, 0),
        encode_abx(Opcode::InitName, 1, 0),
        encode_abx(Opcode::LoadName, 1, 0),
        encode_abc(Opcode::CallRet, 1, 0, 0),
    ];
    let (bytecode, constants) = optimize_bytecode(bytecode, vec![make_number(0.0)]);
    let asm = disassemble_clean(&bytecode, &constants);

    assert_eq!(
        asm,
        vec![
            "new_func r1, const[0]",
            "set_upval r1",
            "init_name r1, identifier[0]",
            "call_ret r1, 0",
        ]
    );
}

#[test]
fn optimize_bytecode_folds_acc_move_into_name_write_and_return() {
    let bytecode = vec![
        encode_abx(Opcode::LoadName, 1, 0),
        encode_abc(Opcode::Inc, 1, 0, 0),
        encode_abc(Opcode::Mov, 1, 255, 0),
        encode_abx(Opcode::StoreName, 1, 0),
        encode_abx(Opcode::LoadName, 1, 0),
        encode_abc(Opcode::RetReg, 1, 0, 0),
    ];
    let (bytecode, constants) = optimize_bytecode(bytecode, Vec::new());
    let asm = disassemble_clean(&bytecode, &constants);

    assert!(
        !asm.iter().any(|line| line.starts_with("mov ")),
        "expected ACC copy into name write to fold away:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter()
            .any(|line| line == "store_name r255, identifier[0]"),
        "expected store_name to write ACC directly:\n{}",
        asm.join("\n")
    );
    assert_eq!(asm.last().map(String::as_str), Some("ret_reg r255"));
}

#[test]
fn optimize_bytecode_constant_folds_obvious_integer_ops() {
    let bytecode = vec![
        encode_asbx(Opcode::LoadI, 1, 1),
        encode_abc(Opcode::Add, 0, 1, 1),
        encode_asbx(Opcode::LoadI, 3, 2),
        encode_abc(Opcode::MulAcc, 0, 3, 0),
        encode_abc(Opcode::Ret, 0, 0, 0),
    ];
    let (bytecode, constants) = optimize_bytecode(bytecode, Vec::new());
    let asm = disassemble_clean(&bytecode, &constants);

    assert!(
        asm.iter().any(|line| line == "load_i r255, 4"),
        "expected local integer arithmetic to fold to a constant:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("add ")),
        "expected folded arithmetic to remove add:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("mul_acc ")),
        "expected folded arithmetic to remove mul_acc:\n{}",
        asm.join("\n")
    );
}

#[test]
fn optimize_bytecode_constant_folds_integer_comparisons() {
    let bytecode = vec![
        encode_asbx(Opcode::LoadI, 1, 2),
        encode_asbx(Opcode::LoadI, 2, 1),
        encode_abc(Opcode::StrictEq, 0, 1, 2),
        encode_asbx(Opcode::LoadI, 3, 5),
        encode_asbx(Opcode::LoadI, 4, 10),
        encode_abc(Opcode::Lt, 0, 3, 4),
        encode_abc(Opcode::Ret, 0, 0, 0),
    ];
    let (bytecode, constants) = optimize_bytecode(bytecode, Vec::new());
    let asm = disassemble_clean(&bytecode, &constants);

    assert!(
        asm.iter().any(|line| line == "load_false"),
        "expected strict_eq of known ints to fold to false:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter().any(|line| line == "load_true"),
        "expected lt of known ints to fold to true:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("strict_eq ")),
        "expected folded comparison to remove strict_eq:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("lt ")),
        "expected folded comparison to remove lt:\n{}",
        asm.join("\n")
    );
}

#[test]
fn optimize_bytecode_fuses_recursive_sub_calls_into_single_superinstruction() {
    let bytecode = vec![
        encode_abc(Opcode::Call1SubI, 2, 1, 1),
        encode_abc(Opcode::Mov, 3, 255, 0),
        encode_abc(Opcode::Call1SubI, 2, 1, 2),
        encode_abc(Opcode::Add, 0, 3, 255),
        encode_abc(Opcode::RetReg, 255, 0, 0),
    ];
    let (bytecode, constants) = optimize_bytecode(bytecode, Vec::new());
    let asm = disassemble_clean(&bytecode, &constants);

    assert!(
        asm.iter().any(|line| line == "call2_sub_i_add r2, r1, 1"),
        "expected recursive sub-call pattern to fuse:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("mov ")),
        "expected accumulator shuffle to disappear:\n{}",
        asm.join("\n")
    );
    assert_eq!(asm.last().map(String::as_str), Some("ret_reg r255"));
}

#[test]
fn optimize_compiled_inlines_immediately_invoked_root_function() {
    let compiled = compile_source(
        "function normalize_suite() { let a = 1; let b = a + 1; console.log(b); } normalize_suite();",
    )
    .expect("compile");
    let optimized = optimize_compiled(compiled);
    let asm = disassemble_clean(&optimized.bytecode, &optimized.constants);

    assert!(
        !asm.iter().any(|line| line.starts_with("new_func ")),
        "expected direct optimizer to inline the immediate root function:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("call_ret ")),
        "expected direct optimizer to inline away the root call:\n{}",
        asm.join("\n")
    );
}

#[test]
fn optimize_compiled_minimizes_trivial_normalize_suite() {
    let compiled = compile_source(
        "function normalize_suite() { let a = 1; let b = a + 1; console.log(b); } normalize_suite();",
    )
    .expect("compile");
    let optimized = optimize_compiled(compiled);
    let asm = disassemble_clean(&optimized.bytecode, &optimized.constants);

    assert_eq!(asm.len(), 4, "expected 4 instructions:\n{}", asm.join("\n"));
    assert!(
        asm.iter().any(|line| line == "ret_u"),
        "expected ret_u terminator:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter().any(|line| line.starts_with("load_name ")),
        "expected console load to remain:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter().any(|line| line.ends_with(", 2")),
        "expected folded constant `2`:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter().any(|line| line.starts_with("call_method1 ")),
        "expected a single method call to remain:\n{}",
        asm.join("\n")
    );
}

#[test]
fn optimize_bytecode_removes_dead_acc_constant_before_call_method1() {
    let bytecode = vec![
        encode_asbx(Opcode::LoadI, 255, 2),
        encode_asbx(Opcode::LoadI, 4, 2),
        encode_abx(Opcode::LoadName, 3, 0),
        encode_abx(Opcode::CallMethod1, 3, 0),
        encode_abc(Opcode::RetU, 0, 0, 0),
    ];
    let (bytecode, constants) = optimize_bytecode(bytecode, Vec::new());
    let asm = disassemble_clean(&bytecode, &constants);

    assert_eq!(
        asm,
        vec![
            "load_i r4, 2",
            "load_name r3, identifier[0]",
            "call_method1 r3, property[0], r4",
            "ret_u",
        ],
        "expected dead ACC constant to be removed:\n{}",
        asm.join("\n")
    );
}

#[test]
fn eliminate_dead_code_removes_dead_acc_constant_before_call_method1() {
    let bytecode = vec![
        encode_asbx(Opcode::LoadI, 255, 2),
        encode_asbx(Opcode::LoadI, 4, 2),
        encode_abx(Opcode::LoadName, 3, 0),
        encode_abx(Opcode::CallMethod1, 3, 0),
        encode_abc(Opcode::RetU, 0, 0, 0),
    ];
    let (bytecode, constants) = eliminate_dead_code(bytecode, Vec::new());
    let asm = disassemble_clean(&bytecode, &constants);

    assert_eq!(
        asm,
        vec![
            "load_i r4, 2",
            "load_name r3, identifier[0]",
            "call_method1 r3, property[0], r4",
            "ret_u",
        ],
        "expected eliminate_dead_code to remove the dead ACC constant:\n{}",
        asm.join("\n")
    );
}
