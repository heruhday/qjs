mod analysis;
mod builder;
mod ir;
mod optimization;
mod passes;
mod semantics;

pub use analysis::{DominanceInfo, analyze_dominance};
pub use builder::{PhiNode, SSABlock, SSAForm, SSAInstr, SSAValue, build_ssa};
pub use ir::{
    BytecodeLoweringError, IRBinaryOp, IRBlock, IRCondition, IRFunction, IRInst, IRTerminator,
    IRUnaryOp, IRValue,
};
pub use optimization::{
    coalesce_registers, constant_fold, copy_propagation, eliminate_dead_code,
    fold_temporary_checks, loop_invariant_code_motion, optimize_basic_peephole, optimize_bytecode,
    optimize_ir, optimize_superinstructions, optimize_to_bytecode, reuse_registers_linear_scan,
    run_fixed_point_round, run_until_stable, simplify_branches,
};
pub use passes::{
    CfgSimplification, ConstantFolding, CopyPropagation, DeadCodeElimination, GlobalValueNumbering,
    LoopInvariantCodeMotion, Pass, PassManager, SparseConditionalConstantPropagation,
    ValueRangePropagation,
};
