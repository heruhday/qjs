use codegen::{Opcode, compile_source};
use disasm::{disassemble, disassemble_clean, disassemble_compiled_clean};
use value::{JSValue, make_number};
use vm::optimization::{optimize_bytecode, optimize_compiled};

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
fn disassembles_optimized_arithmetic_and_call_shapes_cleanly() {
    let bytecode = vec![
        encode_abc(Opcode::LoadAdd, 1, 2, 3),
        encode_abc(Opcode::LoadInc, 4, 5, 0),
        encode_abc(Opcode::LoadDec, 6, 7, 0),
        encode_abc(Opcode::CallRet, 8, 2, 0),
    ];
    let asm = disassemble_clean(&bytecode, &Vec::<JSValue>::new());

    assert_eq!(asm[0], "load_add r1, r2, r3");
    assert_eq!(asm[1], "load_inc r4, r5");
    assert_eq!(asm[2], "load_dec r6, r7");
    assert_eq!(asm[3], "call_ret r8, 2");
}

#[test]
fn disassembles_optimized_jump_opcodes_with_targets() {
    let bytecode = vec![
        encode_abc(Opcode::LoadJfalse, 4, 2, 0),
        encode_abc(Opcode::LoadCmpEqJfalse, 1, 2, (-2_i8) as u8),
        encode_abc(Opcode::LoadCmpLtJfalse, 3, 5, 1),
        encode_abc(Opcode::JmpLtF64, 6, 7, 2),
        encode_abc(Opcode::JmpLteF64, 8, 9, (-1_i8) as u8),
        encode_abc(Opcode::JmpLteFalseF64, 10, 11, 3),
    ];
    let asm = disassemble(&bytecode, &Vec::<JSValue>::new());
    let clean = disassemble_clean(&bytecode, &Vec::<JSValue>::new());

    assert_eq!(asm[0], "0000: load_jfalse r4, -> 000C");
    assert_eq!(asm[1], "0004: load_cmp_eq_jfalse r1, r2, -> 0000");
    assert_eq!(asm[2], "0008: load_cmp_lt_jfalse r3, r5, -> 0010");
    assert_eq!(asm[3], "000C: jmp_lt_f64 r6, r7, -> 0018");
    assert_eq!(asm[4], "0010: jmp_lte_f64 r8, r9, -> 0010");
    assert_eq!(asm[5], "0014: jmp_lte_false_f64 r10, r11, -> 0024");

    assert_eq!(clean[0], "load_jfalse r4, -> L2");
    assert_eq!(clean[1], "load_cmp_eq_jfalse r1, r2, -> L-2");
    assert_eq!(clean[2], "load_cmp_lt_jfalse r3, r5, -> L1");
    assert_eq!(clean[3], "jmp_lt_f64 r6, r7, -> L2");
    assert_eq!(clean[4], "jmp_lte_f64 r8, r9, -> L-1");
    assert_eq!(clean[5], "jmp_lte_false_f64 r10, r11, -> L3");
}

#[test]
fn disassembles_numeric_compare_opcodes() {
    let bytecode = vec![
        encode_abc(Opcode::LtF64, 1, 2, 3),
        encode_abc(Opcode::LteF64, 4, 5, 6),
    ];
    let asm = disassemble_clean(&bytecode, &Vec::<JSValue>::new());

    assert_eq!(asm[0], "lt_f64 r1, r2, r3");
    assert_eq!(asm[1], "lte_f64 r4, r5, r6");
}

#[test]
fn disassembles_optimized_property_access_opcodes() {
    let bytecode = vec![
        encode_abc(Opcode::LoadGetProp, 2, 9, 0),
        encode_abc(Opcode::LoadGetPropCmpEq, 2, 9, 3),
        encode_abc(Opcode::GetProp2Ic, 5, 6, 7),
        encode_abc(Opcode::GetProp3Ic, 8, 9, 10),
        encode_abc(Opcode::CallMethodIc, 11, 12, 13),
        encode_abc(Opcode::CallMethod2Ic, 14, 15, 16),
    ];
    let asm = disassemble_clean(&bytecode, &Vec::<JSValue>::new());

    assert_eq!(asm[0], "load_get_prop r2, r9, r0");
    assert_eq!(asm[1], "load_get_prop_cmp_eq r2, r9, r3");
    assert_eq!(asm[2], "get_prop2_ic r5, r6, r7");
    assert_eq!(asm[3], "get_prop3_ic r8, r9, r10");
    assert_eq!(asm[4], "call_method_ic r11, r12, r13");
    assert_eq!(asm[5], "call_method2_ic r14, r15, r16");
}

