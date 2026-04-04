use cfg::CFG;
use codegen::{CompiledBytecode, Opcode};
use direct_optimization::optimize_bytecode as super_optimize_bytecode;
use ssa::{IRFunction, IRInst, build_ssa, optimize_mixed_bytecode, optimize_to_bytecode};
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
    inline_immediate_root_function_call(&mut compiled);
    compiled
}

fn decode_opcode(word: u32) -> Opcode {
    Opcode::from((word & 0xFF) as u8)
}

fn decode_a(word: u32) -> u8 {
    ((word >> 8) & 0xFF) as u8
}

fn decode_b(word: u32) -> u8 {
    ((word >> 16) & 0xFF) as u8
}

fn decode_abx(word: u32) -> u16 {
    ((word >> 16) & 0xFFFF) as u16
}

fn decode_function_descriptor(value: JSValue) -> Option<(usize, bool)> {
    let entry = to_f64(value)?;
    if !entry.is_finite() || entry.fract() != 0.0 {
        return None;
    }
    if entry >= 0.0 {
        return Some((entry as usize, false));
    }

    let decoded = -entry - 1.0;
    (decoded >= 0.0).then_some((decoded as usize, true))
}

fn encode_function_descriptor(entry_pc: usize, is_async: bool) -> JSValue {
    let entry = entry_pc as f64;
    let encoded = if is_async { -(entry + 1.0) } else { entry };
    make_number(encoded)
}

fn inline_immediate_root_function_call(compiled: &mut CompiledBytecode) {
    if compiled.function_constants.len() != 1 || compiled.bytecode.len() < 5 {
        return;
    }

    let function_slot = compiled.function_constants[0];
    let Some((entry_pc, is_async)) = compiled
        .constants
        .get(function_slot as usize)
        .and_then(|value| decode_function_descriptor(*value))
    else {
        return;
    };
    if is_async {
        return;
    }

    if entry_pc != 4 || entry_pc >= compiled.bytecode.len() {
        return;
    }

    let new_func = compiled.bytecode[0];
    let set_upval = compiled.bytecode[1];
    let init_name = compiled.bytecode[2];
    let call_ret = compiled.bytecode[3];

    let callee_reg = decode_a(new_func);
    if decode_opcode(new_func) != Opcode::NewFunc
        || decode_abx(new_func) != function_slot
        || decode_opcode(set_upval) != Opcode::SetUpval
        || decode_a(set_upval) != callee_reg
        || decode_b(set_upval) != 0
        || decode_opcode(init_name) != Opcode::InitName
        || decode_a(init_name) != callee_reg
        || decode_opcode(call_ret) != Opcode::CallRet
        || decode_a(call_ret) != callee_reg
        || decode_b(call_ret) != 0
    {
        return;
    }

    let body = &compiled.bytecode[entry_pc..];
    if body.iter().any(|word| {
        matches!(
            decode_opcode(*word),
            Opcode::GetUpval | Opcode::LoadClosure | Opcode::SetUpval | Opcode::NewFunc
        )
    }) {
        return;
    }

    let (bytecode, constants) = optimize_segment(body.to_vec(), compiled.constants.clone());
    compiled.bytecode = bytecode;
    compiled.constants = constants;
    compiled.function_constants.clear();
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
                .and_then(|value| decode_function_descriptor(*value))
                .map(|(entry, is_async)| (slot, entry, is_async))
        })
        .filter(|(_, entry_pc, _)| *entry_pc < bytecode.len())
        .collect::<Vec<_>>();
    function_entries.sort_by_key(|&(_, entry_pc, _)| entry_pc);

    if function_entries.is_empty() {
        return optimize_segment(bytecode, constants);
    }

    let original_bytecode = bytecode;
    let mut optimized = Vec::new();
    let mut cursor = 0usize;

    for (index, &(slot, entry_pc, is_async)) in function_entries.iter().enumerate() {
        if entry_pc > cursor {
            let (segment, next_constants) =
                optimize_segment(original_bytecode[cursor..entry_pc].to_vec(), constants);
            optimized.extend(segment);
            constants = next_constants;
        }

        let next_entry = function_entries
            .get(index + 1)
            .map(|&(_, next_entry, _)| next_entry)
            .unwrap_or(original_bytecode.len());
        let new_entry = optimized.len();
        if let Some(constant) = constants.get_mut(slot as usize) {
            *constant = encode_function_descriptor(new_entry, is_async);
        }

        if is_async {
            optimized.extend_from_slice(&original_bytecode[entry_pc..next_entry]);
        } else {
            let (segment, next_constants) =
                optimize_segment(original_bytecode[entry_pc..next_entry].to_vec(), constants);
            optimized.extend(segment);
            constants = next_constants;
        }
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

    let optimized = if let Ok(cfg) = CFG::from_parts(bytecode, constants, 0) {
        let ssa = build_ssa(cfg, usize::from(u8::MAX) + 1);
        let ir = ssa.to_ir();
        if contains_opaque_bytecode(&ir) {
            optimize_mixed_bytecode(original_bytecode, original_constants)
        } else {
            match optimize_to_bytecode(&ir) {
                Ok(optimized) => optimized,
                Err(_) => (original_bytecode, original_constants),
            }
        }
    } else {
        (original_bytecode, original_constants)
    };

    let (bytecode, constants) = super_optimize_bytecode(optimized.0, optimized.1);

    (bytecode, constants)
}

