use super::*;

pub fn coalesce_registers(bytecode: Vec<u32>, constants: Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>) {
    let mut insts = decode_program(&bytecode);
    let changed = run_block_pass(&mut insts, &constants, |insts, start, end, _terminal| {
        coalesce_registers_block(insts, start, end)
    });
    if !changed {
        return (bytecode, constants);
    }
    encode_program(&insts, constants)
}
