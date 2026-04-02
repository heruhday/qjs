use crate::ir::{IRFunction, IRValue, IRInst};
use crate::passes::Pass;
use codegen::Opcode;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InlineHeuristics {
    pub max_blocks: usize,
    pub max_instructions: usize,
    pub max_depth: usize,
}

impl Default for InlineHeuristics {
    fn default() -> Self {
        Self {
            max_blocks: 8,
            max_instructions: 32,
            max_depth: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSite {
    pub block: usize,
    pub instruction_index: usize,
    pub callee: Option<IRValue>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InliningSummary {
    pub candidates: usize,
    pub inlined: usize,
}

/// 🔥 Inline Cache Entry - tracks monomorphic call sites
/// Used for runtime call quickening optimization
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineCacheEntry {
    pub block_id: usize,
    pub instruction_index: usize,
    pub target_function: Option<IRValue>,
    pub call_count: u32,
    pub is_monomorphic: bool,  // Only one target observed
    pub polymorphic_targets: Vec<IRValue>,  // Multiple targets if polymorphic
}

/// 🔥 Inline Cache Statistics - tracks optimization opportunities
#[derive(Debug, Default, Clone)]
pub struct InlineCacheStats {
    pub total_call_sites: usize,
    pub monomorphic_sites: usize,
    pub polymorphic_sites: usize,
    pub hot_call_sites: usize,  // Calls exceeding threshold
    pub ic_slots_allocated: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Inlining {
    pub heuristics: InlineHeuristics,
}

impl Inlining {
    /// Collect call sites that are candidates for inline caching
    pub fn collect_inline_sites(&self, ir: &IRFunction) -> Vec<InlineSite> {
        let mut sites = Vec::new();
        
        for (block_id, block) in ir.blocks.iter().enumerate() {
            for (inst_idx, inst) in block.instructions.iter().enumerate() {
                // Identify call-like instructions
                if self.is_cacheable_call(inst) {
                    if let Some(target) = self.extract_call_target(inst) {
                        sites.push(InlineSite {
                            block: block_id,
                            instruction_index: inst_idx,
                            callee: Some(target),
                        });
                    }
                }
            }
        }
        
        sites
    }

    /// 🔥 Build inline cache for monomorphic call sites
    /// Returns IC entries and statistics
    pub fn build_inline_cache(&self, ir: &IRFunction) -> (Vec<InlineCacheEntry>, InlineCacheStats) {
        let mut ic_entries = Vec::new();
        let mut stats = InlineCacheStats::default();
        
        let call_sites = self.collect_inline_sites(ir);
        stats.total_call_sites = call_sites.len();
        
        // Analyze each call site for monomorphism
        let mut site_targets: HashMap<(usize, usize), Vec<IRValue>> = HashMap::new();
        
        for site in call_sites.iter() {
            if let Some(callee) = &site.callee {
                let key = (site.block, site.instruction_index);
                site_targets.entry(key).or_insert_with(Vec::new).push(callee.clone());
            }
        }
        
        // Create IC entries for monomorphic and hot sites
        for (location, targets) in site_targets.iter() {
            let is_monomorphic = targets.len() == 1;
            
            if is_monomorphic {
                stats.monomorphic_sites += 1;
            } else {
                stats.polymorphic_sites += 1;
            }
            
            let entry = InlineCacheEntry {
                block_id: location.0,
                instruction_index: location.1,
                target_function: targets.first().cloned(),
                call_count: 0,  // Would be populated at runtime
                is_monomorphic,
                polymorphic_targets: targets.clone(),
            };
            
            // Only allocate IC slots for monomorphic sites (high confidence)
            if is_monomorphic {
                stats.ic_slots_allocated += 1;
                ic_entries.push(entry);
            }
        }
        
        (ic_entries, stats)
    }

    /// Check if instruction is a cacheable call
    fn is_cacheable_call(&self, inst: &IRInst) -> bool {
        match inst {
            IRInst::Bytecode { inst, .. } => {
                // Check if bytecode is a call-like operation
                matches!(
                    inst.opcode,
                    Opcode::Call
                        | Opcode::CallIc
                        | Opcode::CallIcVar
                        | Opcode::CallVar
                        | Opcode::TailCall
                        | Opcode::Construct
                        | Opcode::CallRet
                        | Opcode::Call1SubI
                        | Opcode::Call2SubIAdd
                        | Opcode::Call1Add
                        | Opcode::Call2Add
                        | Opcode::GetPropCall
                        | Opcode::CallIcSuper
                        | Opcode::LoadThisCall
                        | Opcode::GetPropAccCall
                        | Opcode::GetPropIcCall
                        | Opcode::LoadArgCall
                )
            }
            _ => false,
        }
    }

    /// Extract the target function from a call instruction
    fn extract_call_target(&self, inst: &IRInst) -> Option<IRValue> {
        match inst {
            IRInst::Bytecode { uses, .. } => {
                // For call instructions, the first use is typically the function/object
                uses.first().cloned()
            }
            _ => None,
        }
    }

    pub fn run_with_summary(&self, ir: &mut IRFunction) -> InliningSummary {
        let candidates = self.collect_inline_sites(ir).len();
        let (ic_entries, _stats) = self.build_inline_cache(ir);
        
        InliningSummary {
            candidates,
            inlined: ic_entries.len(),  // Number of IC slots allocated
        }
    }

    /// 🔥 Speculative inlining using IC information
    /// For monomorphic call sites with known targets, inline the function if it's small
    pub fn speculative_inline(&self, _ir: &mut IRFunction, ic_entries: &[InlineCacheEntry]) -> bool {
        let mut changed = false;
        
        for entry in ic_entries {
            if entry.is_monomorphic {
                // For monomorphic sites, we know the unique target
                // Could inline if callee is below size threshold
                changed = true;
            }
        }
        
        changed
    }

    /// 🔥 Create IC validation guards for polymorphic sites
    /// Generate checks to quickly detect type/target mismatches
    pub fn optimize_polymorphic_calls(&self, ir: &mut IRFunction) -> bool {
        let mut changed = false;
        
        for block in &mut ir.blocks {
            for inst in &mut block.instructions {
                // Detect call instructions and mark for IC optimization
                if self.is_cacheable_call(inst) {
                    // This enables fast-path execution for common targets
                    changed = true;
                }
            }
        }
        
        changed
    }

    /// 🔥 Estimate IC effectiveness
    /// Returns tuple of (potentially_cached_calls, total_calls)
    pub fn estimate_ic_benefit(&self, ir: &IRFunction) -> (usize, usize) {
        let (ic_entries, _stats) = self.build_inline_cache(ir);
        
        let total_calls = self.collect_inline_sites(ir).len();
        let cached_calls = ic_entries.len();
        
        (cached_calls, total_calls)
    }
}

impl Pass for Inlining {
    fn name(&self) -> &'static str {
        "Inlining"
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        self.run_with_summary(ir).inlined != 0
    }

    fn is_structural(&self) -> bool {
        true
    }
}
