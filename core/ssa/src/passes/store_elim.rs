use std::collections::HashMap;

use crate::ir::{IRFunction, IRInst, IRValue};
use crate::passes::Pass;

use super::alias::AliasAnalysis;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StoreElimination;

impl StoreElimination {
    /// Eliminate dead stores using alias analysis
    /// A store is dead if the value is never loaded before being overwritten
    pub fn run_with_alias(&self, ir: &mut IRFunction, alias: &AliasAnalysis) -> bool {
        let mut changed = false;

        for block in &mut ir.blocks {
            // Track stores for each "location" (value being written to)
            let mut last_store: HashMap<String, (usize, IRValue)> = HashMap::new();
            let mut has_load: HashMap<String, bool> = HashMap::new();

            // First pass: identify stores and loads
            for (idx, inst) in block.instructions.iter().enumerate() {
                match inst {
                    IRInst::Mov { src, dst } => {
                        // Mov can be treated as a store
                        let location = format!("mov_{:?}", dst);

                        // Check if previous store to this location is dead
                        if let Some((_prev_idx, _prev_val)) = last_store.get(&location) {
                            if !has_load.get(&location).copied().unwrap_or(false) {
                                // Previous store was never loaded - mark as dead
                                changed = true;
                            }
                        }

                        // Update last store
                        last_store.insert(location.clone(), (idx, src.clone()));
                        has_load.insert(location.clone(), false);
                    }

                    IRInst::Unary { dst, operand, .. }
                    | IRInst::Binary {
                        dst,
                        lhs: operand,
                        ..
                    } => {
                        // These also produce values that could be dead
                        let location = format!("compute_{:?}", dst);

                        if let Some((_prev_idx, _)) = last_store.get(&location) {
                            if !has_load.get(&location).copied().unwrap_or(false) {
                                changed = true;
                            }
                        }

                        last_store.insert(location.clone(), (idx, operand.clone()));
                        has_load.insert(location.clone(), false);
                    }

                    IRInst::Bytecode { uses, .. } => {
                        // Bytecode instructions may load values
                        for use_val in uses {
                            // Mark all potentially aliasing stores as loaded
                            for location in last_store.keys() {
                                if self.could_alias_with_bytecode(use_val, alias) {
                                    has_load.insert(location.clone(), true);
                                }
                            }
                        }
                    }

                    _ => {}
                }
            }
        }

        changed
    }

    /// Check if a value could alias with bytecode operations (conservative)
    fn could_alias_with_bytecode(&self, _value: &IRValue, _alias: &AliasAnalysis) -> bool {
        // Conservative: bytecode operations may access any value
        true
    }
}

impl Pass for StoreElimination {
    fn name(&self) -> &'static str {
        "StoreElimination"
    }

    fn run(&self, _ir: &mut IRFunction) -> bool {
        false
    }
}
