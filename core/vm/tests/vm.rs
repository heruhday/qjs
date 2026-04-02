#[path = "../../ssa/tests/support/js_suite_support.rs"]
mod js_suite_support;

use cfg::CFG;
use codegen::{CompiledBytecode, Opcode, compile_source};
use disasm::disassemble_clean;
use ssa::{build_ssa, optimize_to_bytecode};
use std::path::PathBuf;
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

fn encode_raw(opcode: Opcode, a: u8, b: u8, c: u8) -> u32 {
    ((c as u32) << 24) | ((b as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

fn encode_asbx(opcode: Opcode, a: u8, sbx: i16) -> u32 {
    (((sbx as u16) as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
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

fn repo_root_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(relative)
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
fn optimize_compiled_matches_ssa_optimizer_for_pure_ir_segments() {
    let compiled = CompiledBytecode {
        bytecode: vec![
            encode_asbx(Opcode::LoadI, 0, 7),
            encode_raw(Opcode::Mov, 1, 0, 0),
            encode_raw(Opcode::RetReg, 1, 0, 0),
        ],
        constants: Vec::new(),
        string_constants: Vec::new(),
        function_constants: Vec::new(),
        names: Vec::new(),
        properties: Vec::new(),
    };

    let cfg =
        CFG::from_parts(compiled.bytecode.clone(), compiled.constants.clone(), 0).expect("cfg");
    let ir = build_ssa(cfg, usize::from(u8::MAX) + 1).to_ir();
    let (expected_bytecode, expected_constants) =
        optimize_to_bytecode(&ir).expect("optimized bytecode");

    let optimized = optimization::optimize_compiled(compiled.clone());

    assert_eq!(optimized.bytecode, expected_bytecode);
    assert_eq!(optimized.constants, expected_constants);
    assert!(optimized.bytecode.len() < compiled.bytecode.len());
}

#[test]
fn run_test_js() {
    let compiled = compile_source(include_str!("../../.././test.js")).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);

    vm.run(false);
}

#[test]
fn run_test_simple() -> std::io::Result<()> {
    let content = js_suite_support::combined_normalized_suites();
    let compiled = compile_source(&content).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    assert!(
        !optimized.bytecode.is_empty(),
        "expected combined SSA suite to produce bytecode"
    );
    let out = format!(
        "Optimized bytecode:\n{}",
        disassemble_clean(&optimized.bytecode, &optimized.constants).join("\n")
    );
    std::fs::write(repo_root_path("core/ssa/tests/js/all.bc"), &out)?;
    Ok(())
}

#[test]
fn optimized_mixed_bytecode_tracks_named_object_properties() {
    let compiled =
        compile_source("Test = {}; Test.x = 2 + 3; res = Test.x === 5; console.log(res);")
            .expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let asm = disassemble_clean(&optimized.bytecode, &optimized.constants);

    assert!(
        asm.iter().any(|line| line == "load_true"),
        "expected comparison to fold to a boolean constant:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter().any(|line| line.starts_with("set_prop ")),
        "expected property store to remain:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("get_prop ")),
        "expected property load to be forwarded:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("strict_eq ")),
        "expected strict equality to fold:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("mov ")),
        "expected redundant moves to be removed:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter()
            .any(|line| line.starts_with("load_name ") && line.contains("identifier[0]")),
        "expected `Test` reloads to be removed:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter()
            .any(|line| line.starts_with("load_name ") && line.contains("identifier[1]")),
        "expected `res` reloads to be removed:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter()
            .any(|line| line.ends_with(", 2") || line.ends_with(", 3")),
        "expected `2 + 3` to fold to `5`:\n{}",
        asm.join("\n")
    );

    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(vm.console_output, vec!["true"]);
}

#[test]
fn optimized_mixed_bytecode_forwards_name_values_without_reloads() {
    let compiled = compile_source("let a = 1; b = a; a = b + 3;").expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let asm = disassemble_clean(&optimized.bytecode, &optimized.constants);

    assert_eq!(asm[0], "load_i r1, 1");
    assert_eq!(asm[1], "init_name r1, identifier[0]");
    assert!(
        asm.iter()
            .any(|line| line == "store_name r1, identifier[1]"),
        "expected copy store to remain:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter()
            .any(|line| line.starts_with("load_i r") && line.ends_with(", 4")),
        "expected final assignment to fold to `4`:\n{}",
        asm.join("\n")
    );
    assert!(
        asm.iter()
            .any(|line| line.starts_with("store_name ") && line.contains("identifier[0]")),
        "expected final write back to `a`:\n{}",
        asm.join("\n")
    );
    assert_eq!(asm.last().map(String::as_str), Some("ret"));
    assert!(
        !asm.iter().any(|line| line.starts_with("load_name ")),
        "optimized bytecode still reloads names:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line.starts_with("add ")),
        "optimized bytecode still contains an unfused add:\n{}",
        asm.join("\n")
    );

    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(to_f64(vm.frame.regs[255]), Some(4.0));
}

#[test]
fn run_test_fib() {
    let compiled = compile_source(include_str!("../../../fib_recursive.qjs")).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    println!(
        "Optimized bytecode:\n{}",
        disassemble_clean(&optimized.bytecode, &optimized.constants).join("\n")
    );
    let mut vm = VM::from_compiled(optimized, vec![]);

    vm.run(false);
}

#[test]
fn optimized_fib_uses_recursive_call_fusion() {
    let compiled = compile_source(include_str!("../../../fib_recursive.qjs")).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let asm = disassemble_clean(&optimized.bytecode, &optimized.constants);

    assert!(
        asm.iter().any(|line| line == "call2_sub_i_add r2, r1, 1"),
        "expected recursive fib body to use the fused recursive call opcode:\n{}",
        asm.join("\n")
    );
    assert!(
        !asm.iter().any(|line| line == "mov r3, r255, r0"),
        "expected recursive fib body to drop the accumulator shuttle:\n{}",
        asm.join("\n")
    );

    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert!(
        vm.console_output.iter().any(|line| line.contains("75025")),
        "expected fib benchmark output to contain the final recursive result"
    );
}

#[test]
fn run_test_mandelbrot() {
    let compiled = compile_source(include_str!("../../../mandelbrot.js")).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);

    vm.run(false);
}

#[test]
fn run_test_binary() {
    let compiled = compile_source(include_str!("../../../test_binary.js")).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    println!(
        "Optimized bytecode:\n{}",
        disassemble_clean(&optimized.bytecode, &optimized.constants).join("\n")
    );
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

// ============================================================================
// 🔥 INLINE CACHE OPTIMIZATION TESTS
// ============================================================================

/// Test 1: Simple monomorphic function call (single target)
/// 
/// JavaScript:
/// ```javascript
/// function add(a, b) { return a + b; }
/// function caller() {
///     let result = 0;
///     for (let i = 0; i < 5; i++) {
///         result = add(result, 1);
///     }
///     return result;
/// }
/// console.log(caller());
/// ```
///
/// Expected: Monomorphic call site (always calls `add`), IC can cache target
#[test]
fn ic_test_monomorphic_call_site() {
    let source = r#"
        function add(a, b) { return a + b; }
        function caller() {
            let result = 0;
            for (let i = 0; i < 5; i++) {
                result = add(result, 1);
            }
            return result;
        }
        console.log(caller());
    "#;

    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);
    
    assert_eq!(vm.console_output, vec!["5"]);
}

/// Test 2: Recursive function with IC optimization
///
/// JavaScript:
/// ```javascript
/// function factorial(n) {
///     if (n <= 1) return 1;
///     return n * factorial(n - 1);
/// }
/// console.log(factorial(5));
/// ```
///
/// Expected: Recursive call to `factorial` is monomorphic (always same function)
/// IC can cache the tail-call target for significant speedup
#[test]
fn ic_test_recursive_factorial() {
    let source = r#"
        function factorial(n) {
            if (n <= 1) return 1;
            return n * factorial(n - 1);
        }
        console.log(factorial(5));
    "#;

    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(vm.console_output, vec!["120"]);
}

/// Test 3: Multiple function targets (polymorphic call site)
///
/// JavaScript:
/// ```javascript
/// function add(a, b) { return a + b; }
/// function mul(a, b) { return a * b; }
/// function apply(fn, a, b) {
///     return fn(a, b);
/// }
/// console.log(apply(add, 2, 3));
/// console.log(apply(mul, 2, 3));
/// ```
///
/// Expected: Polymorphic call site (apply calls different functions)
/// IC tracks multiple targets for type checking and dispatching
#[test]
fn ic_test_polymorphic_call_site() {
    let source = r#"
        function add(a, b) { return a + b; }
        function mul(a, b) { return a * b; }
        function apply(fn, a, b) {
            return fn(a, b);
        }
        console.log(apply(add, 2, 3));
        console.log(apply(mul, 2, 3));
    "#;

    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(vm.console_output, vec!["5", "6"]);
}

/// Test 4: Loop with HOT call site (repeated calls)
///
/// JavaScript:
/// ```javascript
/// function process(x) { return x * 2; }
/// let sum = 0;
/// for (let i = 0; i < 100; i++) {
///     sum += process(i);
/// }
/// console.log(sum);
/// ```
///
/// Expected: 100 identical calls to `process` → IC identifies as hot/monomorphic
/// Perfect candidate for runtime quickening
#[test]
fn ic_test_hot_loop_call_site() {
    let source = r#"
        function process(x) { return x * 2; }
        let sum = 0;
        for (let i = 0; i < 100; i++) {
            sum += process(i);
        }
        console.log(sum);
    "#;

    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(vm.console_output, vec!["9900"]);
}

/// Test 5: Constructor calls (IC for new operator)
///
/// JavaScript:
/// ```javascript
/// function Point(x, y) {
///     this.x = x;
///     this.y = y;
/// }
/// let p1 = new Point(1, 2);
/// let p2 = new Point(3, 4);
/// console.log(p1.x, p2.y);
/// ```
///
/// Expected: Monomorphic constructor calls to `Point`
/// IC can optimize object shape/layout predictions
#[test]
fn ic_test_constructor_monomorphic() {
    let source = r#"
        function Point(x, y) {
            this.x = x;
            this.y = y;
        }
        let p1 = new Point(1, 2);
        let p2 = new Point(3, 4);
        console.log(p1.x, p2.y);
    "#;

    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(vm.console_output, vec!["1 4"]);
}

/// Test 6: Method call with receiver prediction (IC for object methods)
///
/// JavaScript:
/// ```javascript
/// let obj = {
///     value: 10,
///     getValue: function() { return this.value; }
/// };
/// for (let i = 0; i < 3; i++) {
///     console.log(obj.getValue());
/// }
/// ```
///
/// Expected: Stable object receiver (`obj` doesn't change)
/// IC can cache both the function and receiver shape
#[test]
fn ic_test_method_call_stable_receiver() {
    let source = r#"
        let obj = {
            value: 10,
            getValue: function() { return this.value; }
        };
        for (let i = 0; i < 3; i++) {
            console.log(obj.getValue());
        }
    "#;

    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(vm.console_output, vec!["10", "10", "10"]);
}

/// Test 7: Call with loop-invariant function reference
///
/// JavaScript:
/// ```javascript
/// function operation(x) { return x + 5; }
/// let fn = operation;
/// let result = 0;
/// for (let i = 0; i < 10; i++) {
///     result += fn(i);
/// }
/// console.log(result);
/// ```
///
/// Expected: Function stored in variable is never reassigned
/// IC can cache and predict function reference through entire loop
#[test]
fn ic_test_loop_invariant_function() {
    let source = r#"
        function operation(x) { return x + 5; }
        let fn = operation;
        let result = 0;
        for (let i = 0; i < 10; i++) {
            result += fn(i);
        }
        console.log(result);
    "#;

    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(vm.console_output, vec!["95"]);
}

/// Test 8: Tail call optimization with IC
///
/// JavaScript:
/// ```javascript
/// function sum(n, acc = 0) {
///     if (n <= 0) return acc;
///     return sum(n - 1, acc + n);  // Tail call
/// }
/// console.log(sum(10));
/// ```
///
/// Expected: Tail-recursive call can be optimized with IC
/// Repeated self-calls enable fast-path dispatch
#[test]
fn ic_test_tail_call_recursive() {
    let source = r#"
        function sum(n, acc) {
            if (acc === undefined) acc = 0;
            if (n <= 0) return acc;
            return sum(n - 1, acc + n);
        }
        console.log(sum(10));
    "#;

    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(vm.console_output, vec!["55"]);
}

/// Test 9: Mixed monomorphic and polymorphic sites in one function
///
/// JavaScript:
/// ```javascript
/// function always_same(x) { return x; }
/// function maybe_change(x) { return x * 2; }
/// function mixed(fn, x) {
///     return always_same(maybe_change(x));  // First is always same, second varies
/// }
/// console.log(mixed(maybe_change, 5));
/// ```
///
/// Expected: `always_same` is monomorphic (IC eligible)
///          `maybe_change` is polymorphic (IC still tracks)
#[test]
fn ic_test_mixed_monomorphic_and_polymorphic() {
    let source = r#"
        function always_same(x) { return x; }
        function maybe_change(x) { return x * 2; }
        function mixed(fn, x) {
            return always_same(maybe_change(x));
        }
        console.log(mixed(maybe_change, 5));
    "#;

    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(vm.console_output, vec!["10"]);
}

/// Test 10: Recursive Fibonacci with IC (performance test)
///
/// JavaScript:
/// ```javascript
/// function fib(n) {
///     if (n <= 1) return n;
///     return fib(n - 1) + fib(n - 2);
/// }
/// console.log(fib(10));
/// ```
///
/// Expected: Highly recursive monomorphic call site
/// IC should provide significant speedup for many repeated calls
#[test]
fn ic_test_fibonacci_recursive_ic() {
    let source = r#"
        function fib(n) {
            if (n <= 1) return n;
            return fib(n - 1) + fib(n - 2);
        }
        console.log(fib(10));
    "#;

    let compiled = compile_source(source).expect("compile");
    let optimized = optimization::optimize_compiled(compiled);
    let mut vm = VM::from_compiled(optimized, vec![]);
    vm.set_console_echo(false);
    vm.run(false);

    assert_eq!(vm.console_output, vec!["55"]);
}