fn contains_opaque_bytecode(ir: &IRFunction) -> bool {
    ir.blocks.iter().any(|block| {
        block
            .instructions
            .iter()
            .any(|inst| matches!(inst, IRInst::Bytecode { .. }))
    })
}

/// 🔥 Problem C: Eliminate redundant mov instructions  
/// Patterns:
/// 1. mov r_X, r_X (self-move)
/// 2. mov r_X, r_Y; ...; mov r_Y, r_X (reverse moves, if no interfering writes)
#[allow(dead_code)]
fn eliminate_redundant_moves(
    bytecode: Vec<u32>,
    constants: Vec<JSValue>,
) -> (Vec<u32>, Vec<JSValue>) {
    let mut optimized = Vec::with_capacity(bytecode.len());
    let mut skip_next_reverse: Option<(u8, u8, usize)> = None; // (prev_dst, prev_src, prev_idx)

    for word in bytecode.iter() {
        let opcode = decode_opcode(*word);

        // Skip self-moves: mov r_X, r_X
        if opcode == Opcode::Mov {
            let dst = decode_a(*word);
            let src = decode_b(*word);
            if dst == src {
                continue; // Remove self-move
            }

            // Check for reverse move pattern
            if let Some((prev_dst, prev_src, prev_idx)) = skip_next_reverse {
                let curr_dst = decode_a(*word);
                let curr_src = decode_b(*word);

                // If this is reversing the previous move: mov r1,r2; mov r2,r1
                if prev_dst == curr_src && prev_src == curr_dst {
                    // Check if there were no interfering writes between them
                    let mut interferes = false;
                    for check_idx in (prev_idx + 1)..optimized.len() {
                        let check_word = optimized[check_idx];
                        let check_opcode = decode_opcode(check_word);
                        let check_dst = decode_a(check_word);

                        // These opcodes write to a destination register
                        if matches!(
                            check_opcode,
                            Opcode::Mov
                                | Opcode::LoadK
                                | Opcode::LoadName
                                | Opcode::InitName
                                | Opcode::StoreName
                                | Opcode::LoadI
                                | Opcode::LoadArg
                                | Opcode::AddI
                                | Opcode::SubI
                                | Opcode::MulI
                                | Opcode::DivI
                                | Opcode::Inc
                                | Opcode::Dec
                        ) {
                            // If it writes to prev_dst or prev_src, it interferes
                            if check_dst == prev_dst || check_dst == prev_src {
                                interferes = true;
                                break;
                            }
                        }
                    }

                    if !interferes {
                        // Remove the previous move and skip this one
                        if let Some(_) = optimized.pop() {
                            skip_next_reverse = None;
                            continue; // Skip this reverse move too
                        }
                    }
                }
            }

            // Record this move as a potential anchor for reverse detection
            skip_next_reverse = Some((dst, src, optimized.len()));
        } else {
            skip_next_reverse = None; // Reset when we hit a non-mov
        }

        optimized.push(*word);
    }

    (optimized, constants)
}

/// 🔥 Problem A: Cache loop-invariant loads
/// Simplified: Detects common patterns and creates opportunities for caching
/// Full implementation would require SSA-level analysis or JIT quickening
#[allow(dead_code)]
fn cache_loop_invariants(bytecode: Vec<u32>, constants: Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>) {
    // This optimization requires more context than bytecode alone provides
    // The proper solution is at codegen time or via JIT quickening
    // For now, we document the pattern:
    //
    // Pattern to optimize (appears in benchmark):
    //   loop_start:
    //     load_name r1, identifier[i]      ; Loop variable
    //     load_name r2, identifier[runs]   ; Loop bound
    //     lt r1, r2
    //     jmp_false end
    //     ...  (loop body)
    //     load_name r2, identifier[fib]    ; Loop-invariant function
    //     call1 r2, 25
    //     ...
    //     jmp loop_start
    //
    // Should become:
    //     load_name r10, identifier[fib]   ; Cache before loop
    //     load_i r20, 0                     ; i = 0
    //   loop_start:
    //     load_name r2, identifier[runs]   ; Loop bound
    //     lt r20, r2
    //     ... use r10 instead of reloading fib
    //     inc r20
    //     jmp loop_start
    //
    // This optimization is better done during codegen with careful register allocation
    // Or via runtime quickening (JIT patching)
    //
    // For now, we'll leave this as documented future work

    (bytecode, constants)
}
