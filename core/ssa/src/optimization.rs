// Adding a new numeric specialization pass for optimizing numeric binary operations

fn specialize_numeric_operations() {
    // Check both operands are numeric
    if !is_numeric(operand1) || !is_numeric(operand2) {
        return;
    }

    // Extract register values
    let reg1 = extract_register(operand1);
    let reg2 = extract_register(operand2);

    // Emit optimized bytecode instructions
    emit_instruction("AddF64Fast", reg1, reg2);
    // More instructions for SubF64Fast, MulF64Fast goes here...
}

// Inserting the pass into optimize_tier1
fn optimize_tier1() {
    // Existing passes
    loop_invariant_code_motion();
    specialize_numeric_operations(); // Inserted here
    // Other passes...
}

// Fixing existing bug in try_fuse_instruction_superinstruction
fn try_fuse_instruction_superinstruction() {
    // Other code...
    
    if both_numeric { // Updated condition
        emit_specialized_ops(); // Emit specialized ops for numeric values
    }
    // Other code...
}