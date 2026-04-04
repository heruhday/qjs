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
fn math_number_string_boolean_and_symbol_builtins_work() {
    let vm = run_program(
        r#"
        console.log(
            Math.abs(-3),
            Math.floor(3.8),
            Number.parseInt("ff", 16),
            Number.isInteger(42),
            String.fromCharCode(65, 66),
            Boolean(""),
            Symbol.keyFor(Symbol.for("shared"))
        );
    "#,
    );

    assert_eq!(vm.console_output, vec!["3 3 255 true AB false shared"]);
}

#[test]
fn array_and_object_builtins_work() {
    let vm = run_program(
        r#"
        const arr = Array.of(1, 2, 3);
        const copied = Array.from(arr);
        const obj = Object.assign({}, { a: 1, b: 2 });
        console.log(
            Array.isArray(copied),
            Object.keys(obj),
            Object.values(obj),
            Object.hasOwn(obj, "a"),
            Object.entries(obj)
        );
    "#,
    );

    assert_eq!(vm.console_output, vec!["true a,b 1,2 true a,1,b,2"]);
}

#[test]
fn date_and_eval_builtins_work() {
    let vm = run_program(
        r#"
        const date = new Date(0);
        console.log(
            date.getTime(),
            date.toISOString(),
            eval("1 + 2;"),
            eval("Date.UTC(1970, 0, 1)")
        );
    "#,
    );

    assert_eq!(vm.console_output, vec!["0 1970-01-01T00:00:00+00:00 3 0"]);
}

#[test]
fn map_set_and_weak_collection_builtins_work() {
    let vm = run_program(
        r#"
        const map = new Map([["a", 1]]);
        map.set("b", 2);
        console.log(map.get("a"), map.has("b"), map.size);

        map.delete("a");
        const set = new Set([1, 2, 2]);
        set.add(3);
        set.delete(2);
        console.log(set.has(2), set.has(3), set.size);

        const weakMap = new WeakMap();
        const key = {};
        weakMap.set(key, 9);
        console.log(weakMap.get(key));
    "#,
    );

    assert_eq!(vm.console_output, vec!["1 true 2", "false true 2", "9"]);
}

#[test]
fn buffer_and_typed_array_builtins_work() {
    let vm = run_program(
        r#"
        const buf = new ArrayBuffer(4);
        const view = new DataView(buf);
        view.setUint8(0, 255);
        view.setUint8(1, 7);

        const typed = TypedArray.from([1, 2, 3]);
        typed.set([9, 8], 1);

        console.log(
            buf.byteLength,
            view.getUint8(0),
            view.getUint8(1),
            ArrayBuffer.isView(view),
            ArrayBuffer.isView(typed)
        );
        console.log(typed.length, typed.at(0), typed.at(1), typed.toArray());
    "#,
    );

    assert_eq!(vm.console_output, vec!["4 255 7 true true", "3 1 9 1,9,8"]);
}

#[test]
fn function_reflect_uri_and_regexp_builtins_work() {
    let vm = run_program(
        r#"
        const add = Function("a", "b", "return a + b;");
        const bound = add.bind(null, 2);
        const obj = {};
        Reflect.set(obj, "value", 9);
        const re = new RegExp("ab", "ig");
        const match = re.exec("xxAByyab");

        console.log(
            add.apply(null, [3, 4]),
            bound(5),
            Reflect.apply(add, null, [6, 7]),
            Reflect.get(obj, "value"),
            Reflect.has(obj, "value"),
            Reflect.deleteProperty(obj, "value"),
            encodeURIComponent("a b"),
            decodeURIComponent("a%20b"),
            new RegExp("ab", "i").test("cab")
        );
        console.log(match[0], match.index, re.toString());
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec!["7 7 13 9 true true a%20b a b true", "AB 2 /ab/ig"]
    );
}

#[test]
fn iterator_generator_promise_and_proxy_builtins_work() {
    let vm = run_program(
        r#"
        const iter = Iterator.from("ab");
        console.log(iter.next().value, iter.next().value, iter.next().done);

        const gen = Generator.from([7, 8]);
        console.log(gen.next().value, gen.toArray());

        const make = GeneratorFunction("values", "return values;");
        const made = make([9, 8]);
        console.log(made.next().value, made.next().value, made.next().done);

        Promise.resolve(1)
          .then(v => v + 2)
          .then(v => console.log(v));

        Promise.all([Promise.resolve(1), 2, Promise.resolve(3)])
          .then(values => console.log(values));

        const proxiedValue = new Proxy(
          { value: 2 },
          { get() { return 5; } }
        );
        console.log(Reflect.get(proxiedValue, "value"));

        const proxiedCall = new Proxy(
          function (n) { return n + 1; },
          { apply(target, thisArg, args) { return target(args[0]) * 2; } }
        );
        console.log(proxiedCall(4));

        const revocable = Proxy.revocable({ value: 10 }, {});
        revocable.revoke();
        console.log(Reflect.get(revocable.proxy, "value"));
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec![
            "a b true",
            "7 8",
            "9 8 true",
            "3",
            "1,2,3",
            "5",
            "10",
            "undefined"
        ]
    );
}

#[test]
fn intl_and_temporal_builtins_work() {
    let vm = run_program(
        r#"
        const locale = new Intl.Locale("EN_us");
        const nf = new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 });
        const df = new Intl.DateTimeFormat("en-US");
        const instant = Temporal.Instant.fromEpochMilliseconds(0);
        const plainDate = Temporal.PlainDate.from("2024-05-06");
        const plainDateTime = Temporal.PlainDateTime.from("2024-05-06T07:08:09.010");

        console.log(
          Intl.getCanonicalLocales(["EN_us", "id-ID"]),
          locale.toString(),
          nf.format(12.345),
          df.format(new Date(0))
        );
        console.log(
          instant.toString(),
          plainDate.toString(),
          plainDateTime.toString(),
          Temporal.Now.instant().epochMilliseconds > 0,
          Temporal.Now.plainDateISO().year > 2000
        );
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec![
            "en-US,id-ID en-US 12.35 1970-01-01 00:00:00 UTC",
            "1970-01-01T00:00:00+00:00 2024-05-06 2024-05-06T07:08:09.010 true true",
        ]
    );
}
