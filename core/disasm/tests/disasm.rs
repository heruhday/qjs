use codegen::compile_source;
use disasm::{disassemble_compiled, disassemble_compiled_clean};

#[test]
fn disassembles_compiled_bytecode() {
    let compiled = compile_source("1 + 2;").expect("source should compile");
    let asm = disassemble_compiled(&compiled);
    println!("{:#?}", asm);
    assert!(!asm.is_empty());
}

#[test]
fn disassembles_clean_output() {
    let compiled = compile_source("const answer = 40 + 2; answer;").expect("source should compile");
    let asm = disassemble_compiled_clean(&compiled);
    println!("{:#?}", asm);
    assert!(!asm.is_empty());
}
