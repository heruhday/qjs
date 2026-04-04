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
fn array_static_methods_and_constructor_work() {
    let vm = run_program(
        r#"
        const sized = new Array(3);
        const from = Array.from([1, 2, 3]);
        const of = Array.of(4, 5);

        console.log(
          Array.isArray(from),
          from.join("-"),
          of.join("-"),
          sized.length,
          sized[0] === undefined
        );
    "#,
    );

    assert_eq!(vm.console_output, vec!["true 1-2-3 4-5 3 true"]);
}

#[test]
fn array_mutation_methods_work() {
    let vm = run_program(
        r#"
        const arr = [1, 2];
        console.log(arr.push(3, 4), arr.join(","));
        console.log(arr.pop(), arr.join(","));
        console.log(arr.shift(), arr.join(","));
        console.log(arr.unshift(9, 8), arr.join(","));
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec!["4 1,2,3,4", "4 1,2,3", "1 2,3", "4 9,8,2,3"]
    );
}

#[test]
fn array_search_and_slice_methods_work() {
    let vm = run_program(
        r#"
        const arr = ["a", "b", "a", NaN];
        console.log(
          arr.indexOf("a", 1),
          arr.lastIndexOf("a", 1),
          arr.includes(NaN),
          ["x", "y", "z"].slice(-2).join(",")
        );
    "#,
    );

    assert_eq!(vm.console_output, vec!["2 0 true y,z"]);
}

#[test]
fn array_callback_and_reducer_methods_work() {
    let vm = run_program(
        r#"
        const arr = [1, 2, 3];
        console.log(
          arr.map((value, index) => value + index).join(","),
          arr.filter(value => value > 1).join(","),
          arr.find(value => value === 2),
          arr.findIndex(value => value === 2),
          arr.some(value => value === 3),
          arr.every(value => value < 4)
        );

        console.log(
          [1, 2, 3, 4].reduce((acc, value) => acc + value),
          [1, 2, 3, 4].reduceRight((acc, value) => acc - value)
        );
    "#,
    );

    assert_eq!(vm.console_output, vec!["1,3,5 2,3 2 1 true true", "10 -2"]);
}

#[test]
fn array_transform_methods_work() {
    let vm = run_program(
        r#"
        const arr = [1, 2, 3, 4, 5];
        console.log(arr.fill.apply(arr, [9, 1, 4]).join(","));
        console.log(arr.copyWithin(0, -2).join(","));

        const splice = [1, 2, 3, 4];
        const removed = splice.splice.apply(splice, [1, 2, 8, 9]);
        console.log(removed.join(","), splice.join(","));

        const order = [3, 1, 2];
        order.sort((a, b) => a - b);
        console.log(order.join(","));

        const rev = [1, 2, 3];
        rev.reverse();
        console.log(rev.join(","));

        console.log([1, [2, [3]], 4].flat(2).join(","));
        console.log([1, 2, 3].flatMap(value => [value, value * 10]).join(","));
        console.log([1, 2, 3].at(-1), [1, 2, 3].concat([4], 5).join(","));
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec![
            "1,9,9,9,5",
            "9,5,9,9,5",
            "2,3 1,8,9,4",
            "1,2,3",
            "3,2,1",
            "1,2,3,4",
            "1,10,2,20,3,30",
            "3 1,2,3,4,5",
        ]
    );
}
