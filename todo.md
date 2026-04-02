## Missing Major Pieces

These are the main gaps between the current SSA optimizer and a more serious optimizing JIT:

- inlining
- escape analysis
- scalar replacement
- alias analysis
- load elimination
- store elimination
- induction variable optimization
- strength reduction
- loop unswitching
- loop unrolling
- better register allocation with spills and coalescing
- machine block layout / scheduling
- JIT machine IR lowering
- machine code emission
