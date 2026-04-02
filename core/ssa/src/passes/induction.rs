use std::collections::HashSet;

use cfg::BlockId;

use crate::ir::{IRFunction, IRInst, IRValue};
use crate::passes::Pass;


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InductionVariable {
    pub base: IRValue,
    pub step: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InductionVariableOptimization;

impl InductionVariableOptimization {
    /// Detect induction variables in loops
    /// Returns list of (base_value, step_amount) tuples
    pub fn detect(&self, ir: &IRFunction) -> Vec<InductionVariable> {
        let mut result = Vec::new();

        // Find loop headers (blocks with backedges pointing to them)
        let loop_headers = self.find_loop_headers(ir);

        for header_id in loop_headers {
            // Analyze phi nodes in loop header for induction patterns
            if let Some(header_block) = ir.blocks.iter().find(|b| b.id == header_id) {
                for inst in &header_block.instructions {
                    if let IRInst::Phi { incoming, .. } = inst {
                        // Phi nodes are candidates for induction variables
                        if let Some((base, step)) = self.analyze_phi_for_induction(ir, header_id, incoming) {
                            result.push(InductionVariable { base, step });
                        }
                    }
                }
            }
        }

        result
    }

    /// Find loop headers by identifying blocks with backedges
    /// A backedge is when a successor of a block can reach back to that block
    fn find_loop_headers(&self, ir: &IRFunction) -> Vec<BlockId> {
        let mut headers = HashSet::new();

        for block in &ir.blocks {
            // Check each successor - if it's a predecessor of current block, we have a backedge
            for &succ_id in &block.successors {
                if let Some(succ_block) = ir.blocks.iter().find(|b| b.id == succ_id) {
                    // If successor has current block as predecessor, it's a backedge
                    if succ_block.predecessors.contains(&block.id) {
                        // The successor is a loop header
                        headers.insert(succ_id);
                    }
                }
            }
        }

        headers.into_iter().collect()
    }

    /// Analyze a phi node to detect if it's an induction variable
    /// Induction variables typically have pattern: phi(init, updated)
    /// where updated = phi_var OP constant
    fn analyze_phi_for_induction(
        &self,
        ir: &IRFunction,
        _header_id: BlockId,
        incoming: &[(BlockId, IRValue)],
    ) -> Option<(IRValue, i64)> {
        // Need exactly 2 incoming edges: one from loop entry, one from loop body (backedge)
        if incoming.len() != 2 {
            return None;
        }

        // Find the backedge and entry edge
        let (entry_val, backedge_val) = {
            let (pred0_id, val0) = &incoming[0];
            let (pred1_id, val1) = &incoming[1];

            // Check which predecessor is the backedge (goes through the loop)
            // Heuristic: backedge source usually has more predecessors (loop body)
            let pred0_block = ir.blocks.iter().find(|b| b.id == *pred0_id)?;
            let pred1_block = ir.blocks.iter().find(|b| b.id == *pred1_id)?;

            if pred0_block.predecessors.len() > pred1_block.predecessors.len() {
                (val1.clone(), val0.clone())
            } else {
                (val0.clone(), val1.clone())
            }
        };

        // Basic induction variable: init is constant, updated is register with same base
        // For now, conservatively detect: constant entry + register backedge
        match (&entry_val, &backedge_val) {
            (IRValue::Constant(_), IRValue::Register(_, _)) => {
                // Likely pattern: i = 0; ... i = i + 1
                // Assume +1 step for common case (can be refined)
                Some((entry_val, 1))
            }
            _ => None,
        }
    }

    /// Apply induction variable optimizations
    /// Currently: identify induction variables and mark strength reduction targets
    pub fn apply_optimizations(&self, _ir: &mut IRFunction) -> bool {
        // Actual optimization would be done in StrengthReduction pass
        // This pass just detects and classifies
        false
    }
}

impl Pass for InductionVariableOptimization {
    fn name(&self) -> &'static str {
        "InductionVariableOptimization"
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        self.apply_optimizations(ir)
    }
}
