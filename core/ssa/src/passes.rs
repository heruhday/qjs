mod cfg_simplification;
mod constant_folding;
mod copy_propagation;
mod dead_code_elimination;
mod global_value_numbering;
mod licm;
mod sccp;
mod value_range_propagation;

use crate::ir::IRFunction;

pub use cfg_simplification::CfgSimplification;
pub use constant_folding::ConstantFolding;
pub use copy_propagation::CopyPropagation;
pub use dead_code_elimination::DeadCodeElimination;
pub use global_value_numbering::GlobalValueNumbering;
pub use licm::LoopInvariantCodeMotion;
pub use sccp::SparseConditionalConstantPropagation;
pub use value_range_propagation::ValueRangePropagation;

pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&self, ir: &mut IRFunction) -> bool;

    fn is_structural(&self) -> bool {
        false
    }
}

pub struct PassManager {
    passes: Vec<Box<dyn Pass>>,
    max_iterations: usize,
}

impl PassManager {
    pub fn new() -> Self {
        Self {
            passes: Vec::new(),
            max_iterations: 10,
        }
    }

    pub fn add_pass<P: Pass + 'static>(&mut self, pass: P) {
        self.passes.push(Box::new(pass));
    }

    pub fn set_max_iterations(&mut self, max_iterations: usize) {
        self.max_iterations = max_iterations.max(1);
    }

    pub fn run(&self, ir: &mut IRFunction) -> bool {
        let mut changed = false;

        for _ in 0..self.max_iterations {
            let mut iteration_changed = false;
            let mut restart_iteration = false;

            for pass in &self.passes {
                let pass_changed = pass.run(ir);
                iteration_changed |= pass_changed;
                changed |= pass_changed;

                if pass_changed && pass.is_structural() {
                    restart_iteration = true;
                    break;
                }
            }

            if !iteration_changed {
                break;
            }

            if restart_iteration {
                continue;
            }
        }

        changed
    }
}
