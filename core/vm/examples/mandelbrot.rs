use codegen::compile_source;
use vm::{VM, optimization};


fn main() {
    let compiled = compile_source(include_str!("../../../mandelbrot.js")).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);

    vm.run(false);
}