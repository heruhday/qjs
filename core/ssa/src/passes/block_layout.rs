use std::collections::{HashMap, HashSet};

use cfg::BlockId;

use crate::ir::{IRFunction, IRTerminator};
use crate::passes::Pass;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BlockLayoutOptimization;

impl BlockLayoutOptimization {
    /// Reorder blocks for better instruction cache locality
    /// Strategy: place fallthrough targets immediately after branches when possible
    pub fn reorder_blocks(&self, ir: &mut IRFunction) -> bool {
        if ir.blocks.is_empty() {
            return false;
        }

        let mut new_order = Vec::new();
        let mut visited = HashSet::new();
        
        // Start from entry block and build a layout that tries to keep hot paths together
        self.layout_blocks(ir, ir.entry, &mut new_order, &mut visited);
        
        // Add any remaining blocks (unreachable code)
        for block in &ir.blocks {
            if !visited.contains(&block.id) {
                new_order.push(block.id);
            }
        }
        
        // Check if layout changed
        let old_ids: Vec<_> = ir.blocks.iter().map(|b| b.id).collect();
        if old_ids == new_order {
            return false;
        }
        
        // Reorder blocks according to new layout
        let mut block_map: HashMap<BlockId, _> = ir.blocks
            .iter()
            .map(|b| (b.id, b.clone()))
            .collect();
        
        ir.blocks.clear();
        for id in new_order {
            if let Some(block) = block_map.remove(&id) {
                ir.blocks.push(block);
            }
        }
        
        true
    }
    
    /// Layout blocks using a DFS-like traversal that prioritizes fallthrough paths
    fn layout_blocks(
        &self,
        ir: &IRFunction,
        block_id: BlockId,
        layout: &mut Vec<BlockId>,
        visited: &mut HashSet<BlockId>,
    ) {
        if visited.contains(&block_id) {
            return;
        }
        
        visited.insert(block_id);
        layout.push(block_id);
        
        // Find the block to get its terminator
        if let Some(block) = ir.blocks.iter().find(|b| b.id == block_id) {
            match &block.terminator {
                IRTerminator::Jump { target } => {
                    // For unconditional jumps, immediately place the target
                    self.layout_blocks(ir, *target, layout, visited);
                }
                IRTerminator::Branch {
                    target,
                    fallthrough,
                    ..
                } => {
                    // For branches, prioritize fallthrough path (usually the more common case)
                    // Place fallthrough target immediately after
                    self.layout_blocks(ir, *fallthrough, layout, visited);
                    // Then place branch target
                    self.layout_blocks(ir, *target, layout, visited);
                }
                IRTerminator::Try {
                    handler,
                    fallthrough,
                } => {
                    // Fallthrough is the common case
                    self.layout_blocks(ir, *fallthrough, layout, visited);
                    self.layout_blocks(ir, *handler, layout, visited);
                }
                IRTerminator::ConditionalReturn { fallthrough, .. } => {
                    // Fallthrough continues normal execution
                    self.layout_blocks(ir, *fallthrough, layout, visited);
                }
                IRTerminator::Switch {
                    default_target,
                    cases,
                    ..
                } => {
                    // Place default target first (most common)
                    self.layout_blocks(ir, *default_target, layout, visited);
                    // Then other cases
                    for (_, target) in cases {
                        self.layout_blocks(ir, *target, layout, visited);
                    }
                }
                _ => {} // Return, Throw, etc. have no successors
            }
        }
    }
}

impl Pass for BlockLayoutOptimization {
    fn name(&self) -> &'static str {
        "BlockLayoutOptimization"
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        self.reorder_blocks(ir)
    }

    fn is_structural(&self) -> bool {
        true
    }
}
