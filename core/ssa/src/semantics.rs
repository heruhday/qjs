use cfg::{ACC_REG, DecodedInst};
use codegen::Opcode;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstructionOperands {
    pub uses: Vec<u8>,
    pub defs: Vec<u8>,
}

pub fn instruction_operands(inst: &DecodedInst) -> InstructionOperands {
    let mut uses = Vec::new();
    let mut defs = Vec::new();

    match inst.opcode {
        Opcode::Mov => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::LoadI
        | Opcode::LoadK
        | Opcode::LoadGlobalIc
        | Opcode::GetGlobal
        | Opcode::GetUpval
        | Opcode::GetScope
        | Opcode::ResolveScope
        | Opcode::NewObj
        | Opcode::NewArr
        | Opcode::NewFunc
        | Opcode::NewThis
        | Opcode::LoadClosure
        | Opcode::TypeofName
        | Opcode::CreateEnv => {
            push_unique_reg(&mut defs, inst.a);
        }
        Opcode::NewClass
        | Opcode::Typeof
        | Opcode::ToNum
        | Opcode::ToStr
        | Opcode::IsUndef
        | Opcode::IsNull
        | Opcode::DeleteProp
        | Opcode::HasProp
        | Opcode::Keys => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::ForIn => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::IteratorNext => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::Spread => {
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::Destructure => {
            // The bytecode only encodes the base destination register, not the destructure width.
            // Track the base write conservatively so the instruction is represented in SSA/IR.
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::SetGlobalIc
        | Opcode::SetGlobal
        | Opcode::SetUpval
        | Opcode::SetScope
        | Opcode::StoreName
        | Opcode::InitName => {
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::LoadName => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut defs, ACC_REG);
        }
        Opcode::LoadArg | Opcode::LoadRestArgs => {
            push_unique_reg(&mut defs, inst.a);
        }
        Opcode::LoadAcc => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::LoadThis => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
        }
        Opcode::Load0 | Opcode::Load1 => {
            push_unique_reg(&mut defs, ACC_REG);
        }
        Opcode::LoadNull | Opcode::LoadTrue | Opcode::LoadFalse => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut defs, inst.a);
        }
        Opcode::IcMiss
        | Opcode::AssertValue
        | Opcode::AssertOk
        | Opcode::AssertFail
        | Opcode::AssertThrows
        | Opcode::AssertDoesNotThrow
        | Opcode::AssertRejects
        | Opcode::AssertDoesNotReject
        | Opcode::AssertEqual
        | Opcode::AssertNotEqual
        | Opcode::AssertDeepEqual
        | Opcode::AssertNotDeepEqual
        | Opcode::AssertStrictEqual
        | Opcode::AssertNotStrictEqual
        | Opcode::AssertDeepStrictEqual
        | Opcode::AssertNotDeepStrictEqual => {
            push_unique_reg(&mut defs, ACC_REG);
        }
        Opcode::Add
        | Opcode::Eq
        | Opcode::Lt
        | Opcode::Lte
        | Opcode::StrictEq
        | Opcode::StrictNeq
        | Opcode::BitAnd
        | Opcode::BitOr
        | Opcode::BitXor
        | Opcode::Shl
        | Opcode::Shr
        | Opcode::Ushr
        | Opcode::Pow
        | Opcode::LogicalAnd
        | Opcode::LogicalOr
        | Opcode::NullishCoalesce
        | Opcode::In
        | Opcode::PrivateIn
        | Opcode::Instanceof
        | Opcode::AddStr
        | Opcode::EqI32Fast
        | Opcode::LtI32Fast => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::AddAcc | Opcode::SubAcc | Opcode::MulAcc | Opcode::DivAcc | Opcode::AddStrAcc => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::AddAccImm8
        | Opcode::SubAccImm8
        | Opcode::MulAccImm8
        | Opcode::DivAccImm8
        | Opcode::IncAcc
        | Opcode::LoadKAddAcc
        | Opcode::LoadKMulAcc
        | Opcode::LoadKSubAcc
        | Opcode::IncAccJmp => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, ACC_REG);
        }
        Opcode::AddI | Opcode::SubI | Opcode::MulI | Opcode::DivI | Opcode::ModI => {
            push_unique_reg(&mut defs, ACC_REG);
            if inst.a != ACC_REG {
                push_unique_reg(&mut defs, inst.a);
            }
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::Mod => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::AddI32
        | Opcode::AddF64
        | Opcode::SubI32
        | Opcode::SubF64
        | Opcode::MulI32
        | Opcode::MulF64
        | Opcode::LtF64
        | Opcode::LteF64
        | Opcode::AddI32Fast
        | Opcode::AddF64Fast
        | Opcode::SubI32Fast
        | Opcode::MulI32Fast => {
            push_unique_reg(&mut defs, ACC_REG);
            if inst.a != ACC_REG {
                push_unique_reg(&mut defs, inst.a);
            }
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::Neg | Opcode::Inc | Opcode::Dec | Opcode::ToPrimitive | Opcode::BitNot => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::GetPropAcc => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::SetPropAcc => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::GetProp | Opcode::GetSuper | Opcode::GetPropIc | Opcode::GetPropMono | Opcode::GetPrivateProp => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::SetProp | Opcode::SetSuper | Opcode::SetPropIc | Opcode::SetPrivateProp => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::GetIdxFast | Opcode::GetElem => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::GetIdxIc => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::SetIdxFast | Opcode::SetElem => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::SetIdxIc => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::GetLengthIc => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::ArrayPushAcc => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, ACC_REG);
        }
        Opcode::Jmp => {}
        Opcode::JmpTrue | Opcode::JmpFalse | Opcode::TestJmpTrue | Opcode::LoadJfalse => {
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::JmpEq
        | Opcode::JmpNeq
        | Opcode::JmpLt
        | Opcode::JmpLtF64
        | Opcode::JmpLte
        | Opcode::JmpLteF64
        | Opcode::JmpLteFalse
        | Opcode::JmpLteFalseF64
        | Opcode::JmpI32Fast
        | Opcode::CmpJmp
        | Opcode::LoadCmpEqJfalse
        | Opcode::LoadCmpLtJfalse => {
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::LoopIncJmp => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, ACC_REG);
        }
        Opcode::IncJmpFalseLoop => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::Switch => {
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::Ret => {
            push_unique_reg(&mut uses, ACC_REG);
        }
        Opcode::RetReg => {
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::RetU => {}
        Opcode::Yield | Opcode::Await => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::ProfileType | Opcode::ProfileCall | Opcode::CheckType | Opcode::CheckStruct => {
            if inst.b != 0 || inst.c != 0 {
                push_unique_reg(&mut uses, inst.b);
            } else {
                push_unique_reg(&mut uses, ACC_REG);
            }
        }
        Opcode::ProfileRet => {
            push_unique_reg(&mut uses, ACC_REG);
        }
        Opcode::CheckIc => {
            push_unique_reg(&mut defs, ACC_REG);
            if inst.b != 0 || inst.c != 0 {
                push_unique_reg(&mut uses, inst.b);
            } else {
                push_unique_reg(&mut uses, ACC_REG);
            }
        }
        Opcode::IcInit | Opcode::IcUpdate => {
            if inst.b != 0 || inst.c != 0 {
                push_unique_reg(&mut uses, inst.b);
            } else {
                push_unique_reg(&mut uses, ACC_REG);
            }
        }
        Opcode::LoopHint
        | Opcode::OsrEntry
        | Opcode::ProfileHotLoop
        | Opcode::OsrExit
        | Opcode::JitHint
        | Opcode::Enter
        | Opcode::Leave
        | Opcode::Try
        | Opcode::EndTry
        | Opcode::Finally => {}
        Opcode::Catch => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut defs, ACC_REG);
        }
        Opcode::Throw => {
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::SafetyCheck => {
            if inst.a != 0 {
                push_unique_reg(&mut uses, inst.a);
            } else {
                push_unique_reg(&mut uses, ACC_REG);
            }
        }
        Opcode::Call1SubI | Opcode::Call2SubIAdd => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::Call
        | Opcode::Construct
        | Opcode::CallIc
        | Opcode::CallIcSuper
        | Opcode::CallThis
        | Opcode::CallMono => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(
                &mut uses,
                if inst.opcode == Opcode::CallThis {
                    inst.b
                } else {
                    0
                },
            );
            let _ = push_call_bundle(
                &mut uses,
                inst.a,
                if inst.opcode == Opcode::CallThis {
                    inst.c
                } else {
                    inst.b
                },
            );
        }
        Opcode::TailCall | Opcode::CallRet => {
            push_unique_reg(&mut uses, 0);
            let _ = push_call_bundle(&mut uses, inst.a, inst.b);
        }
        Opcode::ProfileHotCall => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            let _ = push_call_bundle(&mut uses, inst.b, inst.c);
        }
        Opcode::CallVar | Opcode::CallIcVar | Opcode::CallThisVar => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(
                &mut uses,
                if inst.opcode == Opcode::CallThisVar {
                    inst.b
                } else {
                    0
                },
            );
            push_unique_reg(&mut uses, inst.a);
            if matches!(inst.opcode, Opcode::CallThisVar) {
                push_unique_reg(&mut uses, inst.c);
            } else if inst.a < ACC_REG {
                push_unique_reg(&mut uses, inst.a + 1);
            }
        }
        Opcode::Call0 => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::Call1 => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::Call2 => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::Call3 => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::CallMethod1 => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            let _ = push_call_bundle(&mut uses, inst.a, 1);
        }
        Opcode::CallMethod2 => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            let _ = push_call_bundle(&mut uses, inst.a, 2);
        }
        Opcode::GetPropIcCall | Opcode::GetPropCall => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::AddMov
        | Opcode::LoadAdd
        | Opcode::LoadSub
        | Opcode::LoadMul
        | Opcode::LoadCmpEq
        | Opcode::LoadCmpLt => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
            if matches!(inst.opcode, Opcode::AddMov) {
                push_unique_reg(&mut defs, ACC_REG);
            }
        }
        Opcode::GetPropAccCall => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::GetPropIcMov | Opcode::NewObjInitProp => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::GetPropAddImmSetPropIc => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::AddAccImm8Mov => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, ACC_REG);
        }
        Opcode::LoadThisCall => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
        }
        Opcode::GetLengthIcCall => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::AddStrAccMov | Opcode::MulAccMov => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::GetPropChainAcc => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::LoadArgCall => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
        }
        Opcode::RetIfLteI => {
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::AddAccReg => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::Call1Add => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            push_unique_reg(&mut uses, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::Call2Add => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, 0);
            push_unique_reg(&mut uses, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::LoadKAdd => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, ACC_REG);
        }
        Opcode::LoadKCmp | Opcode::LoadGetProp => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::LoadInc | Opcode::LoadDec => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::CallMethodIc | Opcode::CallMethod2Ic => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
        }
        Opcode::LoadGetPropCmpEq => {
            push_unique_reg(&mut defs, ACC_REG);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.c);
        }
        Opcode::GetProp2Ic | Opcode::GetProp3Ic => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::GetPropElem => {
            push_unique_reg(&mut defs, inst.a);
            push_unique_reg(&mut uses, inst.a);
            push_unique_reg(&mut uses, inst.b);
        }
        Opcode::Reserved(_) => {}
    }

    InstructionOperands { uses, defs }
}

