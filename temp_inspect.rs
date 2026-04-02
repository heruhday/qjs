use codegen::compile_source;
use disasm::disassemble_clean;
use vm::optimization;

fn main() {
    let src = r#"function inc(x){ return x + 1; } function pair(x){ return inc(x) + inc(x); } pair(20);"#;
    let compiled = compile_source(src).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    for line in disassemble_clean(&optimized.bytecode, &optimized.constants) {
        println!("{line}");
    }
}
