use cfg::CFG;
use codegen::CompiledBytecode;
use ssa::{IRFunction, IRInst, build_ssa};
use value::{JSValue, make_number, to_f64};

pub fn optimize_bytecode(bytecode: Vec<u32>, constants: Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>) {
    optimize_segment(bytecode, constants)
}

pub fn optimize_compiled(mut compiled: CompiledBytecode) -> CompiledBytecode {
    let (bytecode, constants) = optimize_program_parts(
        compiled.bytecode,
        compiled.constants,
        &compiled.function_constants,
    );
    compiled.bytecode = bytecode;
    compiled.constants = constants;
    compiled
}

pub(crate) fn optimize_program_parts(
    bytecode: Vec<u32>,
    mut constants: Vec<JSValue>,
    function_constants: &[u16],
) -> (Vec<u32>, Vec<JSValue>) {
    let mut function_entries = function_constants
        .iter()
        .filter_map(|&slot| {
            constants
                .get(slot as usize)
                .and_then(|value| to_f64(*value))
                .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
                .map(|entry| (slot, entry as usize))
        })
        .filter(|(_, entry_pc)| *entry_pc < bytecode.len())
        .collect::<Vec<_>>();
    function_entries.sort_by_key(|&(_, entry_pc)| entry_pc);

    if function_entries.is_empty() {
        return optimize_segment(bytecode, constants);
    }

    let original_bytecode = bytecode;
    let mut optimized = Vec::new();
    let mut cursor = 0usize;

    for (index, &(slot, entry_pc)) in function_entries.iter().enumerate() {
        if entry_pc > cursor {
            let (segment, next_constants) =
                optimize_segment(original_bytecode[cursor..entry_pc].to_vec(), constants);
            optimized.extend(segment);
            constants = next_constants;
        }

        let next_entry = function_entries
            .get(index + 1)
            .map(|&(_, next_entry)| next_entry)
            .unwrap_or(original_bytecode.len());
        let new_entry = optimized.len();
        if let Some(constant) = constants.get_mut(slot as usize) {
            *constant = make_number(new_entry as f64);
        }

        let (segment, next_constants) =
            optimize_segment(original_bytecode[entry_pc..next_entry].to_vec(), constants);
        optimized.extend(segment);
        constants = next_constants;
        cursor = next_entry;
    }

    if cursor < original_bytecode.len() {
        let (segment, next_constants) =
            optimize_segment(original_bytecode[cursor..].to_vec(), constants);
        optimized.extend(segment);
        constants = next_constants;
    }

    (optimized, constants)
}

fn optimize_segment(bytecode: Vec<u32>, constants: Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>) {
    if bytecode.is_empty() {
        return (bytecode, constants);
    }

    let original_bytecode = bytecode.clone();
    let original_constants = constants.clone();

    let Ok(cfg) = CFG::from_parts(bytecode, constants, 0) else {
        return (original_bytecode, original_constants);
    };
    let ssa = build_ssa(cfg, usize::from(u8::MAX) + 1);
    let ir = ssa.to_ir();
    if contains_opaque_bytecode(&ir) {
        return (original_bytecode, original_constants);
    }

    match ir.into_bytecodes() {
        Ok(optimized) => optimized,
        Err(_) => (original_bytecode, original_constants),
    }
}

fn contains_opaque_bytecode(ir: &IRFunction) -> bool {
    ir.blocks.iter().any(|block| {
        block
            .instructions
            .iter()
            .any(|inst| matches!(inst, IRInst::Bytecode { .. }))
    })
}
