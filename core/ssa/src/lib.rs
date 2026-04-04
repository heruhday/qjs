mod analysis;
mod builder;
mod bytecode_fallback;
mod ir;
mod jit;
#[allow(unused)]
mod optimization;
mod passes;
mod semantics;

pub use analysis::{DominanceInfo, analyze_dominance};
pub use builder::{PhiNode, SSABlock, SSAForm, SSAInstr, SSAValue, build_ssa};
pub use bytecode_fallback::optimize_mixed_bytecode;
pub use ir::{
    BytecodeLoweringError, IRBinaryOp, IRBlock, IRCondition, IRFunction, IRInst, IRTerminator,
    IRUnaryOp, IRValue,
};
pub use jit::{
    JitLowering, MachineBlock, MachineBlockId, MachineCode, MachineCodeEmitter, MachineFunction,
    MachineInst, MachineOpcode, MachineOperand, MachineReg, MachineTerminator, RegAllocMapping,
    RegAllocResult, RegisterAllocator, SpillSlot,
};
pub use optimization::{
    OptTier, Optimizer, coalesce_registers, constant_fold, copy_propagation, eliminate_dead_code,
    fold_temporary_checks, loop_invariant_code_motion, optimize_basic_peephole, optimize_bytecode,
    optimize_ir, optimize_superinstructions, optimize_tier0, optimize_tier2, optimize_to_bytecode,
    reuse_registers_linear_scan, run_fixed_point_round, run_until_stable, simplify_branches,
};
pub use passes::{
    AliasAnalysis, AliasResult, BlockLayoutOptimization, CfgSimplification, ConstantFolding,
    CopyPropagation, DeadCodeElimination, EscapeAnalysis, EscapeKind, GlobalValueNumbering,
    InductionVariable, InductionVariableOptimization, InlineHeuristics, InlineSite, Inlining,
    InliningSummary, LoadElimination, LoopInvariantCodeMotion, LoopUnrolling, LoopUnswitching,
    Pass, PassManager, ScalarReplacement, SparseConditionalConstantPropagation, StoreElimination,
    StrengthReduction, ValueRangePropagation,
};