#[test]
fn optimizes_compiled_program() {
    let compiled = compile_source("let x = 1; x + 2;").expect("compile");
    let optimized = optimize_compiled(compiled);
    let optimized_asm = disassemble_clean(&optimized.bytecode, &optimized.constants);
    println!("Optimized bytecode:\n{}", optimized_asm.join("\n"));
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
        "expected immediate root function declaration to inline away, got:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("call_ret ")),
        "expected immediate root function call to inline away, got:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter().any(|line| line.starts_with("call_method1 ")),
        "expected inlined root body to keep the console.log side effect, got:\n{}",
        asm.join("\n")
    );
    assert_eq!(asm.last().map(String::as_str), Some("ret_u"));
}

#[test]
fn while_false_branch_skips_loop_backedge() {
    let compiled = compile_source(
        "function sample() { let iter = 0; while (iter < 5) { iter++; } return iter; } sample();",
    )
    .expect("compile");
    let asm = disassemble_compiled_clean(&compiled);

    assert!(
        asm.iter().any(|line| line == "jmp_false r2, -> L4"),
        "expected while exit jump to skip the backedge, got:\n{}",
        asm.join("\n")
    );
}

#[test]
fn optimize_bytecode_preserves_numeric_loop_shape() {
    let bytecode = vec![
        encode_asbx(Opcode::LoadI, 1, 0),
        encode_abx(Opcode::LoadK, 2, 0),
        encode_abx(Opcode::LoadK, 3, 1),
        encode_abc(Opcode::Add, 0, 1, 2),
        encode_abc(Opcode::Mov, 1, 255, 0),
        encode_abc(Opcode::JmpLte, 1, 3, (-3_i8) as u8),
        encode_abc(Opcode::RetReg, 1, 0, 0),
    ];
    let constants = vec![make_number(1.5), make_number(10.0)];
    let (optimized_bytecode, optimized_constants) = optimize_bytecode(bytecode, constants);
    let asm = disassemble_clean(&optimized_bytecode, &optimized_constants);

    assert!(
        asm.iter().any(|line| line.starts_with("ret_reg r")),
        "expected optimized loop to keep its return, got:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter()
            .any(|line| line.contains("jmp_lte") || line.contains("jmp_lte_f64")),
        "expected optimized loop to keep its loop branch, got:\n{}",
        asm.join("\n")
    );
}

#[test]
fn optimize_bytecode_applies_direct_superinstruction_cleanup_after_ssa() {
    let bytecode = vec![
        encode_abc(Opcode::GetProp, 1, 0, 2),
        encode_abc(Opcode::GetProp, 2, 1, 3),
        encode_abc(Opcode::RetReg, 2, 0, 0),
    ];
    let (optimized_bytecode, optimized_constants) = optimize_bytecode(bytecode, Vec::new());
    let asm = disassemble_clean(&optimized_bytecode, &optimized_constants);

    assert!(
        asm.iter().any(|line| line.starts_with("get_prop2_ic ")),
        "expected direct superinstruction fusion after SSA, got:\n{}",
        asm.join("\n")
    );
    assert_eq!(asm.last().map(String::as_str), Some("ret_reg r2"));
}

#[test]
fn optimize_bytecode_specializes_fixed_arity_calls() {
    let bytecode = vec![
        encode_abc(Opcode::Call, 4, 2, 0),
        encode_abc(Opcode::RetU, 0, 0, 0),
    ];
    let (optimized_bytecode, optimized_constants) = optimize_bytecode(bytecode, Vec::new());
    let asm = disassemble_clean(&optimized_bytecode, &optimized_constants);

    assert_eq!(asm[0], "call2 r4, r5, r6");
    assert_eq!(asm[1], "ret_u");
}

#[test]
fn optimize_bytecode_fuses_call_followed_by_return() {
    let bytecode = vec![
        encode_abc(Opcode::Call, 7, 0, 0),
        encode_abc(Opcode::Ret, 0, 0, 0),
    ];
    let (optimized_bytecode, optimized_constants) = optimize_bytecode(bytecode, Vec::new());
    let asm = disassemble_clean(&optimized_bytecode, &optimized_constants);

    assert_eq!(asm, vec!["call_ret r7, 0"]);
}

#[test]
fn optimize_bytecode_removes_dead_call_result_copies_across_name_and_method_ops() {
    let bytecode = vec![
        encode_abc(Opcode::Call2, 4, 5, 6),
        encode_abc(Opcode::Mov, 4, 255, 0),
        encode_abx(Opcode::LoadName, 7, 0),
        encode_abx(Opcode::LoadK, 8, 0),
        encode_abx(Opcode::CallMethod1, 7, 0),
        encode_abc(Opcode::RetU, 0, 0, 0),
    ];
    let constants = vec![make_number(1.0)];
    let (optimized_bytecode, optimized_constants) = optimize_bytecode(bytecode, constants);
    let asm = disassemble_clean(&optimized_bytecode, &optimized_constants);

    assert!(
        !asm.iter().any(|line| line.starts_with("mov ")),
        "expected dead ACC copy after call2 to be removed, got:\n{}",
        asm.join("\n")
    );
    assert!(asm.iter().any(|line| line.starts_with("call2 ")));
    assert!(asm.iter().any(|line| line.starts_with("call_method1 ")));
}

