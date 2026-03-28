use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use cfg::BlockId;

use crate::ir::{IRBlock, IRFunction, IRInst, IRTerminator, IRValue};
use crate::passes::Pass;

pub struct LoopInvariantCodeMotion;

impl Pass for LoopInvariantCodeMotion {
    fn name(&self) -> &'static str {
        "LoopInvariantCodeMotion"
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
            let dominance = analyze_dominance(ir);
            let loops = natural_loops(ir, &dominance);

            let mut local_changed = false;
            for natural_loop in loops {
                if hoist_loop_invariants(ir, &dominance, &natural_loop) {
                    local_changed = true;
                    changed = true;
                    break;
                }
            }

            if !local_changed {
                break;
            }
        }

        if changed {
            recompute_cfg_links(ir);
        }

        changed
    }
}

#[derive(Debug, Clone)]
struct DominanceInfo {
    idom: Vec<Option<BlockId>>,
}

impl DominanceInfo {
    fn dominates(&self, dominator: BlockId, mut block: BlockId) -> bool {
        if dominator == block {
            return true;
        }

        while block < self.idom.len() {
            let Some(idom) = self.idom[block] else {
                break;
            };
            if idom == dominator {
                return true;
            }
            if idom == block {
                break;
            }
            block = idom;
        }

        false
    }
}

#[derive(Debug, Clone)]
struct NaturalLoop {
    header: BlockId,
    blocks: BTreeSet<BlockId>,
}

#[derive(Debug, Clone)]
struct HoistCandidate {
    block_id: BlockId,
    inst_index: usize,
    dst: IRValue,
    inst: IRInst,
}

fn hoist_loop_invariants(
    ir: &mut IRFunction,
    dominance: &DominanceInfo,
    natural_loop: &NaturalLoop,
) -> bool {
    let Some(outside_pred) = unique_outside_predecessor(ir, natural_loop) else {
        return false;
    };

    let definitions = definition_blocks(ir);
    let hoists = collect_hoistable_instructions(ir, dominance, natural_loop, &definitions);
    if hoists.is_empty() {
        return false;
    }

    let preheader = ensure_preheader(ir, natural_loop, outside_pred);
    apply_hoists(ir, preheader, hoists);
    true
}

fn collect_hoistable_instructions(
    ir: &IRFunction,
    dominance: &DominanceInfo,
    natural_loop: &NaturalLoop,
    definitions: &HashMap<IRValue, BlockId>,
) -> Vec<HoistCandidate> {
    let mut candidates = Vec::<HoistCandidate>::new();

    for block_id in natural_loop.blocks.iter().copied() {
        let Some(block) = ir.blocks.get(block_id) else {
            continue;
        };

        for (inst_index, inst) in block.instructions.iter().cloned().enumerate() {
            let Some(dst) = defined_value(&inst) else {
                continue;
            };
            if !is_hoistable_instruction(&inst) {
                continue;
            }

            candidates.push(HoistCandidate {
                block_id,
                inst_index,
                dst,
                inst,
            });
        }
    }

    let mut invariant_values = HashSet::<IRValue>::new();
    let mut selected = vec![false; candidates.len()];
    let mut hoists = Vec::<HoistCandidate>::new();

    loop {
        let mut local_changed = false;

        for (index, candidate) in candidates.iter().enumerate() {
            if selected[index] {
                continue;
            }

            if instruction_operands_are_invariant(
                &candidate.inst,
                natural_loop,
                dominance,
                definitions,
                &invariant_values,
            ) {
                invariant_values.insert(candidate.dst.clone());
                hoists.push(candidate.clone());
                selected[index] = true;
                local_changed = true;
            }
        }

        if !local_changed {
            break;
        }
    }

    hoists
}

fn instruction_operands_are_invariant(
    inst: &IRInst,
    natural_loop: &NaturalLoop,
    dominance: &DominanceInfo,
    definitions: &HashMap<IRValue, BlockId>,
    invariants: &HashSet<IRValue>,
) -> bool {
    instruction_uses(inst).into_iter().all(|value| {
        value_is_loop_invariant(value, natural_loop, dominance, definitions, invariants)
    })
}

fn value_is_loop_invariant(
    value: &IRValue,
    natural_loop: &NaturalLoop,
    dominance: &DominanceInfo,
    definitions: &HashMap<IRValue, BlockId>,
    invariants: &HashSet<IRValue>,
) -> bool {
    match value {
        IRValue::Constant(_) => true,
        IRValue::Register(_, _) if invariants.contains(value) => true,
        IRValue::Register(_, _) => {
            let Some(def_block) = definitions.get(value).copied() else {
                return false;
            };
            !natural_loop.blocks.contains(&def_block)
                && dominance.dominates(def_block, natural_loop.header)
        }
    }
}

