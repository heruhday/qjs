use super::*;

pub fn fold_temporary_checks(
    bytecode: Vec<u32>,
    mut constants: Vec<JSValue>,
) -> (Vec<u32>, Vec<JSValue>) {
    let mut insts = decode_program(&bytecode);
    let leaders = collect_block_leaders(&insts, &constants);
    let mut changed = false;

    for (block_index, &start) in leaders.iter().enumerate() {
        if start >= insts.len() {
            continue;
        }
        let end = leaders
            .get(block_index + 1)
            .copied()
            .unwrap_or(insts.len())
            .min(insts.len());
        if fold_temporary_checks_block(&mut insts, &mut constants, start, end) {
            changed = true;
        }
    }

    if !changed {
        return (bytecode, constants);
    }
    encode_program(&insts, constants)
}
