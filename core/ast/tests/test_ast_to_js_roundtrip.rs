use ::ast::{ParseOptions, parse, parse_with_options, program_to_js};

#[test]
fn test262_ast_to_js_roundtrip_basic() {
    // Test basic AST -> JS -> AST round-trip for simple cases
    let test_cases = [
        // Basic expressions
        "1 + 2;",
        "a = b + c;",
        "function f(x) { return x * 2; }",
        "const obj = { a: 1, b: 2 };",
        "class C { method() { return this.value; } }",
        "for (let i = 0; i < 10; i++) { console.log(i); }",
        "if (condition) { doSomething(); } else { doSomethingElse(); }",
        "try { risky(); } catch (e) { handle(e); }",
        "export const x = 1;",
        "import { y } from './module.js';",
    ];

    for (i, source) in test_cases.iter().enumerate() {
        // Determine if this looks like a module
        let is_module = source.contains("import") || source.contains("export");
        let options = ParseOptions {
            is_module,
            is_strict: is_module,
        };

        let parse_result = if is_module {
            parse_with_options(source, options)
        } else {
            parse(source)
        };

        let program = match parse_result {
            Ok(program) => program,
            Err(e) => panic!("Test case {} failed to parse: {}\nSource: {}", i, e, source),
        };

        let emitted = program_to_js(&program);
        let reparsed = if is_module {
            parse_with_options(&emitted, options)
        } else {
            parse(&emitted)
        };

        let reparsed_program = match reparsed {
            Ok(program) => program,
            Err(e) => panic!(
                "Test case {} failed to reparse emitted JS: {}\nSource: {}\nEmitted: {}",
                i, e, source, emitted
            ),
        };

        let re_emitted = program_to_js(&reparsed_program);

        // Check that the re-emitted code matches the first emission
        // Note: We don't check that the ASTs are equal because formatting might differ
        // but the emitted code should be stable after one round-trip
        if emitted != re_emitted {
            panic!(
                "Test case {} failed: emitted code not stable after round-trip\nOriginal source: {}\nFirst emission:\n{}\nSecond emission:\n{}",
                i, source, emitted, re_emitted
            );
        }
    }
}

#[test]
fn test262_ast_to_js_roundtrip_with_test262_examples() {
    // Test with some actual test262 test cases that are known to work
    // These are simple test cases that should round-trip correctly
    let test_cases = [
        // Simple expression test
        r#"assert.sameValue(1 + 2, 3, '1 + 2 should be 3');"#,
        // Function declaration test
        r#"function add(a, b) { return a + b; }"#,
        // Class test
        r#"class Point { constructor(x, y) { this.x = x; this.y = y; } }"#,
        // Arrow function
        r#"const add = (a, b) => a + b;"#,
        // Template literal
        r#"const name = 'world'; const greeting = `Hello, ${name}!`;"#,
    ];

    for (i, source) in test_cases.iter().enumerate() {
        let program = match parse(source) {
            Ok(program) => program,
            Err(e) => panic!(
                "Test262 example {} failed to parse: {}\nSource: {}",
                i, e, source
            ),
        };

        let emitted = program_to_js(&program);
        let reparsed = match parse(&emitted) {
            Ok(program) => program,
            Err(e) => panic!(
                "Test262 example {} failed to reparse emitted JS: {}\nSource: {}\nEmitted: {}",
                i, e, source, emitted
            ),
        };

        let re_emitted = program_to_js(&reparsed);

        if emitted != re_emitted {
            panic!(
                "Test262 example {} failed: emitted code not stable after round-trip\nOriginal source: {}\nFirst emission:\n{}\nSecond emission:\n{}",
                i, source, emitted, re_emitted
            );
        }
    }
}
