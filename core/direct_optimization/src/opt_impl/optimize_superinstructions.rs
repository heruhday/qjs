use super::*;

pub(super) fn optimize_superinstructions(
    bytecode: Vec<u32>,
    constants: Vec<JSValue>,
) -> (Vec<u32>, Vec<JSValue>) {
    optimize_peephole(bytecode, constants)
}