fn push_unique_reg(regs: &mut Vec<u8>, reg: u8) {
    if !regs.contains(&reg) {
        regs.push(reg);
    }
}

fn push_call_bundle(regs: &mut Vec<u8>, base: u8, arg_count: u8) -> bool {
    let last = base as usize + arg_count as usize;
    if last >= ACC_REG as usize {
        return false;
    }

    for reg in base..=base + arg_count {
        push_unique_reg(regs, reg);
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfg::decode_word;

    #[test]
    fn covers_every_encoded_opcode() {
        for raw in 0u32..=255 {
            let inst = decode_word(0, raw);
            let _ = instruction_operands(&inst);
        }
    }

    #[test]
    fn load_true_defines_acc_and_destination() {
        let inst = DecodedInst {
            raw: Opcode::LoadTrue.as_u8() as u32,
            pc: 0,
            opcode: Opcode::LoadTrue,
            a: 7,
            b: 0,
            c: 0,
            bx: 0,
            sbx: 0,
        };
        let ops = instruction_operands(&inst);
        assert_eq!(ops.uses, Vec::<u8>::new());
        assert_eq!(ops.defs, vec![ACC_REG, 7]);
    }

    #[test]
    fn call_uses_contiguous_bundle() {
        let inst = DecodedInst {
            raw: Opcode::Call.as_u8() as u32,
            pc: 0,
            opcode: Opcode::Call,
            a: 5,
            b: 2,
            c: 0,
            bx: 0,
            sbx: 0,
        };
        let ops = instruction_operands(&inst);
        assert_eq!(ops.uses, vec![0, 5, 6, 7]);
        assert_eq!(ops.defs, vec![ACC_REG]);
    }
}
