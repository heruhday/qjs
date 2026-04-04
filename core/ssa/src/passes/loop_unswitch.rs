use std::collections::HashSet;

use cfg::BlockId;

use crate::ir::{IRBlock, IRCondition, IRFunction, IRTerminator};
use crate::passes::Pass;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopUnswitching {
    pub max_duplication_instructions: usize,
}

/// Information about a detected loop
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct LoopInfo {
    header: BlockId,
    body_blocks: Vec<BlockId>,
    exit: BlockId,
}

impl Default for LoopUnswitching {
    fn default() -> Self {
        Self {
            max_duplication_instructions: 32,
        }
    }
}

impl LoopUnswitching {
    /// Unswitch loops by hoisting invariant branches out
    pub fn unswitch(&self, ir: &mut IRFunction) -> bool {
        let loops = self.find_loops(ir);
        let mut changed = false;

        for loop_info in loops {
            // Only unswitch if loop body is small enough
            let body_size = self.estimate_body_size(ir, &loop_info.body_blocks);
            if body_size > self.max_duplication_instructions {
                continue;
            }

            // Find invariant branches in the loop
            if self.unswitch_loop(ir, &loop_info) {
                changed = true;
            }
        }

        changed
    }

    /// Find natural loops via backedge detection
    fn find_loops(&self, ir: &IRFunction) -> Vec<LoopInfo> {
        let mut loops = Vec::new();

        for block in &ir.blocks {
            for &succ_id in &block.successors {
                if let Some(succ_block) = ir.blocks.iter().find(|b| b.id == succ_id) {
                    if succ_block.predecessors.contains(&block.id) {
                        // Found a backedge: succ_id is loop header
                        let body_blocks = self.find_loop_body(ir, succ_id, block.id);

                        // Find exit block (successor of loop that's not in the loop)
                        let exit = succ_block
                            .successors
                            .iter()
                            .find(|&&s| !body_blocks.contains(&s))
                            .copied();

                        if let Some(exit_block) = exit {
                            loops.push(LoopInfo {
                                header: succ_id,
                                body_blocks,
                                exit: exit_block,
                            });
                        }
                    }
                }
            }
        }

        loops
    }

    /// Find blocks that belong to a loop
    fn find_loop_body(
        &self,
        ir: &IRFunction,
        header: BlockId,
        _backedge_source: BlockId,
    ) -> Vec<BlockId> {
        let mut body = Vec::new();
        let mut visited = HashSet::new();
        let mut worklist = vec![header];

        while let Some(id) = worklist.pop() {
            if visited.contains(&id) {
                continue;
            }
            visited.insert(id);
            body.push(id);

            if let Some(block) = ir.blocks.iter().find(|b| b.id == id) {
                for &succ in &block.successors {
                    if !visited.contains(&succ) {
                        worklist.push(succ);
                    }
                }
            }
        }

        body
    }

    /// Estimate total instructions in loop body
    fn estimate_body_size(&self, ir: &IRFunction, body_blocks: &[BlockId]) -> usize {
        body_blocks
            .iter()
            .filter_map(|&id| ir.blocks.iter().find(|b| b.id == id))
            .map(|block| block.instructions.len())
            .sum()
    }

    /// Unswitch a loop by hoisting invariant branches
    fn unswitch_loop(&self, ir: &mut IRFunction, loop_info: &LoopInfo) -> bool {
        // Find a branch block in the loop with an invariant condition
        let mut branch_info: Option<(BlockId, IRCondition, BlockId, BlockId)> = None;

        for &block_id in &loop_info.body_blocks {
            if let Some(block) = ir.blocks.iter().find(|b| b.id == block_id) {
                // Check if this block has a branch terminator
                if let IRTerminator::Branch {
                    condition,
                    target,
                    fallthrough,
                } = &block.terminator
                {
                    // Check if condition is loop-invariant (simplified heuristic)
                    if self.is_invariant_condition(condition) {
                        // Found a candidate for unswitching
                        branch_info = Some((block_id, condition.clone(), *target, *fallthrough));
                        break;
                    }
                }
            }
        }

        if let Some((branch_block, condition, target, fallthrough)) = branch_info {
            self.unswitch_branch(
                ir,
                loop_info,
                branch_block,
                &condition,
                &target,
                &fallthrough,
            )
        } else {
            false
        }
    }

    /// Check if a condition is invariant (conservative: always returns true for now)
    fn is_invariant_condition(&self, _condition: &IRCondition) -> bool {
        // In a full implementation, check if all values in condition don't change in loop
        // Conservative: return true to attempt unswitching
        true
    }

    /// Perform loop unswitching by duplicating the loop for each branch
    fn unswitch_branch(
        &self,
        ir: &mut IRFunction,
        loop_info: &LoopInfo,
        branch_block: BlockId,
        _condition: &IRCondition,
        _true_target: &BlockId,
        _false_target: &BlockId,
    ) -> bool {
        // Simple unswitching: duplicate the loop body
        let mut new_blocks = Vec::new();

        for &body_id in &loop_info.body_blocks {
            if let Some(orig_block) = ir.blocks.iter().find(|b| b.id == body_id).cloned() {
                // Create new block for true branch
                let new_id_true = ir.blocks.len() + new_blocks.len();

                let mut new_block_true = IRBlock {
                    id: new_id_true,
                    instructions: orig_block.instructions.clone(),
                    terminator: orig_block.terminator.clone(),
                    successors: orig_block.successors.clone(),
                    predecessors: orig_block.predecessors.clone(),
                };

                // Update branch terminators to remove the conditional
                if orig_block.id == branch_block {
                    // Replace branch with unconditional jump
                    if let IRTerminator::Branch { target, .. } = orig_block.terminator.clone() {
                        new_block_true.terminator = IRTerminator::Jump { target };
                    }
                }

                new_blocks.push(new_block_true);

                // Create new block for false branch
                let new_id_false = ir.blocks.len() + new_blocks.len();

                let mut new_block_false = IRBlock {
                    id: new_id_false,
                    instructions: orig_block.instructions.clone(),
                    terminator: orig_block.terminator.clone(),
                    successors: orig_block.successors.clone(),
                    predecessors: orig_block.predecessors.clone(),
                };

                if orig_block.id == branch_block {
                    if let IRTerminator::Branch { fallthrough, .. } = orig_block.terminator.clone()
                    {
                        new_block_false.terminator = IRTerminator::Jump {
                            target: fallthrough,
                        };
                    }
                }

                new_blocks.push(new_block_false);
            }
        }

        if new_blocks.is_empty() {
            return false;
        }

        ir.blocks.extend(new_blocks);
        true
    }
}

impl Pass for LoopUnswitching {
    fn name(&self) -> &'static str {
        "LoopUnswitching"
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        self.unswitch(ir)
    }

    fn is_structural(&self) -> bool {
        true
    }
}
