use std::collections::VecDeque;

use cfg::BlockId;

use crate::ir::{IRFunction, IRInst, IRTerminator, IRValue};
use crate::passes::Pass;

pub struct CfgSimplification;

impl Pass for CfgSimplification {
    fn name(&self) -> &'static str {
        "CfgSimplification"
    }

    fn is_structural(&self) -> bool {
        true
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        if ir.blocks.is_empty() || ir.entry >= ir.blocks.len() {
            return false;
        }

        let mut changed = false;

        loop {
            recompute_cfg_links(ir);

            let mut local = false;
            local |= simplify_trivial_terminators(ir);
            recompute_cfg_links(ir);

            local |= simplify_phi_nodes(ir);
            recompute_cfg_links(ir);

            local |= merge_linear_blocks(ir);
            recompute_cfg_links(ir);

            local |= prune_unreachable_blocks(ir);
            recompute_cfg_links(ir);

            local |= simplify_phi_nodes(ir);
            recompute_cfg_links(ir);

            if !local {
                break;
            }
            changed = true;
        }

        changed
    }
}

fn simplify_trivial_terminators(ir: &mut IRFunction) -> bool {
    let mut changed = false;

    for block in &mut ir.blocks {
        match &block.terminator {
            IRTerminator::Branch {
                target,
                fallthrough,
                ..
            } if target == fallthrough => {
                block.terminator = IRTerminator::Jump { target: *target };
                changed = true;
            }
            IRTerminator::Switch {
                cases,
                default_target,
                ..
            } => {
                let only_target = cases
                    .iter()
                    .map(|(_, target)| *target)
                    .try_fold(*default_target, |current, target| {
                        (target == current).then_some(current)
                    });
                if let Some(target) = only_target {
                    block.terminator = IRTerminator::Jump { target };
                    changed = true;
                }
            }
            _ => {}
        }
    }

    changed
}

fn simplify_phi_nodes(ir: &mut IRFunction) -> bool {
    let mut changed = false;

    for block in &mut ir.blocks {
        let predecessors = block.predecessors.clone();
        for inst in &mut block.instructions {
            let IRInst::Phi { dst, incoming } = inst else {
                continue;
            };

            let original = incoming.clone();
            let mut filtered = Vec::with_capacity(incoming.len());

            for (pred, value) in original {
                if !predecessors.contains(&pred) {
                    changed = true;
                    continue;
                }

                if let Some((_, existing)) = filtered
                    .iter_mut()
                    .find(|(seen_pred, _)| *seen_pred == pred)
                {
                    changed = true;
                    *existing = value;
                    continue;
                }

                filtered.push((pred, value));
            }

            if *incoming != filtered {
                *incoming = filtered;
            }

            if incoming.len() == 1 {
                let replacement = replacement_for_value(dst.clone(), incoming[0].1.clone());
                *inst = replacement;
                changed = true;
                continue;
            }

            if let Some((_, first)) = incoming.first()
                && incoming.iter().all(|(_, value)| value == first)
            {
                let replacement = replacement_for_value(dst.clone(), first.clone());
                *inst = replacement;
                changed = true;
            }
        }
    }

    changed
}

fn merge_linear_blocks(ir: &mut IRFunction) -> bool {
    let mut removed = vec![false; ir.blocks.len()];
    let mut changed = false;

    for block_id in 0..ir.blocks.len() {
        if removed[block_id] {
            continue;
        }

        let IRTerminator::Jump { target } = ir.blocks[block_id].terminator else {
            continue;
        };

        if target >= ir.blocks.len()
            || target == ir.entry
            || target == block_id
            || removed[target]
            || ir.blocks[target].predecessors.as_slice() != [block_id]
            || ir.blocks[target]
                .instructions
                .iter()
                .any(|inst| matches!(inst, IRInst::Phi { .. }))
        {
            continue;
        }

        let target_block = ir.blocks[target].clone();
        ir.blocks[block_id]
            .instructions
            .extend(target_block.instructions);
        ir.blocks[block_id].terminator = target_block.terminator;
        removed[target] = true;
        changed = true;
    }

    if changed {
        let keep = removed
            .into_iter()
            .map(|removed| !removed)
            .collect::<Vec<_>>();
        retain_blocks(ir, &keep);
    }

    changed
}

fn prune_unreachable_blocks(ir: &mut IRFunction) -> bool {
    let reachable = reachable_blocks(ir);
    if reachable.iter().all(|reachable| *reachable) {
        return false;
    }

    retain_blocks(ir, &reachable);
    true
}

fn reachable_blocks(ir: &IRFunction) -> Vec<bool> {
    let mut reachable = vec![false; ir.blocks.len()];
    let mut worklist = VecDeque::new();

    if ir.entry < ir.blocks.len() {
        reachable[ir.entry] = true;
        worklist.push_back(ir.entry);
    }

    while let Some(block_id) = worklist.pop_front() {
        for &successor in &ir.blocks[block_id].successors {
            if successor < reachable.len() && !reachable[successor] {
                reachable[successor] = true;
                worklist.push_back(successor);
            }
        }
    }

    reachable
}

