use crate::ir::{IRBinaryOp, IRFunction, IRInst, IRValue};
use crate::passes::Pass;
use value::JSValue;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StrengthReduction;

impl StrengthReduction {
    /// Rewrite loop arithmetic to use cheaper operations
    /// Example: multiply by power of 2 becomes bit shift
    pub fn rewrite_loop_arithmetic(&self, ir: &mut IRFunction) -> bool {
        let mut changed = false;

        for block in &mut ir.blocks {
            for inst in &mut block.instructions {
                match inst {
                    IRInst::Binary {
                        op: IRBinaryOp::Mul,
                        lhs,
                        rhs,
                        dst,
                    } => {
                        // Check if rhs is a constant we can optimize
                        if let IRValue::Constant(constant) = rhs {
                            if let Some(shift_amount) = self.is_power_of_two(constant) {
                                // Replace multiply with shift
                                *inst = IRInst::Binary {
                                    op: IRBinaryOp::Shl,
                                    lhs: lhs.clone(),
                                    rhs: IRValue::Constant(make_int32(shift_amount as i32)),
                                    dst: dst.clone(),
                                };
                                changed = true;
                            }
                        }
                    }

                    IRInst::Binary {
                        op: IRBinaryOp::Div,
                        lhs,
                        rhs,
                        dst,
                    } => {
                        // Check if rhs is a constant power of 2
                        if let IRValue::Constant(constant) = rhs {
                            if let Some(shift_amount) = self.is_power_of_two(constant) {
                                // Replace divide with shift
                                *inst = IRInst::Binary {
                                    op: IRBinaryOp::Ushr,
                                    lhs: lhs.clone(),
                                    rhs: IRValue::Constant(make_int32(shift_amount as i32)),
                                    dst: dst.clone(),
                                };
                                changed = true;
                            }
                        }
                    }

                    _ => {}
                }
            }
        }

        changed
    }

    /// Check if a JSValue is a power of 2 and return the shift amount
    /// Returns Some(shift_amount) if value is 2^n, None otherwise
    fn is_power_of_two(&self, value: &JSValue) -> Option<u32> {
        // Try to extract integer value from JSValue
        // Conservative: only handle obvious cases
        // In a real implementation, we'd have access to the value internals
        match self.extract_int64(value) {
            Some(n) if n > 0 && (n & (n - 1)) == 0 => {
                // It's a power of 2, find which power
                Some((n.trailing_zeros()) as u32)
            }
            _ => None,
        }
    }

    /// Extract i64 from JSValue if possible (conservative extraction)
    fn extract_int64(&self, _value: &JSValue) -> Option<i64> {
        // JSValue doesn't expose direct integer extraction in public API
        // Conservative: return None to be safe
        None
    }
}

/// Helper to create int32 JSValue
fn make_int32(val: i32) -> JSValue {
    use value::make_int32;
    make_int32(val)
}

impl Pass for StrengthReduction {
    fn name(&self) -> &'static str {
        "StrengthReduction"
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        self.rewrite_loop_arithmetic(ir)
    }
}
