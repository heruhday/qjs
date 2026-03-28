use codegen::compile_source;
use vm::{VM, optimization, to_f64};

fn run_program(source: &str, optimized: bool) -> VM {
    let compiled = compile_source(source).expect("compile");
    let mut vm = VM::from_compiled(compiled, vec![]);
    vm.set_console_echo(false);
    vm.run(optimized);
    vm
}

fn run_compiled_program(source: &str) -> VM {
    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);
    vm
}

fn mandelbrot_test_source(width: u32, height: u32, max_iter: u32) -> String {
    include_str!("../../../mandelbrot.js").replace(
        "bench();",
        &format!("mandelbrot({width}, {height}, {max_iter});"),
    )
}

fn mandelbrot_bench_source(width: u32, height: u32, max_iter: u32) -> String {
    include_str!("../../../mandelbrot.js")
        .replace("const width = 400;", &format!("const width = {width};"))
        .replace("const height = 300;", &format!("const height = {height};"))
        .replace(
            "const maxIter = 200;",
            &format!("const maxIter = {max_iter};"),
        )
}

fn assert_fixed_time_line(line: &str) {
    assert!(line.starts_with("  time: "));
    assert!(line.ends_with(" ms"));

    let value = line
        .strip_prefix("  time: ")
        .and_then(|value| value.strip_suffix(" ms"))
        .expect("formatted time line");
    let Some((_, fractional)) = value.split_once('.') else {
        panic!("expected fixed-point millisecond output, got `{line}`");
    };
    assert_eq!(
        fractional.len(),
        3,
        "expected three fractional digits in `{line}`"
    );
}

#[test]
fn runs_simple_program() {
    let vm = run_program("1 + 2;", false);

    assert_eq!(to_f64(vm.frame.regs[255]), Some(3.0));
}

#[test]
fn runs_with_ssa_optimization() {
    let vm = run_program("let x = 1; x + 2; console.log(x);", true);

    assert_eq!(vm.console_output, vec!["1"]);
    assert_eq!(to_f64(vm.frame.regs[255]), None);
}

#[test]
fn optimized_vm_matches_unoptimized_on_branching_program() {
    let source = "let x = 1; if (x < 2) { x = x + 41; } else { x = x - 1; } x;";
    let baseline = run_program(source, false);
    let optimized = run_program(source, true);

    assert_eq!(optimized.console_output, baseline.console_output);
    assert_eq!(optimized.frame.regs[255], baseline.frame.regs[255]);
    assert_eq!(to_f64(optimized.frame.regs[255]), Some(42.0));
}

#[test]
fn optimized_vm_matches_unoptimized_on_property_access() {
    let source = "let obj = { answer: 40 }; obj.answer = obj.answer + 2; console.log(obj.answer); obj.answer;";
    let baseline = run_program(source, false);
    let optimized = run_program(source, true);

    assert_eq!(optimized.console_output, baseline.console_output);
    assert_eq!(optimized.frame.regs[255], baseline.frame.regs[255]);
    assert_eq!(optimized.console_output, vec!["42"]);
    assert_eq!(to_f64(optimized.frame.regs[255]), Some(42.0));
}

#[test]
fn optimized_vm_matches_unoptimized_on_dynamic_string_property_access() {
    let source =
        r#"let obj = {}; let key = "answer"; obj[key] = 40; obj[key] = obj[key] + 2; obj[key];"#;
    let baseline = run_program(source, false);
    let optimized = run_program(source, true);

    assert_eq!(optimized.console_output, baseline.console_output);
    assert_eq!(optimized.frame.regs[255], baseline.frame.regs[255]);
    assert_eq!(to_f64(optimized.frame.regs[255]), Some(42.0));
}

#[test]
fn optimized_vm_preserves_delete_on_shape_backed_properties() {
    let source = "let obj = { answer: 1 }; delete obj.answer; obj.answer;";
    let baseline = run_program(source, false);
    let optimized = run_program(source, true);

    assert_eq!(optimized.console_output, baseline.console_output);
    assert_eq!(optimized.frame.regs[255], baseline.frame.regs[255]);
    assert!(optimized.frame.regs[255].is_undefined());
}

#[test]
fn optimized_vm_preserves_nested_function_entries() {
    let source = "function add(a, b) { return a + b; } add(20, 22);";
    let baseline = run_program(source, false);
    let optimized = run_program(source, true);

    assert_eq!(optimized.console_output, baseline.console_output);
    assert_eq!(optimized.frame.regs[255], baseline.frame.regs[255]);
    assert_eq!(to_f64(optimized.frame.regs[255]), Some(42.0));
}