fn apply_hoists(ir: &mut IRFunction, preheader: BlockId, hoists: Vec<HoistCandidate>) {
    ir.blocks[preheader]
        .instructions
        .extend(hoists.iter().map(|candidate| candidate.inst.clone()));

    let mut removals = HashMap::<BlockId, HashSet<usize>>::new();
    for candidate in hoists {
        removals
            .entry(candidate.block_id)
            .or_default()
            .insert(candidate.inst_index);
    }

    for (block_id, indices) in removals {
        let Some(block) = ir.blocks.get_mut(block_id) else {
            continue;
        };

        let mut instructions =
            Vec::with_capacity(block.instructions.len().saturating_sub(indices.len()));
        for (index, inst) in block.instructions.drain(..).enumerate() {
            if !indices.contains(&index) {
                instructions.push(inst);
            }
        }
        block.instructions = instructions;
    }
}

fn unique_outside_predecessor(ir: &IRFunction, natural_loop: &NaturalLoop) -> Option<BlockId> {
    let header = ir.blocks.get(natural_loop.header)?;
    let outside = header
        .predecessors
        .iter()
        .copied()
        .filter(|pred| !natural_loop.blocks.contains(pred))
        .collect::<Vec<_>>();

    if outside.len() == 1 {
        Some(outside[0])
    } else {
        None
    }
}

fn ensure_preheader(
    ir: &mut IRFunction,
    natural_loop: &NaturalLoop,
    outside_pred: BlockId,
) -> BlockId {
    if is_dedicated_preheader(ir, outside_pred, natural_loop.header) {
        return outside_pred;
    }

    let preheader_id = ir.blocks.len();
    redirect_edge(
        &mut ir.blocks[outside_pred].terminator,
        natural_loop.header,
        preheader_id,
    );

    if let Some(header) = ir.blocks.get_mut(natural_loop.header) {
        for inst in &mut header.instructions {
            let IRInst::Phi { incoming, .. } = inst else {
                continue;
            };
            for (pred, _) in incoming.iter_mut() {
                if *pred == outside_pred {
                    *pred = preheader_id;
                }
            }
        }
    }

    ir.blocks.push(IRBlock {
        id: preheader_id,
        instructions: Vec::new(),
        terminator: IRTerminator::Jump {
            target: natural_loop.header,
        },
        successors: vec![natural_loop.header],
        predecessors: vec![outside_pred],
    });

    preheader_id
}

fn is_dedicated_preheader(ir: &IRFunction, block_id: BlockId, header: BlockId) -> bool {
    let Some(block) = ir.blocks.get(block_id) else {
        return false;
    };

    matches!(block.terminator, IRTerminator::Jump { target } if target == header)
        && block.successors.as_slice() == [header]
}

fn redirect_edge(terminator: &mut IRTerminator, old_target: BlockId, new_target: BlockId) -> bool {
    let mut changed = false;

    match terminator {
        IRTerminator::Jump { target } => {
            if *target == old_target {
                *target = new_target;
                changed = true;
            }
        }
        IRTerminator::Branch {
            target,
            fallthrough,
            ..
        } => {
            if *target == old_target {
                *target = new_target;
                changed = true;
            }
            if *fallthrough == old_target {
                *fallthrough = new_target;
                changed = true;
            }
        }
        IRTerminator::Switch {
            cases,
            default_target,
            ..
        } => {
            if *default_target == old_target {
                *default_target = new_target;
                changed = true;
            }
            for (_, target) in cases {
                if *target == old_target {
                    *target = new_target;
                    changed = true;
                }
            }
        }
        IRTerminator::Try {
            handler,
            fallthrough,
        } => {
            if *handler == old_target {
                *handler = new_target;
                changed = true;
            }
            if *fallthrough == old_target {
                *fallthrough = new_target;
                changed = true;
            }
        }
        IRTerminator::ConditionalReturn { fallthrough, .. } => {
            if *fallthrough == old_target {
                *fallthrough = new_target;
                changed = true;
            }
        }
        IRTerminator::None
        | IRTerminator::Return { .. }
        | IRTerminator::Throw { .. }
        | IRTerminator::TailCall { .. }
        | IRTerminator::CallReturn { .. } => {}
    }

    changed
}

fn natural_loops(ir: &IRFunction, dominance: &DominanceInfo) -> Vec<NaturalLoop> {
    let mut by_header = BTreeMap::<BlockId, BTreeSet<BlockId>>::new();

    for block in &ir.blocks {
        for &successor in &block.successors {
            if successor < ir.blocks.len() && dominance.dominates(successor, block.id) {
                by_header
                    .entry(successor)
                    .or_default()
                    .extend(collect_natural_loop(ir, block.id, successor));
            }
        }
    }

    let mut loops = by_header
        .into_iter()
        .map(|(header, blocks)| NaturalLoop { header, blocks })
        .collect::<Vec<_>>();
    loops.sort_by_key(|natural_loop| (natural_loop.blocks.len(), natural_loop.header));
    loops
}

