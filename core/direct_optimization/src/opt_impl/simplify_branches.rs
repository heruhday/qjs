use super::*;

pub fn simplify_branches(bytecode: Vec<u32>, constants: Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>) {
    let mut insts = decode_program(&bytecode);
    if !thread_jumps(&mut insts) {
        return (bytecode, constants);
    }
    encode_program(&insts, constants)
}
