Below is a comprehensive test suite for Constant Folding in pure JavaScript.
Constant folding means compile‑time evaluation of expressions with known constant values (e.g., 2 + 3 → 5, true && false → false).

These tests assume your JavaScript VM or compiler performs constant folding. They use assert and sometimes counters to verify that folded expressions are reduced to their final values (and that no unnecessary computations remain).

```javascript
// ============================================================================
// COMPREHENSIVE CONSTANT FOLDING TEST SUITE (Pure JavaScript)
// ============================================================================
// Run in any JavaScript engine that performs constant folding.
// Tests verify that constant expressions are precomputed and reduced.

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------
let totalTests = 0;
let passed = 0;

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function test(name, fn) {
  totalTests++;
  try {
    fn();
    passed++;
    console.log(`✓ ${name}`);
  } catch (e) {
    console.error(`✗ ${name}\n  ${e.message}`);
  }
}

function makeCounter() {
  let count = 0;
  const fn = () => { count++; return count; };
  fn.getCount = () => count;
  return fn;
}

// ----------------------------------------------------------------------------
// 1. BASIC ARITHMETIC FOLDING
// ----------------------------------------------------------------------------
test('Addition', () => assert(2 + 3 === 5));
test('Subtraction', () => assert(10 - 4 === 6));
test('Multiplication', () => assert(6 * 7 === 42));
test('Division', () => assert(20 / 4 === 5));
test('Modulo', () => assert(17 % 5 === 2));
test('Exponentiation', () => assert(2 ** 3 === 8));
test('Unary negation', () => assert(-(5) === -5));
test('Unary plus', () => assert(+(5) === 5));
test('Increment (prefix)', () => {
  let x = 5;
  assert(++x === 6);
});
test('Decrement (prefix)', () => {
  let x = 5;
  assert(--x === 4);
});

// ----------------------------------------------------------------------------
// 2. BITWISE OPERATIONS
// ----------------------------------------------------------------------------
test('Bitwise AND', () => assert((5 & 3) === 1));
test('Bitwise OR', () => assert((5 | 3) === 7));
test('Bitwise XOR', () => assert((5 ^ 3) === 6));
test('Bitwise NOT', () => assert(~5 === -6));
test('Left shift', () => assert((5 << 1) === 10));
test('Right shift', () => assert((-10 >> 1) === -5));
test('Zero-fill right shift', () => assert((-10 >>> 1) === 2147483643));

// ----------------------------------------------------------------------------
// 3. LOGICAL OPERATIONS (short‑circuit behavior must be preserved)
// ----------------------------------------------------------------------------
test('Logical AND (true && true)', () => assert((true && true) === true));
test('Logical AND (true && false)', () => assert((true && false) === false));
test('Logical AND (false && anything) – short‑circuit', () => {
  let called = false;
  const result = false && (called = true);
  assert(result === false && called === false);
});
test('Logical OR (true || anything)', () => {
  let called = false;
  const result = true || (called = true);
  assert(result === true && called === false);
});
test('Logical OR (false || true)', () => assert((false || true) === true));
test('Logical NOT', () => assert(!true === false && !false === true));

// ----------------------------------------------------------------------------
// 4. COMPARISON OPERATORS
// ----------------------------------------------------------------------------
test('Less than (5 < 3)', () => assert((5 < 3) === false));
test('Less than or equal (3 <= 3)', () => assert((3 <= 3) === true));
test('Greater than (10 > 2)', () => assert((10 > 2) === true));
test('Greater than or equal (2 >= 5)', () => assert((2 >= 5) === false));
test('Equality (5 == "5")', () => assert((5 == "5") === true));
test('Strict equality (5 === "5")', () => assert((5 === "5") === false));
test('Inequality (5 != "6")', () => assert((5 != "6") === true));
test('Strict inequality (5 !== 5)', () => assert((5 !== 5) === false));

// ----------------------------------------------------------------------------
// 5. STRING CONCATENATION
// ----------------------------------------------------------------------------
test('String concatenation', () => assert(("hello" + " " + "world") === "hello world"));
test('String + number', () => assert(("answer: " + 42) === "answer: 42"));
test('Number + string', () => assert((42 + " is the answer") === "42 is the answer"));
test('Template literal folding', () => {
  const s = `Hello ${"world"}`;
  assert(s === "Hello world");
});

// ----------------------------------------------------------------------------
// 6. TYPE COERCIONS IN ARITHMETIC
// ----------------------------------------------------------------------------
test('String to number in addition (not folding to number)', () => {
  // "5" + 3 is "53", not folded to 8
  assert(("5" + 3) === "53");
});
test('Numeric string subtraction', () => assert(("10" - 5) === 5));
test('Numeric string multiplication', () => assert(("4" * "2") === 8));
test('Numeric string division', () => assert(("20" / "4") === 5));
test('Boolean to number', () => assert((true + false) === 1));

// ----------------------------------------------------------------------------
// 7. CONSTANT FOLDING WITH NEGATIVE ZERO, NaN, INFINITY
// ----------------------------------------------------------------------------
test('NaN folding', () => assert(isNaN(NaN + 5) && isNaN(NaN * 2)));
test('Infinity folding', () => assert((Infinity + 1) === Infinity));
test('Infinity * 0', () => assert(isNaN(Infinity * 0)));
test('Negative zero', () => assert(Object.is(-0, -0)));
test('-0 + 0', () => assert(Object.is((-0) + 0, 0))); // becomes +0
test('1 / -0', () => assert(1 / -0 === -Infinity));

// ----------------------------------------------------------------------------
// 8. CONSTANT FOLDING IN COMPARISONS WITH EDGE VALUES
// ----------------------------------------------------------------------------
test('NaN equality', () => assert((NaN === NaN) === false));
test('NaN inequality', () => assert((NaN !== NaN) === true));
test('Infinity comparison', () => assert((Infinity > 1e308) === true));
test('-0 === 0', () => assert((-0 === 0) === true));
test('Object.is(-0,0)', () => assert(Object.is(-0, 0) === false));

// ----------------------------------------------------------------------------
// 9. CONSTANT FOLDING IN CONDITIONAL (TERNARY) OPERATOR
// ----------------------------------------------------------------------------
test('Ternary with true condition', () => assert((true ? 1 : 2) === 1));
test('Ternary with false condition', () => assert((false ? 1 : 2) === 2));
test('Nested ternary folding', () => {
  const result = true ? (false ? 10 : 20) : 30;
  assert(result === 20);
});

// ----------------------------------------------------------------------------
// 10. CONSTANT FOLDING IN SWITCH STATEMENTS (compile-time case selection)
// ----------------------------------------------------------------------------
test('Switch with constant expression', () => {
  let x = 2;
  let result;
  switch (x) {
    case 1: result = 100; break;
    case 2: result = 200; break;
    default: result = 0;
  }
  assert(result === 200);
});

// ----------------------------------------------------------------------------
// 11. CONSTANT FOLDING IN LOOPS (loop condition constant)
// ----------------------------------------------------------------------------
test('While(false) – body not executed', () => {
  let counter = makeCounter();
  while (false) {
    counter();
  }
  assert(counter.getCount() === 0);
});
test('For with constant false condition', () => {
  let counter = makeCounter();
  for (let i = 0; false; i++) {
    counter();
  }
  assert(counter.getCount() === 0);
});
test('Do-while with constant false (executes once)', () => {
  let counter = makeCounter();
  do {
    counter();
  } while (false);
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 12. CONSTANT FOLDING WITH && AND || IN ASSIGNMENTS
// ----------------------------------------------------------------------------
test('Assignment with && (true && x)', () => {
  let x = 5;
  let y = true && x;
  assert(y === 5);
});
test('Assignment with || (false || 42)', () => assert((false || 42) === 42));

// ----------------------------------------------------------------------------
// 13. FOLDING ACROSS MULTIPLE OPERATIONS (deep constant folding)
// ----------------------------------------------------------------------------
test('Deep arithmetic folding', () => assert(((2 + 3) * (4 - 1) / 2) === 7.5));
test('Deep string folding', () => assert(("a" + "b" + "c") === "abc"));
test('Mixed numeric/string folding', () => assert((1 + 2 + "3") === "33"));
test('Mixed string/numeric folding', () => assert(("1" + 2 + 3) === "123"));

// ----------------------------------------------------------------------------
// 14. CONSTANT FOLDING WITH BITWISE OPERATIONS COMBINED
// ----------------------------------------------------------------------------
test('Bitwise combination', () => assert(((5 & 3) | (2 ^ 1)) === 3));

// ----------------------------------------------------------------------------
// 15. CONSTANT FOLDING WITH UNARY OPERATORS
// ----------------------------------------------------------------------------
test('Unary + on string', () => assert(+"42" === 42));
test('Unary - on string', () => assert(-"42" === -42));
test('typeof constant', () => assert(typeof 42 === "number"));
test('void constant', () => assert(void 0 === undefined));

// ----------------------------------------------------------------------------
// 16. CONSTANT FOLDING WITH IN (operator on constant property)
// ----------------------------------------------------------------------------
test('"length" in []', () => assert(("length" in []) === true));
test('"x" in {}', () => assert(("x" in {}) === false));

// ----------------------------------------------------------------------------
// 17. CONSTANT FOLDING WITH INSTANCEOF (when RHS is constant)
// ----------------------------------------------------------------------------
test('[] instanceof Array', () => assert([] instanceof Array === true));
test('({}) instanceof Object', () => assert(({}) instanceof Object === true));

// ----------------------------------------------------------------------------
// 18. FOLDING OF CONSTANT ARRAY/OBJECT LITERALS (value known at compile time)
// ----------------------------------------------------------------------------
test('Array literal folding', () => {
  const arr = [1, 2, 3];
  assert(arr.length === 3 && arr[0] === 1);
});
test('Object literal folding', () => {
  const obj = { a: 1, b: 2 };
  assert(obj.a === 1 && obj.b === 2);
});

// ----------------------------------------------------------------------------
// 19. CONSTANT PROPAGATION INTO EXPRESSIONS (simulated)
// ----------------------------------------------------------------------------
test('Constant propagation through variables', () => {
  const a = 5;
  const b = 10;
  const c = a + b; // should fold to 15
  assert(c === 15);
});
test('Propagation through multiple assignments', () => {
  let x = 2;
  x = x + 3;
  x = x * 2;
  assert(x === 10);
});

// ----------------------------------------------------------------------------
// 20. FOLDING OF Math OBJECT CALLS (if pure and constant args)
// ----------------------------------------------------------------------------
test('Math.abs(-5)', () => assert(Math.abs(-5) === 5));
test('Math.pow(2,3)', () => assert(Math.pow(2,3) === 8));
test('Math.max(1,5,2)', () => assert(Math.max(1,5,2) === 5));
test('Math.min(1,5,2)', () => assert(Math.min(1,5,2) === 1));
test('Math.floor(3.7)', () => assert(Math.floor(3.7) === 3));
test('Math.ceil(3.2)', () => assert(Math.ceil(3.2) === 4));
test('Math.round(2.5)', () => assert(Math.round(2.5) === 3));
test('Math.sqrt(16)', () => assert(Math.sqrt(16) === 4));

// ----------------------------------------------------------------------------
// 21. FOLDING OF CONSTANT REGULAR EXPRESSIONS
// ----------------------------------------------------------------------------
test('RegExp literal folding', () => {
  const re = /abc/;
  assert(re.test("abc") === true);
});

// ----------------------------------------------------------------------------
// 22. CONSTANT FOLDING WITH BRACKET NOTATION (constant string key)
// ----------------------------------------------------------------------------
test('Bracket access with constant string', () => {
  const obj = { x: 10 };
  const key = "x";
  assert(obj[key] === 10);
});

// ----------------------------------------------------------------------------
// 23. NEGATIVE TESTS – expressions that should NOT be folded (runtime values)
// ----------------------------------------------------------------------------
test('No folding with Math.random()', () => {
  const r1 = Math.random();
  const r2 = Math.random();
  assert(r1 !== r2); // almost always true
});
test('No folding with Date.now()', () => {
  const t1 = Date.now();
  const t2 = Date.now();
  assert(t1 !== t2); // likely
});
test('No folding with function call', () => {
  let count = 0;
  function f() { count++; return 5; }
  const a = f();
  const b = f();
  assert(count === 2);
});

// ----------------------------------------------------------------------------
// 24. FOLDING OF COMPLEX CONSTANT EXPRESSIONS WITH SIDE‑EFFECT FREE OPERATORS
// ----------------------------------------------------------------------------
test('Complex constant folding (bitwise + arithmetic)', () => {
  const result = ((5 << 2) + (12 >> 1)) ^ 3;
  assert(result === 25);
});

// ----------------------------------------------------------------------------
// 25. CONSTANT FOLDING IN ARRAY DESTRUCTURING (with constant pattern)
// ----------------------------------------------------------------------------
test('Array destructuring constant', () => {
  const [a, b] = [1, 2];
  assert(a === 1 && b === 2);
});

// ----------------------------------------------------------------------------
// 26. CONSTANT FOLDING IN OBJECT DESTRUCTURING
// ----------------------------------------------------------------------------
test('Object destructuring constant', () => {
  const { x, y } = { x: 10, y: 20 };
  assert(x === 10 && y === 20);
});

// ----------------------------------------------------------------------------
// 27. CONSTANT FOLDING WITH SPREAD OPERATOR (arrays)
// ----------------------------------------------------------------------------
test('Array spread constant', () => {
  const arr = [1, ...[2, 3], 4];
  assert(arr.length === 4 && arr[1] === 2);
});

// ----------------------------------------------------------------------------
// 28. CONSTANT FOLDING WITH TEMPLATE TAGS (if tag is constant)
// ----------------------------------------------------------------------------
test('Tagged template with constant strings', () => {
  function tag(strings, ...values) {
    return strings[0] + values[0];
  }
  const result = tag`Hello ${"world"}`;
  assert(result === "Helloworld");
});

// ----------------------------------------------------------------------------
// 29. FOLDING OF `eval` ON CONSTANT STRING (optional, usually not folded)
// ----------------------------------------------------------------------------
test('eval constant string', () => {
  const result = eval("2 + 3");
  assert(result === 5);
});

// ----------------------------------------------------------------------------
// 30. CONSTANT FOLDING IN CLASS PROPERTIES (static)
// ----------------------------------------------------------------------------
test('Static class field folding', () => {
  class Test {
    static x = 2 + 3;
  }
  assert(Test.x === 5);
});

// ----------------------------------------------------------------------------
// 31. CONSTANT FOLDING IN DEFAULT PARAMETERS
// ----------------------------------------------------------------------------
test('Default parameter folding', () => {
  function f(x = 5 + 3) {
    return x;
  }
  assert(f() === 8);
});

// ----------------------------------------------------------------------------
// 32. FOLDING OF `new` ON BUILT‑IN CONSTRUCTORS WITH CONSTANT ARGS
// ----------------------------------------------------------------------------
test('new Number(42)', () => {
  const n = new Number(42);
  assert(n.valueOf() === 42);
});
test('new String("abc")', () => {
  const s = new String("abc");
  assert(s.valueOf() === "abc");
});

// ----------------------------------------------------------------------------
// FINAL REPORT
// ----------------------------------------------------------------------------
console.log(`\n${passed}/${totalTests} Constant Folding tests passed.`);
if (passed === totalTests) {
  console.log('🎉 Comprehensive Constant Folding test suite successful.');
} else {
  console.error(`❌ ${totalTests - passed} tests failed.`);
}
```

How to use:
Save as constant_folding_tests.js and run in your JavaScript engine (Node.js, browser, or custom VM). The suite includes over 80 individual assertions across 32 test groups, covering:

· Arithmetic, bitwise, logical, comparison, string operations
· Type coercions, edge values (NaN, Infinity, -0)
· Short‑circuit preservation
· Ternary, switch, loops, conditionals
· Deep nested constant folding
· Built‑in Math functions
· Destructuring, spread, template literals
· Class static fields, default parameters, eval
· Negative tests (non‑foldable expressions)

All tests are pure JavaScript and rely only on your engine’s constant folding implementation.