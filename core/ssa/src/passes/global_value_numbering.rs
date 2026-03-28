use std::collections::HashMap;

use cfg::BlockId;

use crate::ir::{IRBinaryOp, IRFunction, IRInst, IRUnaryOp, IRValue};
use crate::passes::Pass;

pub struct GlobalValueNumbering;

impl Pass for GlobalValueNumbering {
    fn name(&self) -> &'static str {
        "GlobalValueNumbering"
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        if ir.blocks.is_empty() || ir.entry >= ir.blocks.len() {
            return false;
        }

        let dominance = analyze_dominance(ir);
        let mut changed = false;
        let available = HashMap::new();
        rewrite_block(
            ir.entry,
            ir,
            &dominance.tree_children,
            &available,
            &mut changed,
        );
        changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ExpressionKey {
    LoadConst {
        value: value::JSValue,
    },
    Unary {
        op: IRUnaryOp,
        operand: IRValue,
    },
    Binary {
        op: IRBinaryOp,
        lhs: IRValue,
        rhs: IRValue,
    },
}

#[derive(Debug, Clone)]
struct DominanceInfo {
    tree_children: Vec<Vec<BlockId>>,
}

fn rewrite_block(
    block_id: BlockId,
    ir: &mut IRFunction,
    tree_children: &[Vec<BlockId>],
    incoming: &HashMap<ExpressionKey, IRValue>,
    changed: &mut bool,
) {
    let mut available = incoming.clone();
    let Some(block) = ir.blocks.get_mut(block_id) else {
        return;
    };

    for inst in &mut block.instructions {
        if let Some((dst, expr)) = expression_key(inst) {
            if let Some(existing) = available.get(&expr).cloned() {
                *inst = IRInst::Mov { dst, src: existing };
                *changed = true;
            } else {
                available.insert(expr, dst);
            }
            continue;
        }

        if matches!(inst, IRInst::Bytecode { .. }) {
            // Unknown bytecode can hide effects the current IR does not model yet,
            // so keep GVN within regions of structured instructions.
            available.clear();
        }
    }

    let children = tree_children.get(block_id).cloned().unwrap_or_default();
    for child in children {
        rewrite_block(child, ir, tree_children, &available, changed);
    }
}

fn expression_key(inst: &IRInst) -> Option<(IRValue, ExpressionKey)> {
    match inst {
        IRInst::LoadConst { dst, value } => {
            Some((dst.clone(), ExpressionKey::LoadConst { value: *value }))
        }
        IRInst::Unary { dst, op, operand } => Some((
            dst.clone(),
            ExpressionKey::Unary {
                op: *op,
                operand: operand.clone(),
            },
        )),
        IRInst::Binary { dst, op, lhs, rhs } => Some((
            dst.clone(),
            ExpressionKey::Binary {
                op: *op,
                lhs: lhs.clone(),
                rhs: rhs.clone(),
            },
        )),
        IRInst::Phi { .. } | IRInst::Mov { .. } | IRInst::Bytecode { .. } | IRInst::Nop => None,
    }
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

    let mut tree_children = vec![Vec::new(); block_count];
    for &block in reverse_post_order.iter().skip(1) {
        if let Some(parent) = idom[block] {
            tree_children[parent].push(block);
        }
    }

    DominanceInfo { tree_children }
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
