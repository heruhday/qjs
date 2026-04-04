use codegen::compile_source;
use vm::VM;

fn run_program(source: &str) -> VM {
    let compiled = compile_source(source).expect("compile");
    let mut vm = VM::from_compiled(compiled, vec![]);
    vm.set_console_echo(false);
    vm.run(false);
    vm
}

#[test]
fn string_constructor_and_static_methods_work() {
    let vm = run_program(
        r#"
        const expected = "e";
        const boxed = new String("hello");

        console.log(
          String(42),
          String.fromCharCode(65, 66),
          String.fromCodePoint(0x1F642),
          boxed.length,
          boxed[1] === expected,
          boxed.propertyIsEnumerable("length")
        );
    "#,
    );

    assert_eq!(vm.console_output, vec!["42 AB 🙂 5 true false"]);
}

#[test]
fn string_core_methods_work() {
    let vm = run_program(
        r#"
        const text = "banana";

        console.log(
          text.charAt(1),
          text.at(-1),
          text.charCodeAt(1),
          text.codePointAt(1),
          text.concat("!", "?"),
          text.repeat(2)
        );

        console.log(
          text.slice(1, 4),
          text.slice(-3),
          text.substring(4, 1),
          text.startsWith("ana", 1),
          text.endsWith("na"),
          text.includes("nan"),
          text.indexOf("na", 2),
          text.lastIndexOf("na")
        );
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec![
            "a a 97 97 banana!? bananabanana",
            "ana ana ana true true true 2 4"
        ]
    );
}

#[test]
fn string_transform_split_and_search_work() {
    let vm = run_program(
        r#"
        const spaced = "  Hello  ";

        console.log(
          spaced.trim() === "Hello",
          spaced.trimStart() === "Hello  ",
          spaced.trimEnd() === "  Hello",
          "Hi".padStart(5, "0"),
          "Hi".padEnd(5, "!")
        );

        console.log(
          "AbC".toLowerCase(),
          "AbC".toUpperCase(),
          "a,b,,c".split(",", 3),
          "hello".split(""),
          "hello".split(undefined),
          "hello world".search("world")
        );
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec![
            "true true true 000Hi Hi!!!",
            "abc ABC a,b, h,e,l,l,o hello 6"
        ]
    );
}

#[test]
fn string_replace_and_regexp_hooks_work() {
    let vm = run_program(
        r#"
        console.log(
          "hello world".replace("world", "there"),
          "hello".replace("ll", (match, index, input) => match.toUpperCase() + index),
          "a-a-a".replaceAll("-", ":")
        );

        const re = new RegExp("ab");
        console.log(
          "xxabyy".search(re),
          "xxabyy".replace(re, "Z"),
          "one two".replace("two", "[$&]-[$`]-[$']")
        );
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec!["hello there heLL2o a:a:a", "2 xxZyy one [two]-[one ]-[]"]
    );
}

#[test]
fn string_error_like_results_are_stable() {
    let vm = run_program(
        r#"
        console.log(
          "x".repeat(-1),
          String.fromCodePoint(-1)
        );
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec!["RangeError: repeat count must be non-negative RangeError: code point out of range"]
    );
}
