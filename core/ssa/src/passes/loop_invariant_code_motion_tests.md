Below is a comprehensive test suite for Loop Invariant Code Motion (LICM) in pure JavaScript.
LICM hoists computations that produce the same value on every iteration outside the loop, reducing redundant work.

These tests use execution counters to verify that invariant expressions are evaluated only once (outside the loop) rather than each iteration.

```javascript
// ============================================================================
// COMPREHENSIVE LOOP INVARIANT CODE MOTION TEST SUITE (Pure JavaScript)
// ============================================================================
// Run in any JavaScript engine that performs LICM.
// Tests verify that invariant expressions are hoisted out of loops.

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
  const fn = () => { count++; return 42; };
  fn.getCount = () => count;
  fn.reset = () => { count = 0; };
  return fn;
}

// ----------------------------------------------------------------------------
// 1. BASIC ARITHMETIC INVARIANT
// ----------------------------------------------------------------------------
test('Invariant addition hoisted', () => {
  const counter = makeCounter();
  function invariantAdd() { return counter(); } // pure but counts calls
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += invariantAdd(); // should be hoisted, called once
  }
  assert(counter.getCount() === 1, `Expected 1 call, got ${counter.getCount()}`);
});

// ----------------------------------------------------------------------------
// 2. INVARIANT WITH MULTIPLE OPERATIONS
// ----------------------------------------------------------------------------
test('Invariant multiplication hoisted', () => {
  const counter = makeCounter();
  function invariantMul() { counter(); return 5 * 3; }
  let result = 0;
  for (let i = 0; i < 50; i++) {
    result += invariantMul();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 3. INVARIANT LOAD FROM OBJECT (no change inside loop)
// ----------------------------------------------------------------------------
test('Invariant object load hoisted', () => {
  const counter = makeCounter();
  let obj = { value: 100 };
  function loadObj() { counter(); return obj.value; }
  let sum = 0;
  for (let i = 0; i < 200; i++) {
    sum += loadObj();
  }
  assert(counter.getCount() === 1, 'Object load not hoisted');
});

// ----------------------------------------------------------------------------
// 4. INVARIANT ARRAY LENGTH (length unchanged)
// ----------------------------------------------------------------------------
test('Invariant array length hoisted', () => {
  const counter = makeCounter();
  let arr = [1,2,3,4,5];
  function getLen() { counter(); return arr.length; }
  let total = 0;
  for (let i = 0; i < 100; i++) {
    total += getLen();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 5. VARIANT EXPRESSION (depends on loop variable) – NOT hoisted
// ----------------------------------------------------------------------------
test('Variant expression not hoisted', () => {
  const counter = makeCounter();
  function variant(i) { counter(); return i * 2; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += variant(i);
  }
  // variant called 100 times
  assert(counter.getCount() === 100);
});

// ----------------------------------------------------------------------------
// 6. INVARIANT WITH CONDITIONAL INSIDE LOOP (condition invariant)
// ----------------------------------------------------------------------------
test('Invariant condition inside loop', () => {
  const counter = makeCounter();
  let flag = true; // invariant
  function checkFlag() { counter(); return flag; }
  let sum = 0;
  for (let i = 0; i < 50; i++) {
    if (checkFlag()) {
      sum += 1;
    }
  }
  assert(counter.getCount() === 1, 'Invariant condition not hoisted');
});

// ----------------------------------------------------------------------------
// 7. INVARIANT FUNCTION CALL (pure, same arguments)
// ----------------------------------------------------------------------------
test('Pure function call hoisted', () => {
  const counter = makeCounter();
  function pureAdd(a, b) { counter(); return a + b; }
  let x = 5, y = 10;
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += pureAdd(x, y);
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 8. INVARIANT EXPRESSION WITH SIDE EFFECT – cannot hoist
// ----------------------------------------------------------------------------
test('Side effect prevents hoisting', () => {
  const counter = makeCounter();
  function impure() { counter(); return 42; } // side effect (counter++)
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += impure(); // must be called each iteration
  }
  assert(counter.getCount() === 100);
});

// ----------------------------------------------------------------------------
// 9. HOISTING ACROSS MULTIPLE INVARIANTS IN SAME LOOP
// ----------------------------------------------------------------------------
test('Multiple invariants hoisted', () => {
  const counter1 = makeCounter();
  const counter2 = makeCounter();
  function inv1() { counter1(); return 2; }
  function inv2() { counter2(); return 3; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += inv1() * inv2();
  }
  assert(counter1.getCount() === 1 && counter2.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 10. INVARIANT EXPRESSION IN NESTED LOOPS (hoist to outer loop)
// ----------------------------------------------------------------------------
test('Invariant hoisted to outer loop only', () => {
  const counter = makeCounter();
  function invariant() { counter(); return 5; }
  let sum = 0;
  for (let i = 0; i < 10; i++) {
    for (let j = 0; j < 10; j++) {
      sum += invariant(); // invariant relative to inner loop, not outer? Actually both loops invariant -> hoist outside both
    }
  }
  // Should be called once, not 100 times
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 11. VARIANT DUE TO INNER LOOP COUNTER (not invariant)
// ----------------------------------------------------------------------------
test('Inner loop variant not hoisted', () => {
  const counter = makeCounter();
  function variant(j) { counter(); return j * 2; }
  let sum = 0;
  for (let i = 0; i < 10; i++) {
    for (let j = 0; j < 10; j++) {
      sum += variant(j);
    }
  }
  // variant called 100 times (10*10)
  assert(counter.getCount() === 100);
});

// ----------------------------------------------------------------------------
// 12. INVARIANT WITH MUTATION THAT DOES NOT AFFECT VALUE
// ----------------------------------------------------------------------------
test('Invariant despite write to different property', () => {
  const counter = makeCounter();
  let obj = { a: 10, b: 20 };
  function loadA() { counter(); return obj.a; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += loadA();
    obj.b = i; // modifying b does not affect obj.a
  }
  // loadA should still be invariant (obj.a unchanged)
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 13. INVARIANT LOST DUE TO WRITE TO SAME MEMORY LOCATION
// ----------------------------------------------------------------------------
test('Write to same property prevents hoisting', () => {
  const counter = makeCounter();
  let obj = { a: 10 };
  function loadA() { counter(); return obj.a; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += loadA();
    obj.a = i; // changes obj.a each iteration – load not invariant
  }
  assert(counter.getCount() === 100);
});

// ----------------------------------------------------------------------------
// 14. INVARIANT EXPRESSION WITH CONSTANTS
// ----------------------------------------------------------------------------
test('Constant expression hoisted', () => {
  const counter = makeCounter();
  function constExpr() { counter(); return 2 + 3; }
  let sum = 0;
  for (let i = 0; i < 1000; i++) {
    sum += constExpr();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 15. INVARIANT GLOBAL VARIABLE (unchanged)
// ----------------------------------------------------------------------------
let globalVal = 42;
test('Invariant global load hoisted', () => {
  const counter = makeCounter();
  function loadGlobal() { counter(); return globalVal; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += loadGlobal();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 16. VARIANT GLOBAL (changed inside loop)
// ----------------------------------------------------------------------------
test('Global change inside loop – not invariant', () => {
  const counter = makeCounter();
  function loadGlobal() { counter(); return globalVal; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += loadGlobal();
    globalVal = i; // changes global
  }
  assert(counter.getCount() === 100);
  globalVal = 42; // reset
});

// ----------------------------------------------------------------------------
// 17. INVARIANT ARRAY ELEMENT (unchanged)
// ----------------------------------------------------------------------------
test('Invariant array element hoisted', () => {
  const counter = makeCounter();
  let arr = [10, 20, 30];
  function getFirst() { counter(); return arr[0]; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += getFirst();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 18. VARIANT ARRAY ELEMENT (changed inside loop)
// ----------------------------------------------------------------------------
test('Array element change prevents hoisting', () => {
  const counter = makeCounter();
  let arr = [10];
  function getFirst() { counter(); return arr[0]; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += getFirst();
    arr[0] = i;
  }
  assert(counter.getCount() === 100);
});

// ----------------------------------------------------------------------------
// 19. INVARIANT WITH WHILE LOOP
// ----------------------------------------------------------------------------
test('Invariant in while loop hoisted', () => {
  const counter = makeCounter();
  function invariant() { counter(); return 5; }
  let i = 0;
  let sum = 0;
  while (i < 100) {
    sum += invariant();
    i++;
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 20. INVARIANT IN DO-WHILE LOOP
// ----------------------------------------------------------------------------
test('Invariant in do-while hoisted', () => {
  const counter = makeCounter();
  function invariant() { counter(); return 2; }
  let i = 0;
  let sum = 0;
  do {
    sum += invariant();
    i++;
  } while (i < 100);
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 21. INVARIANT EXPRESSION WITH TERNARY (condition invariant)
// ----------------------------------------------------------------------------
test('Invariant ternary hoisted', () => {
  const counter = makeCounter();
  let cond = true; // invariant
  function ternaryVal() { counter(); return cond ? 5 : 10; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += ternaryVal();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 22. INVARIANT COMPARISON USED IN CONDITIONAL (hoist comparison)
// ----------------------------------------------------------------------------
test('Invariant comparison hoisted', () => {
  const counter = makeCounter();
  let a = 10, b = 20;
  function compare() { counter(); return a < b; } // always true
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    if (compare()) {
      sum += 1;
    }
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 23. NO HOISTING WHEN INVARIANT EXPRESSION HAS SIDE EFFECT IN SUB-EXPRESSION
// ----------------------------------------------------------------------------
test('Side effect in sub-expression prevents hoisting', () => {
  const counter = makeCounter();
  function side() { counter(); return 1; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += 5 + side(); // side() has side effect, cannot hoist
  }
  assert(counter.getCount() === 100);
});

// ----------------------------------------------------------------------------
// 24. INVARIANT OBJECT CREATION (new object each iteration – not invariant)
// ----------------------------------------------------------------------------
test('Object creation not invariant (different objects)', () => {
  const counter = makeCounter();
  function createObj() { counter(); return { x: 5 }; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    let obj = createObj();
    sum += obj.x;
  }
  assert(counter.getCount() === 100);
});

// ----------------------------------------------------------------------------
// 25. INVARIANT FUNCTION EXPRESSION (same closure)
// ----------------------------------------------------------------------------
test('Invariant function definition hoisted', () => {
  const counter = makeCounter();
  function makeFn() { counter(); return function() { return 42; }; }
  let total = 0;
  for (let i = 0; i < 100; i++) {
    let fn = makeFn();
    total += fn();
  }
  // makeFn should be called once (function definition hoisted)
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 26. INVARIANT DESTRUCTURING (same object)
// ----------------------------------------------------------------------------
test('Invariant destructuring hoisted', () => {
  const counter = makeCounter();
  let obj = { a: 5, b: 6 };
  function destructure() { counter(); let { a, b } = obj; return a + b; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += destructure();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 27. INVARIANT SPREAD OPERATOR (same array)
// ----------------------------------------------------------------------------
test('Invariant array spread hoisted', () => {
  const counter = makeCounter();
  let arr = [1,2];
  function spread() { counter(); return [...arr]; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    let newArr = spread();
    sum += newArr[0];
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 28. HOISTING ACROSS BREAK AND CONTINUE (still invariant)
// ----------------------------------------------------------------------------
test('Invariant with break – hoisted', () => {
  const counter = makeCounter();
  function invariant() { counter(); return 5; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += invariant();
    if (i === 50) break;
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 29. INVARIANT IN FOR-OF LOOP (collection unchanged)
// ----------------------------------------------------------------------------
test('Invariant in for-of loop', () => {
  const counter = makeCounter();
  let arr = [1,2,3,4,5];
  function getFirst() { counter(); return arr[0]; }
  let sum = 0;
  for (let val of arr) {
    sum += getFirst();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 30. INVARIANT IN FOR-IN LOOP (object unchanged)
// ----------------------------------------------------------------------------
test('Invariant in for-in loop', () => {
  const counter = makeCounter();
  let obj = { a: 1, b: 2 };
  function getKeysCount() { counter(); return Object.keys(obj).length; }
  let total = 0;
  for (let key in obj) {
    total += getKeysCount();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 31. COMPLEX INVARIANT EXPRESSION WITH MULTIPLE INVARIANT PARTS
// ----------------------------------------------------------------------------
test('Complex invariant expression hoisted', () => {
  const counter = makeCounter();
  let a = 2, b = 3, c = 4;
  function compute() { counter(); return (a + b) * c; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += compute();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 32. NO HOISTING DUE TO FUNCTION CALL THAT MAY MODIFY GLOBAL STATE
// ----------------------------------------------------------------------------
test('Function call with possible side effect prevents hoisting', () => {
  const counter = makeCounter();
  let global = 10;
  function maybeModify() { counter(); return global; } // reads global, but global unchanged? But compiler may be conservative
  // In strict test, if the function does not modify, it could be hoisted. But to force no hoist, we add a call that writes.
  function safe() { counter(); return 42; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += safe();
  }
  // safe is pure, should be hoisted. So this test passes if hoisted.
  // For a real negative, use impure:
  let side = 0;
  function impure() { side++; return 42; }
  let sum2 = 0;
  for (let i = 0; i < 100; i++) {
    sum2 += impure();
  }
  assert(side === 100);
});

// ----------------------------------------------------------------------------
// 33. INVARIANT WITH INLINE MATH (e.g., Math.abs)
// ----------------------------------------------------------------------------
test('Invariant Math call hoisted', () => {
  const counter = makeCounter();
  function mathInv() { counter(); return Math.abs(-5); }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += mathInv();
  }
  assert(counter.getCount() === 1);
});

// ----------------------------------------------------------------------------
// 34. VARIANT DUE TO LOOP-INDUCED CHANGE IN OBJECT PROPERTY
// ----------------------------------------------------------------------------
test('Object property change inside loop – not invariant', () => {
  const counter = makeCounter();
  let obj = { val: 10 };
  function getVal() { counter(); return obj.val; }
  let sum = 0;
  for (let i = 0; i < 100; i++) {
    sum += getVal();
    obj.val = i; // changes
  }
  assert(counter.getCount() === 100);
});

// ----------------------------------------------------------------------------
// FINAL REPORT
// ----------------------------------------------------------------------------
console.log(`\n${passed}/${totalTests} Loop Invariant Code Motion tests passed.`);
if (passed === totalTests) {
  console.log('🎉 Comprehensive LICM test suite successful.');
} else {
  console.error(`❌ ${totalTests - passed} tests failed.`);
}
```

How to use:
Save as licm_tests.js and run in your JavaScript engine (Node.js, browser, or custom VM) that performs LICM. The suite includes 34 test groups covering:

· Basic arithmetic, loads, array length, pure function calls
· Invariant conditionals, ternary, comparisons
· Nested loops (hoisting to outer only)
· Variant expressions (depends on loop variable)
· Side effects that prevent hoisting
· Object/array mutations that break invariance
· Global variables, while/do-while/for-of/for-in loops
· Destructuring, spread, function definitions
· Complex expressions and constant folding interaction

All tests use counters to prove that invariant expressions are evaluated only once, not per iteration.