fn collect_natural_loop(ir: &IRFunction, latch: BlockId, header: BlockId) -> BTreeSet<BlockId> {
    let mut blocks = BTreeSet::from([header, latch]);
    let mut worklist = VecDeque::from([latch]);

    while let Some(block_id) = worklist.pop_front() {
        let Some(block) = ir.blocks.get(block_id) else {
            continue;
        };

        for &pred in &block.predecessors {
            if blocks.insert(pred) && pred != header {
                worklist.push_back(pred);
            }
        }
    }

    blocks
}

fn analyze_dominance(ir: &IRFunction) -> DominanceInfo {
    let block_count = ir.blocks.len();
    let reverse_post_order = reverse_post_order(ir);
    let mut order_index = vec![usize::MAX; block_count];
    for (index, block) in reverse_post_order.iter().copied().enumerate() {
        if block < block_count {
            order_index[block] = index;
        }
    }

    let mut idom = vec![None; block_count];
    idom[ir.entry] = Some(ir.entry);

    let mut changed = true;
    while changed {
        changed = false;

        for &block in reverse_post_order.iter().skip(1) {
            let Some(ir_block) = ir.blocks.get(block) else {
                continue;
            };

            let mut preds = ir_block
                .predecessors
                .iter()
                .copied()
                .filter(|pred| *pred < block_count && idom[*pred].is_some());

            let Some(mut new_idom) = preds.next() else {
                continue;
            };

            for pred in preds {
                new_idom = intersect(&idom, &order_index, pred, new_idom);
            }

            if idom[block] != Some(new_idom) {
                idom[block] = Some(new_idom);
                changed = true;
            }
        }
    }

    DominanceInfo { idom }
}

fn reverse_post_order(ir: &IRFunction) -> Vec<BlockId> {
    fn dfs(ir: &IRFunction, block: BlockId, visited: &mut [bool], postorder: &mut Vec<BlockId>) {
        if block >= ir.blocks.len() || visited[block] {
            return;
        }

        visited[block] = true;
        for &successor in &ir.blocks[block].successors {
            dfs(ir, successor, visited, postorder);
        }
        postorder.push(block);
    }

    let mut visited = vec![false; ir.blocks.len()];
    let mut postorder = Vec::with_capacity(ir.blocks.len());
    dfs(ir, ir.entry, &mut visited, &mut postorder);
    postorder.reverse();
    postorder
}

fn intersect(
    idom: &[Option<BlockId>],
    order_index: &[usize],
    mut left: BlockId,
    mut right: BlockId,
) -> BlockId {
    while left != right {
        while order_index[left] > order_index[right] {
            left = idom[left].expect("reachable blocks must have an idom");
        }
        while order_index[right] > order_index[left] {
            right = idom[right].expect("reachable blocks must have an idom");
        }
    }
    left
}

fn definition_blocks(ir: &IRFunction) -> HashMap<IRValue, BlockId> {
    let mut definitions = HashMap::<IRValue, BlockId>::new();

    for block in &ir.blocks {
        for inst in &block.instructions {
            if let Some(dst) = defined_value(inst) {
                definitions.entry(dst).or_insert(block.id);
            }
        }
    }

    definitions
}

fn instruction_uses(inst: &IRInst) -> Vec<&IRValue> {
    match inst {
        IRInst::Phi { .. } | IRInst::LoadConst { .. } | IRInst::Bytecode { .. } | IRInst::Nop => {
            Vec::new()
        }
        IRInst::Mov { src, .. } => vec![src],
        IRInst::Unary { operand, .. } => vec![operand],
        IRInst::Binary { lhs, rhs, .. } => vec![lhs, rhs],
    }
}

fn is_hoistable_instruction(inst: &IRInst) -> bool {
    matches!(
        inst,
        IRInst::Mov { .. }
            | IRInst::LoadConst { .. }
            | IRInst::Unary { .. }
            | IRInst::Binary { .. }
    )
}

fn defined_value(inst: &IRInst) -> Option<IRValue> {
    match inst {
        IRInst::Phi { dst, .. }
        | IRInst::Mov { dst, .. }
        | IRInst::LoadConst { dst, .. }
        | IRInst::Unary { dst, .. }
        | IRInst::Binary { dst, .. } => Some(dst.clone()),
        IRInst::Bytecode { .. } | IRInst::Nop => None,
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
