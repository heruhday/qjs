use std::collections::HashMap;

use crate::ir::{IRFunction, IRInst, IRValue};
use crate::passes::Pass;

use super::alias::{AliasAnalysis, AliasResult};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LoadElimination;

impl LoadElimination {
    /// Eliminate redundant load operations using alias analysis
    pub fn run_with_alias(&self, ir: &mut IRFunction, alias: &AliasAnalysis) -> bool {
        let mut changed = false;

        // Track values in memory (map of "memory location" to last loaded value)
        // Conservative: we track what we know hasn't been modified
        let mut value_store: HashMap<String, IRValue> = HashMap::new();

        for block in &mut ir.blocks {
            // Clear value store at block boundaries (conservative)
            value_store.clear();

            for inst in &mut block.instructions {
                match inst {
                    IRInst::Binary { op, dst, lhs, rhs } => {
                        // Check if we've seen this exact operation before with same operands
                        let op_key = format!("{:?}_{:?}_{:?}", op, lhs, rhs);

                        // If alias analysis says no alias with stored values, we can reuse
                        if let Some(prev_val) = value_store.get(&op_key) {
                            // Check if operands don't alias with anything that might have changed
                            if !self.could_alias(lhs, prev_val, alias)
                                && !self.could_alias(rhs, prev_val, alias)
                            {
                                // Safe to replace: operands haven't changed
                                *inst = IRInst::Mov {
                                    dst: dst.clone(),
                                    src: prev_val.clone(),
                                };
                                changed = true;
                                continue;
                            }
                        }

                        // Store this computation
                        value_store.insert(op_key, dst.clone());
                    }

                    IRInst::Mov { dst, src } => {
                        // Record value copies for potential elimination
                        let key = format!("mov_{:?}", src);
                        value_store.insert(key, dst.clone());
                    }

                    // Loads from unknown sources or stores clear our tracking
                    IRInst::Bytecode { .. } => {
                        value_store.clear();
                    }

                    _ => {
                        // Other instructions might modify memory
                    }
                }
            }
        }

        changed
    }

    /// Check if two values could alias according to alias analysis
    fn could_alias(&self, lhs: &IRValue, rhs: &IRValue, alias: &AliasAnalysis) -> bool {
        matches!(
            alias.query(lhs, rhs),
            AliasResult::MayAlias | AliasResult::MustAlias
        )
    }
}

impl Pass for LoadElimination {
    fn name(&self) -> &'static str {
        "LoadElimination"
    }

    fn run(&self, _ir: &mut IRFunction) -> bool {
        false
    }
}
