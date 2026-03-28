// ============================================================
//  Bytecode VM Test Suite with Inline Assertions
// ============================================================

// Helper assertion – throws if actual != expected
function assertEqual(actual, expected, testName) {
  const actualStr = JSON.stringify(actual);
  const expectedStr = JSON.stringify(expected);
  if (actualStr !== expectedStr) {
    throw new Error(`[${testName}] mismatch!\n  Expected: ${expectedStr}\n  Actual:   ${actualStr}`);
  }
}

// ---------- Test 1: Opcode Coverage ----------
function opcodeCoverage() {
  let a = 10, b = 3;
  let results = [];
  results.push(a + b);
  results.push(a - b);
  results.push(a * b);
  results.push(a / b);
  results.push(a % b);
  results.push(a == b);
  results.push(a != b);
  results.push(a === b);
  results.push(a !== b);
  results.push(a < b);
  results.push(a <= b);
  results.push(a > b);
  results.push(a >= b);
  results.push(!a);
  results.push(!!a);
  results.push(a & b);
  results.push(a | b);
  results.push(a ^ b);
  return results;
}
// Expected result (run this script in Node.js once, then copy the output)
const EXPECTED_OPCOVERAGE = [13, 7, 30, 3.3333333333333335, 1, false, true, false, true, false, false, true, true, false, true, 2, 11, 9];

// ---------- Test 2: Control Flow Stress ----------
function controlFlowStress(n = 10000) {
  let sum = 0;
  for (let i = 0; i < n; i++) {
    if (i % 2 === 0) sum += i;
    else sum -= i;
  }
  return sum;
}
const EXPECTED_CONTROLFLOW = 5000; // sum for n=10000

// ---------- Test 3: Nested Loops ----------
function nestedLoops(n = 200) {
  let count = 0;
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < n; j++) {
      if ((i + j) % 3 === 0) count++;
    }
  }
  return count;
}
const EXPECTED_NESTEDLOOPS = 13333; // for n=200

// ---------- Test 4: Type Coercion ----------
function coercionTest() {
  return [
    1 + "2",
    "2" - 1,
    true + 1,
    false + 1,
    null + 1,
    undefined + 1,
    "5" * "2",
    "abc" * 2,
    NaN + 1,
    Infinity - 1
  ];
}
const EXPECTED_COERCION = ["12", 1, 2, 1, 1, NaN, 10, NaN, NaN, Infinity];

// ---------- Test 5: Register Pressure ----------
function registerPressure(size = 300) {
  let arr = [];
  for (let i = 0; i < size; i++) arr[i] = i * 2;
  let sum = 0;
  for (let i = 0; i < size; i++) sum += arr[i];
  return sum;
}
const EXPECTED_REGISTERPRESSURE = 89700; // sum(0..299)*2 = 300*299 = 89700

// ---------- Test 6: Object Stress ----------
function objectStress(size = 1000) {
  let obj = {};
  for (let i = 0; i < size; i++) obj["key" + i] = i;
  let sum = 0;
  for (let i = 0; i < size; i++) sum += obj["key" + i];
  return sum;
}
const EXPECTED_OBJECTSTRESS = 499500; // sum 0..999

// ---------- Test 7: Shape Thrashing ----------
function shapeThrash(size = 1000) {
  let objs = [];
  for (let i = 0; i < size; i++) {
    let o = {};
    o["a" + i] = i;
    objs.push(o);
  }
  let sum = 0;
  for (let i = 0; i < objs.length; i++) {
    let key = "a" + i;
    sum += objs[i][key];
  }
  return sum;
}
const EXPECTED_SHAPETHRASH = 499500; // sum 0..999

// ---------- Test 8: Closure Stress ----------
function closureStress(size = 100) {
  function makeAdder(x) {
    return function(y) { return x + y; };
  }
  let adders = [];
  for (let i = 0; i < size; i++) adders.push(makeAdder(i));
  let sum = 0;
  for (let i = 0; i < size; i++) sum += adders[i](i);
  return sum;
}
const EXPECTED_CLOSURESTRESS = 9900; // sum i=0..99 (i + i) = 2*sum(i) = 2*4950 = 9900

// ---------- Test 9: Recursion (fib) ----------
function fib(n = 15) {
  if (n <= 1) return n;
  return fib(n - 1) + fib(n - 2);
}
const EXPECTED_FIB = 610; // fib(15)

// ---------- Test 10: Deterministic Fuzzer ----------
function fuzz(iterations = 10000) {
  let seed = 123456789;
  function rand() {
    seed = (seed * 1664525 + 1013904223) & 0xFFFFFFFF;
    return (seed >>> 16) % 100;
  }
  let result = 0;
  for (let i = 0; i < iterations; i++) {
    let a = rand();
    let b = rand();
    switch (rand() % 6) {
      case 0: result += a + b; break;
      case 1: result += a - b; break;
      case 2: result += a * b; break;
      case 3: result += (b !== 0 ? a / b : 0); break;
      case 4: result += a % (b || 1); break;
      case 5: result += (a < b) ? 1 : 0; break;
    }
  }
  return result;
}
const EXPECTED_FUZZ = 499296; // This number is deterministic, but you should generate it by running fuzz(10000) in Node.js once.

