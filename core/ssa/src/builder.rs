use std::collections::{BTreeMap, BTreeSet, VecDeque};

use cfg::{BlockId, CFG, DecodedInst};

use crate::analysis::{DominanceInfo, analyze_dominance};
use crate::semantics::instruction_operands;

pub type SSAValue = (u8, usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhiNode {
    pub target_reg: u8,
    pub target_version: usize,
    pub incoming: Vec<(BlockId, SSAValue)>,
}

#[derive(Debug, Clone)]
pub enum SSAInstr {
    Original {
        inst: DecodedInst,
        uses: Vec<SSAValue>,
        defined: Vec<SSAValue>,
    },
}

impl SSAInstr {
    pub fn inst(&self) -> &DecodedInst {
        match self {
            Self::Original { inst, .. } => inst,
        }
    }

    pub fn uses(&self) -> &[SSAValue] {
        match self {
            Self::Original { uses, .. } => uses,
        }
    }

    pub fn defined(&self) -> &[SSAValue] {
        match self {
            Self::Original { defined, .. } => defined,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SSABlock {
    pub id: BlockId,
    pub phi_nodes: Vec<PhiNode>,
    pub instructions: Vec<SSAInstr>,
    pub successors: Vec<BlockId>,
    pub predecessors: Vec<BlockId>,
}

#[derive(Debug, Clone)]
pub struct SSAForm {
    pub cfg: CFG,
    pub dominance: DominanceInfo,
    pub blocks: Vec<SSABlock>,
    pub entry: BlockId,
    pub exit_blocks: Vec<BlockId>,
    pub reg_count: usize,
    pub(crate) block_out_versions: Vec<Vec<usize>>,
}

impl SSAForm {
    pub fn from_cfg(cfg: CFG) -> Self {
        build_ssa(cfg, usize::from(u8::MAX) + 1)
    }

    pub fn version_at_end(&self, block_id: BlockId, reg: u8) -> usize {
        self.block_out_versions
            .get(block_id)
            .and_then(|versions| versions.get(reg as usize))
            .copied()
            .unwrap_or(0)
    }

    pub fn phi_for(&self, block_id: BlockId, reg: u8) -> Option<&PhiNode> {
        self.blocks
            .get(block_id)?
            .phi_nodes
            .iter()
            .find(|phi| phi.target_reg == reg)
    }
}

pub fn build_ssa(cfg: CFG, reg_count: usize) -> SSAForm {
    SSABuilder::new(cfg, reg_count).build()
}

struct SSABuilder {
    cfg: CFG,
    dominance: DominanceInfo,
    register_count: usize,
    phi_map: BTreeMap<(BlockId, u8), PhiNode>,
    blocks: Vec<Option<SSABlock>>,
    block_out_versions: Vec<Vec<usize>>,
    version_counter: Vec<usize>,
    stacks: Vec<Vec<usize>>,
}

impl SSABuilder {
    fn new(cfg: CFG, reg_count: usize) -> Self {
        let dominance = analyze_dominance(&cfg);
        let register_count = reg_count.min(usize::from(u8::MAX) + 1);
        let block_count = cfg.blocks.len();

        Self {
            cfg,
            dominance,
            register_count,
            phi_map: BTreeMap::new(),
            blocks: vec![None; block_count],
            block_out_versions: vec![vec![0; register_count]; block_count],
            version_counter: vec![0; register_count],
            stacks: vec![Vec::new(); register_count],
        }
    }

    fn build(mut self) -> SSAForm {
        self.insert_phi_nodes();
        self.rename_block(self.cfg.entry);

        let blocks = self
            .blocks
            .into_iter()
            .map(|block| block.expect("every reachable block should be renamed"))
            .collect::<Vec<_>>();

        SSAForm {
            entry: self.cfg.entry,
            exit_blocks: self.cfg.exit_blocks.clone(),
            reg_count: self.register_count,
            cfg: self.cfg,
            dominance: self.dominance,
            blocks,
            block_out_versions: self.block_out_versions,
        }
    }

    fn insert_phi_nodes(&mut self) {
        let def_sites = self.collect_def_sites();

        for (&reg, sites) in &def_sites {
            let mut queued = sites.clone();
            let mut worklist = sites.iter().copied().collect::<VecDeque<_>>();

            while let Some(block) = worklist.pop_front() {
                for &frontier in &self.dominance.frontiers[block] {
                    let key = (frontier, reg);
                    if self.phi_map.contains_key(&key) {
                        continue;
                    }

                    self.phi_map.insert(
                        key,
                        PhiNode {
                            target_reg: reg,
                            target_version: 0,
                            incoming: Vec::new(),
                        },
                    );

                    if queued.insert(frontier) {
                        worklist.push_back(frontier);
                    }
                }
            }
        }
    }

    fn collect_def_sites(&self) -> BTreeMap<u8, BTreeSet<BlockId>> {
        let mut def_sites = BTreeMap::<u8, BTreeSet<BlockId>>::new();

        for block in &self.cfg.blocks {
            for inst in &block.instructions {
                for reg in written_registers(inst) {
                    if (reg as usize) < self.register_count {
                        def_sites.entry(reg).or_default().insert(block.id);
                    }
                }
            }
        }

        def_sites
    }

    fn rename_block(&mut self, block_id: BlockId) {
        let block = self.cfg.blocks[block_id].clone();
        let mut pushed_regs = Vec::new();

        for reg in self.phi_regs(block_id) {
            let version = self.new_version(reg);
            self.stacks[reg as usize].push(version);
            pushed_regs.push(reg);
            if let Some(phi) = self.phi_map.get_mut(&(block_id, reg)) {
                phi.target_version = version;
            }
        }

        let mut instructions = Vec::with_capacity(block.instructions.len());
        for inst in &block.instructions {
            let uses = used_registers(inst)
                .into_iter()
                .map(|reg| (reg, self.current_version(reg)))
                .collect::<Vec<_>>();
            let mut defined = Vec::new();
            for reg in written_registers(inst) {
                let version = self.new_version(reg);
                self.stacks[reg as usize].push(version);
                pushed_regs.push(reg);
                defined.push((reg, version));
            }
            instructions.push(SSAInstr::Original {
                inst: inst.clone(),
                uses,
                defined,
            });
        }

        self.block_out_versions[block_id] = (0..self.register_count)
            .map(|reg| self.current_version(reg as u8))
            .collect::<Vec<_>>();

        let successors = block.successors.clone();
        for successor in successors {
            for reg in self.phi_regs(successor) {
                let version = self.current_version(reg);
                if let Some(phi) = self.phi_map.get_mut(&(successor, reg)) {
                    if !phi.incoming.iter().any(|(pred, _)| *pred == block_id) {
                        phi.incoming.push((block_id, (reg, version)));
                    }
                }
            }
        }

        self.blocks[block_id] = Some(SSABlock {
            id: block_id,
            phi_nodes: self.phis_for_block(block_id),
            instructions,
            successors: block.successors,
            predecessors: block.predecessors,
        });

        let children = self.dominance.tree_children[block_id].clone();
        for child in children {
            self.rename_block(child);
        }

        for reg in pushed_regs.into_iter().rev() {
            self.stacks[reg as usize].pop();
        }
    }

    fn current_version(&self, reg: u8) -> usize {
        self.stacks[reg as usize].last().copied().unwrap_or(0)
    }

    fn new_version(&mut self, reg: u8) -> usize {
        self.version_counter[reg as usize] += 1;
        self.version_counter[reg as usize]
    }

    fn phi_regs(&self, block_id: BlockId) -> Vec<u8> {
        self.phi_map
            .range((block_id, 0)..=(block_id, u8::MAX))
            .map(|((_, reg), _)| *reg)
            .collect::<Vec<_>>()
    }

    fn phis_for_block(&self, block_id: BlockId) -> Vec<PhiNode> {
        self.phi_map
            .range((block_id, 0)..=(block_id, u8::MAX))
            .map(|(_, phi)| phi.clone())
            .collect::<Vec<_>>()
    }
}

fn used_registers(inst: &DecodedInst) -> Vec<u8> {
    instruction_operands(inst).uses
}

fn written_registers(inst: &DecodedInst) -> Vec<u8> {
    instruction_operands(inst).defs
}
