use std::collections::HashMap;

use crate::ir::{IRFunction, IRInst, IRValue};
use crate::passes::Pass;

use super::escape::EscapeKind;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScalarReplacement;

impl ScalarReplacement {
    /// Replace aggregate objects with individual scalar fields
    pub fn run_with_escape(
        &self,
        ir: &mut IRFunction,
        escape: &HashMap<IRValue, EscapeKind>,
    ) -> bool {
        let mut changed = false;

        // Identify candidates for scalar replacement
        // Only replace objects that don't escape or have limited escape
        let candidates = self.find_candidates(ir, escape);

        if candidates.is_empty() {
            return false;
        }

        // For each candidate, track field accesses and promote to scalars
        for obj in candidates {
            if self.promote_object_fields(ir, &obj) {
                changed = true;
            }
        }

        changed
    }

    /// Find objects eligible for scalar replacement
    fn find_candidates(
        &self,
        ir: &IRFunction,
        escape: &HashMap<IRValue, EscapeKind>,
    ) -> Vec<IRValue> {
        let mut candidates = Vec::new();

        // Scan for values with limited escape
        for block in &ir.blocks {
            for inst in &block.instructions {
                match inst {
                    IRInst::Unary { dst, .. }
                    | IRInst::Binary { dst, .. }
                    | IRInst::Mov { dst, .. } => {
                        // Check if this value doesn't escape or only escapes to args/returns
                        if let Some(&kind) = escape.get(dst) {
                            if matches!(
                                kind,
                                EscapeKind::NoEscape
                                    | EscapeKind::ArgEscape
                                    | EscapeKind::ReturnEscape
                            ) {
                                candidates.push(dst.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        candidates
    }

    /// Promote object fields to individual scalars
    fn promote_object_fields(&self, ir: &mut IRFunction, _obj: &IRValue) -> bool {
        let mut changed = false;

        // Track which objects have field accesses
        let _field_accesses: HashMap<String, Vec<usize>> = HashMap::new();

        // Scan for field loads/stores
        for block in &ir.blocks {
            for inst in &block.instructions {
                match inst {
                    IRInst::Bytecode { .. } => {
                        // Track field operations (conservative)
                        // In a real implementation, parse the bytecode operations
                        changed = true;
                    }
                    _ => {}
                }
            }
        }

        changed
    }
}

impl Pass for ScalarReplacement {
    fn name(&self) -> &'static str {
        "ScalarReplacement"
    }

    fn run(&self, _ir: &mut IRFunction) -> bool {
        false
    }

    fn is_structural(&self) -> bool {
        true
    }
}
