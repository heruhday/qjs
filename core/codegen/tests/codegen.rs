use ast::parse;
use codegen::{compile_program, compile_source};

#[test]
fn compiles_simple_expression_source() {
    let compiled = compile_source("1 + 2;").expect("source should compile");
    assert!(!compiled.bytecode.is_empty());
}

#[test]
fn compiles_program_ast() {
    let program = parse("const answer = 40 + 2; answer;").expect("program should parse");
    let compiled = compile_program(&program).expect("program should compile");

    assert!(!compiled.bytecode.is_empty());
    assert!(!compiled.names.is_empty());
}
