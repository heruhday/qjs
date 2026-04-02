#[path = "support/js_suite_support.rs"]
mod js_suite_support;

use codegen::compile_source;
use vm::VM;

fn run_suite(source: &str) -> Vec<String> {
    let source = js_suite_support::normalize_suite(source.trim());
    println!("Running suite:\n{}", source);
    let compiled = compile_source(&source).expect("compile JS suite");
    let mut vm = VM::from_compiled(compiled, vec![]);
    vm.set_console_echo(false);
    vm.run(true);
    vm.console_output.clone()
}

fn assert_suite_passed(output: &[String], summary_fragment: &str) {
    assert!(
        output.iter().any(|line| line.contains(summary_fragment)),
        "missing final pass summary `{summary_fragment}`:\n{}",
        output.join("\n")
    );
    assert!(
        !output.iter().any(|line| line.contains("tests failed")),
        "suite reported failures:\n{}",
        output.join("\n")
    );
}

#[test]
fn constant_folding_js_suite_runs() {
    let output = run_suite(include_str!("js/constant_folding_tests.js"));
    assert_suite_passed(&output, "Constant Folding tests passed.");
}

#[test]
fn cfg_js_suite_runs() {
    let output = run_suite(include_str!("js/cfg_test.js"));
    assert_suite_passed(&output, "CFG Simplification tests passed.");
}

#[test]
fn copy_propagation_js_suite_runs() {
    let output = run_suite(include_str!("js/copy_propagation_test.js"));
    assert_suite_passed(&output, "Copy Propagation tests passed.");
}

#[test]
fn dead_code_elimination_js_suite_runs() {
    let output = run_suite(include_str!("js/dead_code_elimination_test.js"));
    assert_suite_passed(&output, "Dead Code Elimination tests passed.");
}

#[test]
fn gvn_js_suite_runs() {
    let output = run_suite(include_str!("js/gvn_test.js"));
    assert_suite_passed(&output, "GVN tests passed.");
}

#[test]
fn licm_js_suite_runs() {
    let output = run_suite(include_str!("js/loop_invariant_code_motion_test.js"));
    assert_suite_passed(&output, "Loop Invariant Code Motion tests passed.");
}

#[test]
fn sccp_js_suite_runs() {
    let output = run_suite(include_str!(
        "js/sparse_conditional_constant_propagation_tests.js"
    ));
    assert_suite_passed(
        &output,
        "Sparse Conditional Constant Propagation tests passed.",
    );
}

#[test]
fn vrp_js_suite_runs() {
    let output = run_suite(include_str!("js/value_range_propagation_test.js"));
    assert_suite_passed(&output, "Value Range Propagation tests passed.");
}
