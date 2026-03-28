use codegen::compile_source;
use direct_optimization::{optimize_bytecode, optimize_compiled};

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
    // optimized.disassemble();
    assert!(!optimized.bytecode.is_empty());
}