fn retain_blocks(ir: &mut IRFunction, keep: &[bool]) {
    let mut mapping = vec![None; ir.blocks.len()];
    let mut blocks = Vec::new();

    for (old_id, block) in ir.blocks.iter().cloned().enumerate() {
        if keep.get(old_id).copied().unwrap_or(false) {
            let new_id = blocks.len();
            mapping[old_id] = Some(new_id);
            blocks.push(block);
        }
    }

    for block in &mut blocks {
        block.id = mapping[block.id].expect("kept blocks must be remapped");
        remap_terminator(&mut block.terminator, &mapping);
        block.successors = remap_block_list(&block.successors, &mapping);
        block.predecessors = remap_block_list(&block.predecessors, &mapping);

        for inst in &mut block.instructions {
            let IRInst::Phi { incoming, .. } = inst else {
                continue;
            };

            incoming.retain(|(pred, _)| mapping.get(*pred).copied().flatten().is_some());
            for (pred, _) in incoming.iter_mut() {
                *pred = mapping[*pred].expect("phi predecessor must be remapped");
            }
        }
    }

    ir.entry = mapping[ir.entry].unwrap_or(0);
    ir.exit_blocks = remap_block_list(&ir.exit_blocks, &mapping);
    ir.blocks = blocks;
}

fn remap_terminator(terminator: &mut IRTerminator, mapping: &[Option<BlockId>]) {
    match terminator {
        IRTerminator::Jump { target } => {
            *target = mapping[*target].expect("jump target must be retained");
        }
        IRTerminator::Branch {
            target,
            fallthrough,
            ..
        } => {
            *target = mapping[*target].expect("branch target must be retained");
            *fallthrough = mapping[*fallthrough].expect("branch fallthrough must be retained");
        }
        IRTerminator::Switch {
            cases,
            default_target,
            ..
        } => {
            *default_target =
                mapping[*default_target].expect("switch default target must be retained");
            for (_, target) in cases {
                *target = mapping[*target].expect("switch case target must be retained");
            }
        }
        IRTerminator::Try {
            handler,
            fallthrough,
        } => {
            *handler = mapping[*handler].expect("try handler must be retained");
            *fallthrough = mapping[*fallthrough].expect("try fallthrough must be retained");
        }
        IRTerminator::ConditionalReturn { fallthrough, .. } => {
            *fallthrough = mapping[*fallthrough].expect("fallthrough must be retained");
        }
        IRTerminator::None
        | IRTerminator::Return { .. }
        | IRTerminator::Throw { .. }
        | IRTerminator::TailCall { .. }
        | IRTerminator::CallReturn { .. } => {}
    }
}

fn remap_block_list(blocks: &[BlockId], mapping: &[Option<BlockId>]) -> Vec<BlockId> {
    let mut remapped = Vec::new();

    for &block in blocks {
        let Some(block) = mapping.get(block).copied().flatten() else {
            continue;
        };
        if !remapped.contains(&block) {
            remapped.push(block);
        }
    }

    remapped
}

fn replacement_for_value(dst: IRValue, value: IRValue) -> IRInst {
    match value {
        IRValue::Constant(value) => IRInst::LoadConst { dst, value },
        value if value == dst => IRInst::Nop,
        src => IRInst::Mov { dst, src },
    }
}

fn recompute_cfg_links(ir: &mut IRFunction) {
    let mut predecessors = vec![Vec::new(); ir.blocks.len()];

    for block in &mut ir.blocks {
        block.successors = terminator_successors(&block.terminator);
    }

    for block_id in 0..ir.blocks.len() {
        for &successor in &ir.blocks[block_id].successors {
            if successor < predecessors.len() && !predecessors[successor].contains(&block_id) {
                predecessors[successor].push(block_id);
            }
        }
    }

    let mut exit_blocks = Vec::new();
    for block_id in 0..ir.blocks.len() {
        ir.blocks[block_id].predecessors = predecessors[block_id].clone();
        if is_exit_terminator(&ir.blocks[block_id].terminator) {
            exit_blocks.push(block_id);
        }
    }
    ir.exit_blocks = exit_blocks;
}

fn terminator_successors(terminator: &IRTerminator) -> Vec<BlockId> {
    match terminator {
        IRTerminator::None
        | IRTerminator::Return { .. }
        | IRTerminator::Throw { .. }
        | IRTerminator::TailCall { .. }
        | IRTerminator::CallReturn { .. } => Vec::new(),
        IRTerminator::Jump { target } => vec![*target],
        IRTerminator::Branch {
            target,
            fallthrough,
            ..
        } => {
            let mut successors = vec![*target];
            if *fallthrough != *target {
                successors.push(*fallthrough);
            }
            successors
        }
        IRTerminator::Switch {
            cases,
            default_target,
            ..
        } => {
            let mut successors = vec![*default_target];
            for (_, target) in cases {
                if !successors.contains(target) {
                    successors.push(*target);
                }
            }
            successors
        }
        IRTerminator::Try {
            handler,
            fallthrough,
        } => {
            let mut successors = vec![*handler];
            if *fallthrough != *handler {
                successors.push(*fallthrough);
            }
            successors
        }
        IRTerminator::ConditionalReturn { fallthrough, .. } => vec![*fallthrough],
    }
}

fn is_exit_terminator(terminator: &IRTerminator) -> bool {
    matches!(
        terminator,
        IRTerminator::Return { .. }
            | IRTerminator::Throw { .. }
            | IRTerminator::TailCall { .. }
            | IRTerminator::CallReturn { .. }
    )
}
