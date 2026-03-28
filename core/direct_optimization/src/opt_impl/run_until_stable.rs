use super::*;

pub(super) fn run_until_stable<F>(
    mut bytecode: Vec<u32>,
    mut constants: Vec<JSValue>,
    max_rounds: usize,
    mut round: F,
) -> (Vec<u32>, Vec<JSValue>)
where
    F: FnMut(Vec<u32>, Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>),
{
    for _ in 0..max_rounds {
        let prev_bytecode = bytecode.clone();
        let prev_constants = constants.clone();
        (bytecode, constants) = round(bytecode, constants);
        if bytecode == prev_bytecode && constants == prev_constants {
            break;
        }
    }
    (bytecode, constants)
}
