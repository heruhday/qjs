use cfg::{BlockId, CFG};

#[derive(Debug, Clone)]
pub struct DominanceInfo {
    pub reverse_post_order: Vec<BlockId>,
    pub idom: Vec<Option<BlockId>>,
    pub tree_children: Vec<Vec<BlockId>>,
    pub frontiers: Vec<Vec<BlockId>>,
}

impl DominanceInfo {
    pub fn immediate_dominator(&self, block: BlockId) -> Option<BlockId> {
        self.idom.get(block).copied().flatten()
    }

    pub fn dominates(&self, dominator: BlockId, mut block: BlockId) -> bool {
        if dominator == block {
            return true;
        }

        while let Some(parent) = self.idom.get(block).copied().flatten() {
            if parent == dominator {
                return true;
            }
            if parent == block {
                break;
            }
            block = parent;
        }

        false
    }
}

pub fn analyze_dominance(cfg: &CFG) -> DominanceInfo {
    let block_count = cfg.blocks.len();
    let reverse_post_order = reverse_post_order(cfg);
    let mut order_index = vec![usize::MAX; block_count];
    for (index, block) in reverse_post_order.iter().copied().enumerate() {
        order_index[block] = index;
    }

    let mut idom = vec![None; block_count];
    idom[cfg.entry] = Some(cfg.entry);

    let mut changed = true;
    while changed {
        changed = false;
        for &block in reverse_post_order.iter().skip(1) {
            let mut preds = cfg.blocks[block]
                .predecessors
                .iter()
                .copied()
                .filter(|pred| idom[*pred].is_some());

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
    for block in 0..block_count {
        if block == cfg.entry {
            continue;
        }
        if let Some(parent) = idom[block] {
            tree_children[parent].push(block);
        }
    }

    let mut frontiers = vec![Vec::new(); block_count];
    for block in 0..block_count {
        if cfg.blocks[block].predecessors.len() < 2 {
            continue;
        }
        let Some(idom_block) = idom[block] else {
            continue;
        };

        for &pred in &cfg.blocks[block].predecessors {
            let mut runner = pred;
            while runner != idom_block {
                push_unique(&mut frontiers[runner], block);
                let Some(parent) = idom[runner] else {
                    break;
                };
                if parent == runner {
                    break;
                }
                runner = parent;
            }
        }
    }

    DominanceInfo {
        reverse_post_order,
        idom,
        tree_children,
        frontiers,
    }
}

fn reverse_post_order(cfg: &CFG) -> Vec<BlockId> {
    fn dfs(cfg: &CFG, block: BlockId, visited: &mut [bool], postorder: &mut Vec<BlockId>) {
        if visited[block] {
            return;
        }
        visited[block] = true;
        for &successor in &cfg.blocks[block].successors {
            dfs(cfg, successor, visited, postorder);
        }
        postorder.push(block);
    }

    let mut visited = vec![false; cfg.blocks.len()];
    let mut postorder = Vec::with_capacity(cfg.blocks.len());
    dfs(cfg, cfg.entry, &mut visited, &mut postorder);
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

fn push_unique(blocks: &mut Vec<BlockId>, block: BlockId) {
    if !blocks.contains(&block) {
        blocks.push(block);
    }
}