#[test]
fn optimize_compiled_uses_ssa_pipeline() {
    let compiled =
        compile_source("function add(a, b) { return a + b; } add(2, 3);").expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);

    vm.run(false);

    assert_eq!(to_f64(vm.frame.regs[255]), Some(5.0));
}

#[test]
fn run_test_js() {
    let compiled = compile_source(include_str!("../../.././test.js")).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);

    vm.run(false);
}

#[test]
fn run_test_fib() {
    let compiled = compile_source(include_str!("../../../fib_recursive.qjs")).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);

    vm.run(false);
}

#[test]
fn run_test_mandelbrot() {
    let compiled = compile_source(include_str!("../../../mandelbrot.js")).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);

    vm.run(false);
}

#[test]
fn run_test_mandelbrot2() {
    let source = mandelbrot_test_source(32, 24, 50);
    let baseline = run_program(&source, false);
    let optimized = run_program(&source, true);

    assert_eq!(optimized.console_output, baseline.console_output);
    assert_eq!(optimized.frame.regs[255], baseline.frame.regs[255]);
    assert_eq!(to_f64(optimized.frame.regs[255]), Some(121.0));
}

#[test]
fn while_loops_terminate() {
    let vm = run_program(
        "function sample() { let iter = 0; while (iter < 5) { iter++; } return iter; } sample();",
        false,
    );

    assert_eq!(to_f64(vm.frame.regs[255]), Some(5.0));
}

#[test]
fn number_to_fixed_builtin_formats_numbers() {
    let vm = run_program("console.log((1.2345).toFixed(2));", false);

    assert_eq!(vm.console_output, vec!["1.23"]);
}

#[test]
fn number_to_fixed_builtin_formats_variable_numbers() {
    let source = "let ms = 1.2345; console.log(ms.toFixed(2));";
    let baseline = run_program(source, false);
    let optimized = run_program(source, true);

    assert_eq!(baseline.console_output, vec!["1.23"]);
    assert_eq!(optimized.console_output, baseline.console_output);
}

#[test]
fn number_to_fixed_builtin_formats_const_numbers() {
    let source = "const ms = 1.2345; console.log(ms.toFixed(2));";
    let baseline = run_program(source, false);
    let optimized = run_program(source, true);

    assert_eq!(baseline.console_output, vec!["1.23"]);
    assert_eq!(optimized.console_output, baseline.console_output);
}

#[test]
fn number_to_fixed_builtin_formats_console_arguments() {
    let source = r#"const ms = 1003.573486328125; console.log("time:", ms.toFixed(3), "ms");"#;
    let baseline = run_program(source, false);
    let optimized = run_program(source, true);

    assert_eq!(baseline.console_output, vec!["time: 1003.573 ms"]);
    assert_eq!(optimized.console_output, baseline.console_output);
}

#[test]
fn number_to_fixed_builtin_formats_timestamp_deltas() {
    let source = r#"
        const t0 = Date.now();
        const t1 = t0 + 1003.573486328125;
        const ms = t1 - t0;
        console.log("time:", ms.toFixed(3), "ms");
    "#;
    let baseline = run_program(source, false);
    let optimized = run_program(source, true);

    assert_eq!(baseline.console_output, vec!["time: 1003.573 ms"]);
    assert_eq!(optimized.console_output, baseline.console_output);
}

#[test]
fn number_to_fixed_builtin_formats_function_local_values() {
    let source = r#"
        function bench() {
            const ms = 1003.573486328125;
            console.log("time:", ms.toFixed(3), "ms");
        }
        bench();
    "#;
    let baseline = run_program(source, false);
    let optimized = run_program(source, true);

    assert_eq!(baseline.console_output, vec!["time: 1003.573 ms"]);
    assert_eq!(optimized.console_output, baseline.console_output);
}

#[test]
fn number_to_fixed_builtin_formats_mandelbrot_bench_output() {
    let source = mandelbrot_bench_source(4, 3, 5);
    let baseline = run_program(&source, false);
    let optimized = run_compiled_program(&source);

    assert_eq!(baseline.console_output.len(), 5);
    assert_eq!(baseline.console_output[..4], optimized.console_output[..4]);
    assert_fixed_time_line(&baseline.console_output[4]);
    assert_fixed_time_line(&optimized.console_output[4]);
}
