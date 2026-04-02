use super::*;

pub fn eliminate_dead_code(
    bytecode: Vec<u32>,
    constants: Vec<JSValue>,
) -> (Vec<u32>, Vec<JSValue>) {
    let mut insts = decode_program(&bytecode);
    let mut changed = run_block_pass(&mut insts, &constants, eliminate_dead_defs);
    changed |= eliminate_dead_defs_global(&mut insts, &constants);
    if !changed {
        return (bytecode, constants);
    }
    encode_program(&insts, constants)
}
