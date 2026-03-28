use std::io::Write;
use std::process::{Command, Output, Stdio};

use ::ast::{parse, program_to_js};

const TEST262_STA: &str = include_str!("../../../test262/harness/sta.js");
const TEST262_ASSERT: &str = include_str!("../../../test262/harness/assert.js");

struct Tes262Case {
    path: &'static str,
    source: &'static str,
}

const TES262_CASES: &[Tes262Case] = &[
    Tes262Case {
        path: "test262/test/language/expressions/arrow-function/arrow/capturing-closure-variables-1.js",
        source: include_str!(
            "../../../test262/test/language/expressions/arrow-function/arrow/capturing-closure-variables-1.js"
        ),
    },
    Tes262Case {
        path: "test262/test/language/expressions/coalesce/chainable.js",
        source: include_str!("../../../test262/test/language/expressions/coalesce/chainable.js"),
    },
    Tes262Case {
        path: "test262/test/language/expressions/coalesce/follows-null.js",
        source: include_str!("../../../test262/test/language/expressions/coalesce/follows-null.js"),
    },
    Tes262Case {
        path: "test262/test/language/expressions/coalesce/short-circuit-number-42.js",
        source: include_str!(
            "../../../test262/test/language/expressions/coalesce/short-circuit-number-42.js"
        ),
    },
    Tes262Case {
        path: "test262/test/language/expressions/conditional/coalesce-expr-ternary.js",
        source: include_str!(
            "../../../test262/test/language/expressions/conditional/coalesce-expr-ternary.js"
        ),
    },
    Tes262Case {
        path: "test262/test/language/expressions/optional-chaining/member-expression.js",
        source: include_str!(
            "../../../test262/test/language/expressions/optional-chaining/member-expression.js"
        ),
    },
    Tes262Case {
        path: "test262/test/language/expressions/optional-chaining/optional-call-preserves-this.js",
        source: include_str!(
            "../../../test262/test/language/expressions/optional-chaining/optional-call-preserves-this.js"
        ),
    },
    Tes262Case {
        path: "test262/test/language/expressions/optional-chaining/runtime-semantics-evaluation.js",
        source: include_str!(
            "../../../test262/test/language/expressions/optional-chaining/runtime-semantics-evaluation.js"
        ),
    },
    Tes262Case {
        path: "test262/test/language/expressions/optional-chaining/short-circuiting.js",
        source: include_str!(
            "../../../test262/test/language/expressions/optional-chaining/short-circuiting.js"
        ),
    },
];

fn node_is_available() -> bool {
    Command::new("node")
        .arg("-v")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn build_test262_script(source: &str) -> String {
    format!("{TEST262_STA}\n{TEST262_ASSERT}\n{source}")
}

fn run_in_node(script: &str) -> std::io::Result<Output> {
    let mut child = Command::new("node")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .expect("spawned Node process should expose stdin");
    stdin.write_all(script.as_bytes())?;
    drop(stdin);

    child.wait_with_output()
}

fn output_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn ast_to_js_emitted_programs_pass_curated_tes262_runtime_cases() {
    if !node_is_available() {
        eprintln!(
            "skipping Node-backed Test262 ast_to_js runtime test because `node` is unavailable"
        );
        return;
    }

    for case in TES262_CASES {
        let baseline_output =
            run_in_node(&build_test262_script(case.source)).unwrap_or_else(|error| {
                panic!(
                    "failed to execute baseline Test262 case {} with Node: {}",
                    case.path, error
                )
            });
        assert!(
            baseline_output.status.success(),
            "baseline Test262 case failed under Node: {}\nstdout:\n{}\nstderr:\n{}",
            case.path,
            output_text(&baseline_output.stdout),
            output_text(&baseline_output.stderr),
        );

        let program = parse(case.source).unwrap_or_else(|error| {
            panic!("failed to parse Test262 case {}: {:?}", case.path, error)
        });
        let emitted = program_to_js(&program);
        let emitted_output = run_in_node(&build_test262_script(&emitted)).unwrap_or_else(|error| {
            panic!(
                "failed to execute emitted JS for Test262 case {} with Node: {}",
                case.path, error
            )
        });

        assert!(
            emitted_output.status.success(),
            "emitted JS failed Test262 runtime case: {}\nstdout:\n{}\nstderr:\n{}\nemitted:\n{}",
            case.path,
            output_text(&emitted_output.stdout),
            output_text(&emitted_output.stderr),
            emitted,
        );
    }
}
