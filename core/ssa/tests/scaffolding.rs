use std::collections::HashMap;

use ssa::{
    AliasAnalysis, AliasResult, EscapeAnalysis, EscapeKind, IRBlock, IRFunction, IRInst,
    IRTerminator, IRValue, Inlining, JitLowering, LoadElimination, MachineCodeEmitter, Pass,
    RegisterAllocator, ScalarReplacement, StoreElimination,
};
use value::make_int32;

fn simple_ir() -> IRFunction {
    IRFunction {
        blocks: vec![IRBlock {
            id: 0,
            instructions: vec![IRInst::LoadConst {
                dst: IRValue::Register(0, 0),
                value: make_int32(1),
            }],
            terminator: IRTerminator::Return {
                value: Some(IRValue::Register(0, 0)),
            },
            successors: Vec::new(),
            predecessors: Vec::new(),
        }],
        entry: 0,
        exit_blocks: vec![0],
        constants: Vec::new(),
    }
}

#[test]
fn scaffold_passes_are_conservative_noops() {
    let mut ir = simple_ir();
    let escape = EscapeAnalysis;
    let alias = AliasAnalysis::new(HashMap::new());

    assert_eq!(
        escape.classify_value(&IRValue::Register(0, 0)),
        EscapeKind::Unknown
    );
    assert_eq!(
        alias.query(&IRValue::Register(0, 0), &IRValue::Register(1, 0)),
        AliasResult::MayAlias
    );
    assert!(!Inlining::default().run(&mut ir));
    assert!(!ScalarReplacement.run_with_escape(&mut ir, &HashMap::new()));
    assert!(!LoadElimination.run_with_alias(&mut ir, &alias));
    assert!(!StoreElimination.run_with_alias(&mut ir, &alias));
}

#[test]
fn jit_scaffolding_preserves_basic_control_flow_shape() {
    let ir = simple_ir();
    let mut machine = JitLowering.lower(&ir);
    let regalloc = RegisterAllocator::default().allocate(&mut machine);
    let code = MachineCodeEmitter.emit(&machine);

    assert_eq!(machine.entry, ir.entry);
    assert_eq!(machine.blocks.len(), ir.blocks.len());
    assert!(!regalloc.changed);
    assert!(code.bytes.is_empty());
}
