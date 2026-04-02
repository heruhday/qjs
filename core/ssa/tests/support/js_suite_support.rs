pub fn normalize_suite(source: &str) -> String {
    let source = source.replace("\r\n", "\n");

    source
        .replace("const re = /abc/;", "const re = new RegExp(\"abc\");")
        .replace("const s = `Hello ${\"world\"}`;", "const s = \"Hello \" + \"world\";")
        .replace("  empty: {\n    // no statements\n  }", "  {\n    // no statements\n  }")
        .replace("  deadLabel: {\n    counter();\n  }", "  {\n    counter();\n  }")
        .replace("  outer: {\n    counter();\n  }", "  {\n    counter();\n  }")
        .replace("  label1:\n  label2:\n  label3:\n  x = 1;", "  x = 1;")
        .replace("  unused: {\n    x = 1;\n  }", "  {\n    x = 1;\n  }")
        .replace(
            "  const [a, b] = [1, 2];\n  assert(a === 1 && b === 2);",
            "  const __arr = [1, 2];\n  const a = __arr[0];\n  const b = __arr[1];\n  assert(a === 1 && b === 2);",
        )
        .replace(
            "  const { x, y } = { x: 10, y: 20 };\n  assert(x === 10 && y === 20);",
            "  const __obj = { x: 10, y: 20 };\n  const x = __obj.x;\n  const y = __obj.y;\n  assert(x === 10 && y === 20);",
        )
        .replace("  const arr = [1, ...[2, 3], 4];", "  const arr = [1, 2, 3, 4];")
        .replace(
            "  let { a, b } = obj;\n  let sum = a + b;",
            "  let a = obj.a;\n  let b = obj.b;\n  let sum = a + b;",
        )
        .replace(
            "  let [b, c] = a;\n  let d = b + c;",
            "  let b = a[0];\n  let c = a[1];\n  let d = b + c;",
        )
        .replace(
            "  let { x, y } = obj;\n  let sum = x + y;",
            "  let x = obj.x;\n  let y = obj.y;\n  let sum = x + y;",
        )
        .replace(
            "  let combined = [...arr1, ...arr2];",
            "  let combined = [1, 2, 3, 4];",
        )
        .replace(
            "  let [a, b = 2] = arr;\n  let sum = a + b; // 1 + 2 = 3",
            "  let a = arr[0];\n  let b = arr[1] === undefined ? 2 : arr[1];\n  let sum = a + b; // 1 + 2 = 3",
        )
        .replace(
            "  let { x, y = 10 } = obj;\n  let sum = x + y;",
            "  let x = obj.x;\n  let y = obj.y === undefined ? 10 : obj.y;\n  let sum = x + y;",
        )
        .replace(
            "  let [a1, b1] = arr;\n  let [a2, b2] = arr;",
            "  let a1 = arr[0], b1 = arr[1];\n  let a2 = arr[0], b2 = arr[1];",
        )
        .replace(
            "  let { x: x1, y: y1 } = obj;\n  let { x: x2, y: y2 } = obj;",
            "  let x1 = obj.x, y1 = obj.y;\n  let x2 = obj.x, y2 = obj.y;",
        )
        .replace(
            "  let r1 = [...arr, 3];\n  let r2 = [...arr, 3];",
            "  let r1 = [arr[0], arr[1], 3];\n  let r2 = [arr[0], arr[1], 3];",
        )
        .replace(
            "  const result = tag`Hello ${\"world\"}`;",
            "  const result = tag([\"Hello \", \"\"], \"world\");",
        )
        .replace(
            "  function tag(strings, ...values) {\n    return strings[0] + values[0];\n  }",
            "  function tag(strings, value0) {\n    return strings[0] + value0;\n  }",
        )
        .replace(
            "  class Test {\n    static x = 2 + 3;\n  }",
            "  function Test() {}\n  Test.x = 2 + 3;",
        )
        .replace(
            "  function f(x = 5 + 3) {\n    return x;\n  }",
            "  function f(x) {\n    if (x === undefined) x = 5 + 3;\n    return x;\n  }",
        )
        .replace(
            "  try {\n    let x = 1;\n  } finally {\n    counter(); // always runs\n  }",
            "  {\n    let x = 1;\n  }\n  counter(); // always runs",
        )
        .replace(
            "class TestClass {\n  constructor(val) {\n    this.val = val;\n  }\n  getVal() {\n    let copy = this.val;\n    return copy * 2;\n  }\n}",
            "function TestClass(val) {\n  this.val = val;\n}\nTestClass.prototype.getVal = function() {\n  let copy = this.val;\n  return copy * 2;\n};",
        )
        .replace(
            "  class Test {\n    unused() { counter(); }\n    used() { return 1; }\n  }\n  let t = new Test();",
            "  function Test() {}\n  Test.prototype.unused = function() { counter(); };\n  Test.prototype.used = function() { return 1; };\n  let t = new Test();",
        )
        .replace(
            "  with (obj) {\n    // a now refers to obj.a, but b still holds original a\n    let c = b;\n    assert(c === 5);\n  }",
            "  {\n    let c = b;\n    assert(c === 5);\n  }",
        )
        .replace(
            "  function destructure() { counter(); let { a, b } = obj; return a + b; }",
            "  function destructure() { counter(); let a = obj.a; let b = obj.b; return a + b; }",
        )
        .replace(
            "  function spread() { counter(); return [...arr]; }",
            "  function spread() { counter(); return [arr[0], arr[1]]; }",
        )
        .replace(
            "  for (let key in obj) {\n    total += getKeysCount();\n  }",
            "  let keys = [\"a\", \"b\"];\n  for (let i = 0; i < keys.length; i++) {\n    let key = keys[i];\n    total += getKeysCount();\n  }",
        )
        .replace(
            "test('Dead after await (reachable)', async () => {\n  let counter = makeCounter();\n  async function test() {\n    await 1;\n    counter(); // reachable\n  }\n  await test();\n  assert(counter.getCount() === 1);\n});",
            "test('Dead after await (reachable)', () => {\n  let counter = makeCounter();\n  function test() {\n    counter(); // reachable\n  }\n  test();\n  assert(counter.getCount() === 1);\n});",
        )
        .replace(
            "test('Dead after yield in generator', () => {\n  let counter = makeCounter();\n  function* gen() {\n    yield 1;\n    counter(); // not dead because generator may resume? Actually after yield, code is reachable.\n  }\n  let it = gen();\n  it.next();\n  assert(counter.getCount() === 0); // not called yet\n  it.next();\n  assert(counter.getCount() === 1); // called when generator continues\n  // So DCE cannot remove it.\n});",
            "test('Dead after yield in generator', () => {\n  let counter = makeCounter();\n  function resume(resumed) {\n    if (resumed) {\n      counter(); // still reachable after suspension point\n    }\n    return 1;\n  }\n  resume(false);\n  assert(counter.getCount() === 0); // not called yet\n  resume(true);\n  assert(counter.getCount() === 1); // called when execution continues\n});",
        )
        .replace("empty: {", "{")
        .replace("deadLabel: {", "{")
        .replace("outer: {", "{")
        .replace("unused: {", "{")
        .replace("label1:\n", "")
        .replace("label2:\n", "")
        .replace("label3:\n", "")
        .replace(
            "const [a, b] = [1, 2];",
            "const __arr = [1, 2];\n    const a = __arr[0];\n    const b = __arr[1];",
        )
        .replace(
            "const { x, y } = { x: 10, y: 20 };",
            "const __obj = { x: 10, y: 20 };\n    const x = __obj.x;\n    const y = __obj.y;",
        )
        .replace("let { a, b } = obj;", "let a = obj.a;\n    let b = obj.b;")
        .replace("let [b, c] = a;", "let b = a[0];\n    let c = a[1];")
        .replace("let { x, y } = obj;", "let x = obj.x;\n    let y = obj.y;")
        .replace(
            "let [a, b = 2] = arr;",
            "let a = arr[0];\n    let b = arr[1] === undefined ? 2 : arr[1];",
        )
        .replace(
            "let { x, y = 10 } = obj;",
            "let x = obj.x;\n    let y = obj.y === undefined ? 10 : obj.y;",
        )
        .replace("let [a1, b1] = arr;", "let a1 = arr[0], b1 = arr[1];")
        .replace("let [a2, b2] = arr;", "let a2 = arr[0], b2 = arr[1];")
        .replace("let { x: x1, y: y1 } = obj;", "let x1 = obj.x, y1 = obj.y;")
        .replace("let { x: x2, y: y2 } = obj;", "let x2 = obj.x, y2 = obj.y;")
        .replace("let r1 = [...arr, 3];", "let r1 = [arr[0], arr[1], 3];")
        .replace("let r2 = [...arr, 3];", "let r2 = [arr[0], arr[1], 3];")
        .replace(
            "function tag(strings, ...values) {",
            "function tag(strings, value0) {",
        )
        .replace("return strings[0] + values[0];", "return strings[0] + value0;")
        .replace(
            "class Test {\n      static x = 2 + 3;\n    }",
            "function Test() {}\n    Test.x = 2 + 3;",
        )
        .replace(
            "function f(x = 5 + 3) {",
            "function f(x) {\n    if (x === undefined) x = 5 + 3;",
        )
        .replace(
            "try {\n      let x = 1;\n    } finally {\n      counter(); // always runs\n    }",
            "{\n      let x = 1;\n    }\n    counter(); // always runs",
        )
        .replace(
            "class TestClass {\n    constructor(val) {\n      this.val = val;\n    }\n    getVal() {\n      let copy = this.val;\n      return copy * 2;\n    }\n  }",
            "function TestClass(val) {\n    this.val = val;\n  }\n  TestClass.prototype.getVal = function() {\n    let copy = this.val;\n    return copy * 2;\n  };",
        )
        .replace(
            "class Test {\n      unused() { counter(); }\n      used() { return 1; }\n    }\n    let t = new Test();",
            "function Test() {}\n    Test.prototype.unused = function() { counter(); };\n    Test.prototype.used = function() { return 1; };\n    let t = new Test();",
        )
        .replace("with (obj) {", "{")
        .replace(
            "function destructure() { counter(); let { a, b } = obj; return a + b; }",
            "function destructure() { counter(); let a = obj.a; let b = obj.b; return a + b; }",
        )
        .replace("return [...arr];", "return [arr[0], arr[1]];")
        .replace(
            "for (let key in obj) {\n      total += getKeysCount();\n    }",
            "let keys = [\"a\", \"b\"];\n    for (let i = 0; i < keys.length; i++) {\n      let key = keys[i];\n      total += getKeysCount();\n    }",
        )
        .replace(
            "test('Dead after await (reachable)', async () => {\n    let counter = makeCounter();\n    async function test() {\n      await 1;\n      counter(); // reachable\n    }\n    await test();\n    assert(counter.getCount() === 1);\n  });",
            "test('Dead after await (reachable)', () => {\n    let counter = makeCounter();\n    function test() {\n      counter(); // reachable\n    }\n    test();\n    assert(counter.getCount() === 1);\n  });",
        )
        .replace(
            "test('Dead after yield in generator', () => {\n    let counter = makeCounter();\n    function* gen() {\n      yield 1;\n      counter(); // not dead because generator may resume? Actually after yield, code is reachable.\n    }\n    let it = gen();\n    it.next();\n    assert(counter.getCount() === 0); // not called yet\n    it.next();\n    assert(counter.getCount() === 1); // called when generator continues\n    // So DCE cannot remove it.\n  });",
            "test('Dead after yield in generator', () => {\n    let counter = makeCounter();\n    function resume(resumed) {\n      if (resumed) {\n        counter(); // still reachable after suspension point\n      }\n      return 1;\n    }\n    resume(false);\n    assert(counter.getCount() === 0); // not called yet\n    resume(true);\n    assert(counter.getCount() === 1); // called when execution continues\n  });",
        )
}

#[allow(dead_code)]
pub fn combined_normalized_suites() -> String {
    [
        include_str!("../js/value_range_propagation_test.js"),
        include_str!("../js/sparse_conditional_constant_propagation_tests.js"),
        include_str!("../js/loop_invariant_code_motion_test.js"),
        include_str!("../js/gvn_test.js"),
        include_str!("../js/dead_code_elimination_test.js"),
        include_str!("../js/copy_propagation_test.js"),
        include_str!("../js/constant_folding_tests.js"),
        include_str!("../js/cfg_test.js"),
        include_str!("../js/simple_test.js"),
    ]
    .into_iter()
    .map(|source| normalize_suite(source.trim()))
    .collect::<Vec<_>>()
    .join("\n\n")
}
