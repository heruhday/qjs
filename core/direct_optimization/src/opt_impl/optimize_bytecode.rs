use super::*;

pub fn optimize_bytecode(bytecode: Vec<u32>, constants: Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>) {
    let (bytecode, constants) = super::run_until_stable::run_until_stable(
        bytecode,
        constants,
        8,
        super::run_fixed_point_round::run_fixed_point_round,
    );
    let (bytecode, constants) =
        super::reuse_registers_linear_scan::reuse_registers_linear_scan(bytecode, constants);
    let (bytecode, constants) =
        super::optimize_basic_peephole::optimize_basic_peephole(bytecode, constants);
    let (bytecode, constants) =
        super::optimize_superinstructions::optimize_superinstructions(bytecode, constants);
    let (bytecode, constants) = super::simplify_branches::simplify_branches(bytecode, constants);
    relocate_jumps(bytecode, constants)
}