#[test]
fn optimize_bytecode_propagates_callee_alias_into_explicit_call2() {
    let bytecode = vec![
        encode_abc(Opcode::Mov, 7, 3, 0),
        encode_abx(Opcode::LoadK, 8, 0),
        encode_abx(Opcode::NewFunc, 9, 1),
        encode_abc(Opcode::Call2, 7, 8, 9),
        encode_abc(Opcode::RetU, 0, 0, 0),
    ];
    let constants = vec![make_number(1.0), make_number(2.0)];
    let (optimized_bytecode, optimized_constants) = optimize_bytecode(bytecode, constants);
    let asm = disassemble_clean(&optimized_bytecode, &optimized_constants);

    assert!(
        !asm.iter().any(|line| line.starts_with("mov ")),
        "expected callee copy to fold into call2, got:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter().any(|line| line.starts_with("call2 ")),
        "expected explicit call2 to remain after folding the callee copy, got:\n{}",
        asm.join("\n")
    );
}

#[test]
fn optimize_bytecode_retargets_simple_method_arg_builder_into_call_bundle() {
    let bytecode = vec![
        encode_asbx(Opcode::LoadI, 1, 2),
        encode_abx(Opcode::LoadName, 3, 0),
        encode_abc(Opcode::Mov, 4, 1, 0),
        encode_abx(Opcode::CallMethod1, 3, 0),
        encode_abc(Opcode::RetU, 0, 0, 0),
    ];
    let (optimized_bytecode, optimized_constants) = optimize_bytecode(bytecode, Vec::new());
    let asm = disassemble_clean(&optimized_bytecode, &optimized_constants);

    assert!(
        !asm.iter().any(|line| line == "mov r4, r1, r0"),
        "expected method argument copy to fold into the call bundle, got:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter().any(|line| line == "load_i r4, 2"),
        "expected the argument builder to retarget into the bundle slot, got:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter()
            .any(|line| line == "call_method1 r3, property[0], r4"),
        "expected call_method1 to remain with the packed bundle, got:\n{}",
        asm.join("\n")
    );
}

#[test]
fn optimize_bytecode_reruns_copy_prop_after_call_specialization() {
    let bytecode = vec![
        encode_abc(Opcode::LoadTrue, 0, 0, 0),
        encode_abc(Opcode::Mov, 3, 255, 0),
        encode_abx(Opcode::LoadK, 4, 0),
        encode_abc(Opcode::Call, 2, 2, 0),
        encode_abc(Opcode::RetU, 0, 0, 0),
    ];
    let constants = vec![make_number(1.0)];
    let (optimized_bytecode, optimized_constants) = optimize_bytecode(bytecode, constants);
    let asm = disassemble_clean(&optimized_bytecode, &optimized_constants);

    assert!(
        !asm.iter().any(|line| line == "mov r3, r255, r0"),
        "expected ACC copy to disappear after call specialization cleanup, got:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter()
            .any(|line| line.starts_with("call2 ") && line.contains("r255")),
        "expected specialized call2 to consume ACC directly, got:\n{}",
        asm.join("\n")
    );
}

#[test]
fn optimize_bytecode_removes_dead_acc_copy_inside_try_region() {
    let bytecode = vec![
        encode_asbx(Opcode::Try, 0, 4),
        encode_abc(Opcode::Call0, 1, 0, 0),
        encode_abc(Opcode::Mov, 2, 255, 0),
        encode_abc(Opcode::EndTry, 0, 0, 0),
        encode_asbx(Opcode::Jmp, 0, 5),
        encode_abx(Opcode::Enter, 0, 1),
        encode_abx(Opcode::CreateEnv, 3, 0),
        encode_abc(Opcode::Catch, 4, 0, 0),
        encode_abc(Opcode::Leave, 0, 0, 0),
        encode_abc(Opcode::Finally, 0, 0, 0),
        encode_abc(Opcode::RetU, 0, 0, 0),
    ];
    let (optimized_bytecode, optimized_constants) = optimize_bytecode(bytecode, Vec::new());
    let asm = disassemble_clean(&optimized_bytecode, &optimized_constants);

    assert!(
        !asm.iter().any(|line| line == "mov r2, r255, r0"),
        "expected dead ACC copy in try body to be removed, got:\n{}",
        asm.join("\n")
    );
}
