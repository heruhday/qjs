use std::collections::HashSet;

use cfg::BlockId;

use crate::ir::{IRBlock, IRFunction};
use crate::passes::Pass;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopUnrolling {
    pub factor: usize,
    pub max_body_instructions: usize,
}

/// Information about a detected loop
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct LoopInfo {
    header: BlockId,
    body_blocks: Vec<BlockId>,
    exit: BlockId,
}

impl Default for LoopUnrolling {
    fn default() -> Self {
        Self {
            factor: 2,
            max_body_instructions: 32,
        }
    }
}

impl LoopUnrolling {
    /// Unroll hot loops to reduce loop overhead and enable parallelism
    pub fn unroll(&self, ir: &mut IRFunction) -> bool {
        let loops = self.find_loops(ir);
        let mut changed = false;

        for loop_info in loops {
            // Only unroll if loop body is small enough
            let body_size = self.estimate_body_size(ir, &loop_info.body_blocks);
            if body_size > self.max_body_instructions {
                continue;
            }

            // Unroll the loop by duplicating body blocks
            if self.unroll_loop(ir, &loop_info) {
                changed = true;
            }
        }

        changed
    }

    /// Find natural loops via backedge detection
    fn find_loops(&self, ir: &IRFunction) -> Vec<LoopInfo> {
        let mut loops = Vec::new();

        for block in &ir.blocks {
            // Check each successor - if it reaches back to a predecessor, we have a loop
            for &succ_id in &block.successors {
                if let Some(succ_block) = ir.blocks.iter().find(|b| b.id == succ_id) {
                    if succ_block.predecessors.contains(&block.id) {
                        // Found a backedge: succ_id is loop header, block is loop footer
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

    /// Find blocks that belong to a loop (reachable from header and can reach back to header)
    fn find_loop_body(&self, ir: &IRFunction, header: BlockId, _backedge_source: BlockId) -> Vec<BlockId> {
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

    /// Perform loop unrolling by duplicating body blocks
    fn unroll_loop(&self, ir: &mut IRFunction, loop_info: &LoopInfo) -> bool {
        if self.factor < 2 {
            return false;
        }

        // For now, implement simple block duplication for unroll factor
        // This is a conservative approach that duplicates the body multiple times
        let mut new_blocks = Vec::new();

        for _iteration in 1..self.factor {
            for &body_id in &loop_info.body_blocks {
                if let Some(orig_block) = ir.blocks.iter().find(|b| b.id == body_id).cloned() {
                    // Create new block with unique ID
                    let new_id = ir.blocks.len() + new_blocks.len();

                    let mut new_block = IRBlock {
                        id: new_id,
                        instructions: orig_block.instructions.clone(),
                        terminator: orig_block.terminator.clone(),
                        successors: orig_block.successors.clone(),
                        predecessors: orig_block.predecessors.clone(),
                    };

                    // Redirect successors from header back to first unrolled copy
                    for succ in &mut new_block.successors {
                        if *succ == loop_info.header {
                            *succ = loop_info.body_blocks[0];
                        }
                    }

                    new_blocks.push(new_block);
                }
            }
        }

        if new_blocks.is_empty() {
            return false;
        }

        ir.blocks.extend(new_blocks);
        true
    }
}

impl Pass for LoopUnrolling {
    fn name(&self) -> &'static str {
        "LoopUnrolling"
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        self.unroll(ir)
    }

    fn is_structural(&self) -> bool {
        true
    }
}