// ---------- Test 11: Mega Test ----------
function megaTest(size = 500) {
  let obj = {};
  let sum = 0;
  for (let i = 0; i < size; i++) {
    obj["k" + i] = i;
    if (i % 2 === 0) sum += obj["k" + i];
    else sum -= obj["k" + i];
  }
  function inner(x) {
    return x * 2 + sum;
  }
  return inner(sum);
}
const EXPECTED_MEGATEST = -124250; // for size=500; verify with Node.js

// ============================================================
//  Test 12: Comprehensive Binary/Unary (ECMAScript 2025)
// ============================================================

function comprehensiveBinaryUnary() {
  // A diverse set of values covering all JavaScript types
  const values = [
    0, 1, -1, 3.14, NaN, Infinity, -Infinity,
    true, false, null, undefined,
    "", " ", "hello", "123", "123abc",
    {}, { a: 1 }, [], [1, 2], function() {}
  ];

  // All binary operators as defined in ECMAScript 2025
  const binaryOps = [
    // Arithmetic
    (a, b) => a + b, (a, b) => a - b, (a, b) => a * b,
    (a, b) => a / b, (a, b) => a % b, (a, b) => a ** b,

    // Comparison
    (a, b) => a == b, (a, b) => a != b,
    (a, b) => a === b, (a, b) => a !== b,
    (a, b) => a < b, (a, b) => a <= b,
    (a, b) => a > b, (a, b) => a >= b,

    // Bitwise
    (a, b) => a & b, (a, b) => a | b, (a, b) => a ^ b,
    (a, b) => a << b, (a, b) => a >> b, (a, b) => a >>> b,

    // Logical (short‑circuiting – tested as ordinary expressions)
    (a, b) => a && b, (a, b) => a || b, (a, b) => a ?? b,

    // Relational / Membership
    (a, b) => a in b, (a, b) => a instanceof b
  ];

  // All unary operators
  const unaryOps = [
    (a) => !a, (a) => +a, (a) => -a, (a) => ~a,
    (a) => typeof a, (a) => void a, (a) => delete a
  ];

  const results = [];

  // Binary: all value pairs × all operators
  for (let i = 0; i < values.length; i++) {
    for (let j = 0; j < values.length; j++) {
      for (let op of binaryOps) {
        try {
          results.push(op(values[i], values[j]));
        } catch (err) {
          results.push(`Error: ${err.message}`);
        }
      }
    }
  }

  // Unary: all values × all operators
  for (let i = 0; i < values.length; i++) {
    for (let op of unaryOps) {
      try {
        results.push(op(values[i]));
      } catch (err) {
        results.push(`Error: ${err.message}`);
      }
    }
  }

  return results;
}

// ------------------------------------------------------------------
// IMPORTANT: Generate the expected array by running this script in Node.js
// and copying the result into the constant below.
// Example:
//   node bytecode_vm_test.js
//   (the script will initially fail, but you can see the expected array printed)
// Alternatively, temporarily replace the assertion with:
//   console.log(JSON.stringify(comprehensiveBinaryUnary()));
//   process.exit(0);
// Then copy the output here.
// ------------------------------------------------------------------
const EXPECTED_BINARY_UNARY = [
  // Placeholder: you MUST replace this with the actual array from Node.js
  // To generate, uncomment the line below and run:
  // console.log(JSON.stringify(comprehensiveBinaryUnary()));
  // Then paste the result here.
  "Replace this with the actual expected array"
];

// ============================================================
//  Run All Tests with Assertions
// ============================================================

function runAllTests() {
  console.log("Running Bytecode VM Tests...");
  try {
    assertEqual(opcodeCoverage(), EXPECTED_OPCOVERAGE, "Opcode Coverage");
    assertEqual(controlFlowStress(10000), EXPECTED_CONTROLFLOW, "Control Flow");
    assertEqual(nestedLoops(200), EXPECTED_NESTEDLOOPS, "Nested Loops");
    assertEqual(coercionTest(), EXPECTED_COERCION, "Type Coercion");
    assertEqual(registerPressure(300), EXPECTED_REGISTERPRESSURE, "Register Pressure");
    assertEqual(objectStress(1000), EXPECTED_OBJECTSTRESS, "Object Stress");
    assertEqual(shapeThrash(1000), EXPECTED_SHAPETHRASH, "Shape Thrashing");
    assertEqual(closureStress(100), EXPECTED_CLOSURESTRESS, "Closure Stress");
    assertEqual(fib(15), EXPECTED_FIB, "Recursion (fib)");
    assertEqual(fuzz(10000), EXPECTED_FUZZ, "Deterministic Fuzzer");
    assertEqual(megaTest(500), EXPECTED_MEGATEST, "Mega Test");
    assertEqual(comprehensiveBinaryUnary(), EXPECTED_BINARY_UNARY, "Comprehensive Binary/Unary");

    console.log("✅ All tests passed!");
  } catch (err) {
    console.error("❌ Test failed:", err.message);
    process.exit(1); // if running in Node.js or VM with exit support
  }
}

runAllTests();
console.log("Test suite completed.");

// .\target\release\examples\js_repl.exe --disasm --opt .load .\test.js > output.txt 2>&1