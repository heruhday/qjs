use std::collections::{HashMap, HashSet};

use crate::ir::{IRFunction, IRInst, IRTerminator, IRValue};
use crate::passes::Pass;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeKind {
    NoEscape,
    ArgEscape,
    ReturnEscape,
    GlobalEscape,
    Unknown,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EscapeAnalysis;

impl EscapeAnalysis {
    /// Analyze IR to determine which values escape their local scope
    pub fn analyze(&self, ir: &IRFunction) -> HashMap<IRValue, EscapeKind> {
        let mut escape_map = HashMap::new();

        // Collect all values that are used in different contexts
        let mut return_values = HashSet::new();
        let mut local_values = HashSet::new();

        // Scan all blocks for value usage patterns
        for block in &ir.blocks {
            // Check instructions for value usage
            for inst in &block.instructions {
                match inst {
                    IRInst::Phi { dst, incoming } => {
                        local_values.insert(dst.clone());
                        for (_, val) in incoming {
                            self.classify_use(val, &mut local_values, &mut return_values);
                        }
                    }
                    IRInst::Mov { dst, src } => {
                        local_values.insert(dst.clone());
                        self.classify_use(src, &mut local_values, &mut return_values);
                    }
                    IRInst::LoadConst { dst, .. } => {
                        local_values.insert(dst.clone());
                    }
                    IRInst::Unary { dst, operand, .. } => {
                        local_values.insert(dst.clone());
                        self.classify_use(operand, &mut local_values, &mut return_values);
                    }
                    IRInst::Binary {
                        dst, lhs, rhs, ..
                    } => {
                        local_values.insert(dst.clone());
                        self.classify_use(lhs, &mut local_values, &mut return_values);
                        self.classify_use(rhs, &mut local_values, &mut return_values);
                    }
                    IRInst::Bytecode { uses, defs, .. } => {
                        for def in defs {
                            local_values.insert(def.clone());
                        }
                        for use_val in uses {
                            self.classify_use(use_val, &mut local_values, &mut return_values);
                        }
                    }
                    IRInst::Nop => {}
                }
            }

            // Check terminator for return values
            match &block.terminator {
                IRTerminator::Return { value } => {
                    if let Some(val) = value {
                        return_values.insert(val.clone());
                    }
                }
                IRTerminator::ConditionalReturn { value, .. } => {
                    return_values.insert(value.clone());
                }
                _ => {}
            }
        }

        // Build escape classification
        for block in &ir.blocks {
            for inst in &block.instructions {
                match inst {
                    IRInst::Phi { dst, .. }
                    | IRInst::Mov { dst, .. }
                    | IRInst::LoadConst { dst, .. }
                    | IRInst::Unary { dst, .. }
                    | IRInst::Binary { dst, .. } => {
                        let escape_kind = self.determine_escape_kind(dst, &return_values);
                        escape_map.insert(dst.clone(), escape_kind);
                    }
                    IRInst::Bytecode { defs, .. } => {
                        for def in defs {
                            let escape_kind = self.determine_escape_kind(def, &return_values);
                            escape_map.insert(def.clone(), escape_kind);
                        }
                    }
                    _ => {}
                }
            }
        }

        escape_map
    }

    /// Classify a value's use context
    fn classify_use(
        &self,
        _value: &IRValue,
        _local_values: &mut HashSet<IRValue>,
        _return_values: &mut HashSet<IRValue>,
    ) {
        // Values used in instructions are considered local unless returned
    }

    /// Determine escape kind for a value based on usage
    fn determine_escape_kind(
        &self,
        value: &IRValue,
        return_values: &HashSet<IRValue>,
    ) -> EscapeKind {
        // Constants never escape
        if let IRValue::Constant(_) = value {
            return EscapeKind::NoEscape;
        }

        // If value is returned, it has return escape
        if return_values.contains(value) {
            return EscapeKind::ReturnEscape;
        }

        // Default: conservative classification for registers
        EscapeKind::Unknown
    }

    pub fn classify_value(&self, value: &IRValue) -> EscapeKind {
        match value {
            IRValue::Constant(_) => EscapeKind::NoEscape,
            IRValue::Register(_, _) => EscapeKind::Unknown,
        }
    }
}

impl Pass for EscapeAnalysis {
    fn name(&self) -> &'static str {
        "EscapeAnalysis"
    }

    fn run(&self, _ir: &mut IRFunction) -> bool {
        false
    }
}
