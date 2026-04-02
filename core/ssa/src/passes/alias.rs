use std::collections::HashMap;

use crate::ir::{IRFunction, IRValue};
use crate::passes::Pass;

use super::escape::EscapeKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasResult {
    NoAlias,
    MayAlias,
    MustAlias,
}

#[derive(Debug, Clone, Default)]
pub struct AliasAnalysis {
    pub escape: HashMap<IRValue, EscapeKind>,
}

impl AliasAnalysis {
    pub fn new(escape: HashMap<IRValue, EscapeKind>) -> Self {
        Self { escape }
    }

    /// Analyze the IR to build alias relationships
    /// Currently uses escape analysis results to refine alias classification
    pub fn analyze(&mut self, _ir: &IRFunction) {
        // Escape analysis is passed in via constructor
        // This method can be extended to compute additional alias info from IR structure
    }

    /// Query whether two values may alias
    /// Returns:
    /// - NoAlias: definitely don't alias (e.g., two different constants)
    /// - MustAlias: definitely alias (e.g., same constant value)
    /// - MayAlias: might alias (conservative default for registers)
    pub fn query(&self, lhs: &IRValue, rhs: &IRValue) -> AliasResult {
        match (lhs, rhs) {
            // Two constants that are different don't alias
            (IRValue::Constant(c1), IRValue::Constant(c2)) => {
                if c1 == c2 {
                    AliasResult::MustAlias
                } else {
                    AliasResult::NoAlias
                }
            }

            // Constant never aliases with register
            (IRValue::Constant(_), IRValue::Register(_, _))
            | (IRValue::Register(_, _), IRValue::Constant(_)) => AliasResult::NoAlias,

            // Register-to-register: without complete escape info, be conservative
            (IRValue::Register(_, _), IRValue::Register(_, _)) => {
                // If we have escape information, use it for refinement
                if !self.escape.is_empty() {
                    let lhs_escape = self.escape.get(lhs).copied();
                    let rhs_escape = self.escape.get(rhs).copied();

                    match (lhs_escape, rhs_escape) {
                        // Both don't escape: they're local, can't alias
                        (Some(EscapeKind::NoEscape), Some(EscapeKind::NoEscape)) => AliasResult::NoAlias,

                        // One doesn't escape, other has limited escape: no alias
                        (Some(EscapeKind::NoEscape), Some(EscapeKind::ArgEscape))
                        | (Some(EscapeKind::ArgEscape), Some(EscapeKind::NoEscape)) => AliasResult::NoAlias,

                        // Other combinations: conservative may alias
                        _ => AliasResult::MayAlias,
                    }
                } else {
                    // No escape info available: conservative default
                    AliasResult::MayAlias
                }
            }
        }
    }
}

impl Pass for AliasAnalysis {
    fn name(&self) -> &'static str {
        "AliasAnalysis"
    }

    fn run(&self, _ir: &mut IRFunction) -> bool {
        false
    }
}
