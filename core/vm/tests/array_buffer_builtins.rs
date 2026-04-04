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
fn array_buffer_constructor_properties_and_slice_work() {
    let vm = run_program(
        r#"
        const buffer = new ArrayBuffer(6, { maxByteLength: 10 });
        const view = new DataView(buffer);
        view.setUint8(0, 10);
        view.setUint8(1, 20);
        view.setUint8(2, 30);
        view.setUint8(3, 40);
        view.setUint8(4, 50);
        view.setUint8(5, 60);

        const middle = buffer.slice(1, 4);
        const middleView = new DataView(middle);
        const tail = buffer.slice(-3, -1);
        const tailView = new DataView(tail);

        console.log(
          buffer.byteLength,
          buffer.maxByteLength,
          buffer.resizable,
          buffer.detached
        );
        console.log(
          middle.byteLength,
          middle.maxByteLength,
          middle.resizable,
          middleView.getUint8(0),
          middleView.getUint8(2)
        );
        console.log(
          tail.byteLength,
          tailView.getUint8(0),
          tailView.getUint8(1),
          ArrayBuffer.isView(view),
          ArrayBuffer.isView(buffer)
        );
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec!["6 10 true false", "3 3 false 20 40", "2 40 50 true false"]
    );
}

#[test]
fn array_buffer_resize_updates_length_and_zero_fills_growth() {
    let vm = run_program(
        r#"
        const buffer = new ArrayBuffer(2, { maxByteLength: 5 });
        let view = new DataView(buffer);
        view.setUint8(0, 7);
        view.setUint8(1, 9);

        buffer.resize(5);
        view = new DataView(buffer);
        console.log(
          buffer.byteLength,
          buffer.maxByteLength,
          buffer.resizable,
          view.getUint8(0),
          view.getUint8(1),
          view.getUint8(2),
          view.getUint8(4)
        );

        buffer.resize(1);
        view = new DataView(buffer);
        console.log(
          buffer.byteLength,
          view.byteLength,
          view.getUint8(0),
          view.getUint8(1) === undefined
        );
    "#,
    );

    assert_eq!(vm.console_output, vec!["5 5 true 7 9 0 0", "1 1 7 true"]);
}

#[test]
fn array_buffer_fixed_length_resize_reports_error_string() {
    let vm = run_program(
        r#"
        const fixed = new ArrayBuffer(3);
        console.log(
          fixed.byteLength,
          fixed.maxByteLength,
          fixed.resizable,
          fixed.resize(4)
        );
    "#,
    );

    assert_eq!(
        vm.console_output,
        vec!["3 3 false TypeError: Cannot resize a fixed-length ArrayBuffer"]
    );
}
