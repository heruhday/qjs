use super::*;

pub(super) fn run_fixed_point_round(
    bytecode: Vec<u32>,
    constants: Vec<JSValue>,
) -> (Vec<u32>, Vec<JSValue>) {
    let (bytecode, constants) = super::constant_fold::constant_fold(bytecode, constants);
    let (bytecode, constants) =
        super::fold_temporary_checks::fold_temporary_checks(bytecode, constants);
    let (bytecode, constants) = super::coalesce_registers::coalesce_registers(bytecode, constants);
    let (bytecode, constants) = super::copy_propagation::copy_propagation(bytecode, constants);
    let (bytecode, constants) =
        super::eliminate_dead_code::eliminate_dead_code(bytecode, constants);
    let (bytecode, constants) =
        super::optimize_basic_peephole::optimize_basic_peephole(bytecode, constants);
    super::simplify_branches::simplify_branches(bytecode, constants)
}
