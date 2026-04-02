//! Assembly code generator for QJL bytecode
//!
//! This module provides functionality to disassemble bytecode into human-readable
//! assembly code based on the BYTECODE_SPEC_v2.0.md specification.

use codegen::{CompiledBytecode, Opcode};
use value::JSValue;

/// Instruction format for disassembly
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// ABC: three 8-bit register operands
    ABC,
    /// ABx: A + 16-bit unsigned immediate (B and C combined)
    ABx,
    /// AsBx: A + 16-bit signed offset (B and C combined)
    AsBx,
    /// A: single register operand
    A,
    /// BC: two registers, result in accumulator
    BC,
}

/// Assembly instruction with decoded operands
#[derive(Debug, Clone)]
pub struct AsmInstruction {
    /// Program counter (byte offset)
    pub pc: usize,
    /// Raw instruction word
    pub raw: u32,
    /// Opcode
    pub opcode: Opcode,
    /// Format
    pub format: Format,
    /// Decoded A operand
    pub a: u8,
    /// Decoded B operand
    pub b: u8,
    /// Decoded C operand
    pub c: u8,
    /// Decoded Bx/ABx operand (unsigned 16-bit)
    pub bx: u16,
    /// Decoded sBx/AsBx operand (signed 16-bit)
    pub sbx: i16,
}

impl AsmInstruction {
    /// Decode a raw instruction word
    pub fn decode(pc: usize, raw: u32) -> Self {
        let opcode = Opcode::from((raw & 0xFF) as u8);
        let a = ((raw >> 8) & 0xFF) as u8;
        let b = ((raw >> 16) & 0xFF) as u8;
        let c = ((raw >> 24) & 0xFF) as u8;
        let bx = ((raw >> 16) & 0xFFFF) as u16;
        let sbx = bx as i16;

        // Determine format based on opcode
        let format = match opcode {
            Opcode::Mov
            | Opcode::GetPropIc
            | Opcode::SetPropIc
            | Opcode::AddI
            | Opcode::SubI
            | Opcode::MulI
            | Opcode::DivI
            | Opcode::ModI
            | Opcode::GetIdxFast
            | Opcode::SetIdxFast
            | Opcode::GetProp
            | Opcode::SetProp
            | Opcode::GetIdxIc
            | Opcode::SetIdxIc
            | Opcode::GetLengthIc
            | Opcode::GetSuper
            | Opcode::SetSuper
            | Opcode::DeleteProp
            | Opcode::HasProp
            | Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLtF64
            | Opcode::JmpLte
            | Opcode::JmpLteF64
            | Opcode::JmpLteFalse
            | Opcode::JmpLteFalseF64
            | Opcode::GetPropIcCall
            | Opcode::Call1SubI
            | Opcode::AddMov
            | Opcode::GetPropIcMov
            | Opcode::GetPropAddImmSetPropIc
            | Opcode::GetPropChainAcc
            | Opcode::NewObjInitProp
            | Opcode::ProfileHotCall
            | Opcode::AddI32
            | Opcode::AddF64
            | Opcode::SubI32
            | Opcode::SubF64
            | Opcode::MulI32
            | Opcode::MulF64
            | Opcode::LtF64
            | Opcode::LteF64 => Format::ABC,

            Opcode::LoadK
            | Opcode::LoadGlobalIc
            | Opcode::SetGlobalIc
            | Opcode::NewFunc
            | Opcode::GetGlobal
            | Opcode::SetGlobal
            | Opcode::ResolveScope
            | Opcode::LoadName
            | Opcode::StoreName
            | Opcode::InitName
            | Opcode::TypeofName
            | Opcode::LoadKAddAcc
            | Opcode::LoadKMulAcc
            | Opcode::LoadKSubAcc => Format::ABx,

            Opcode::Jmp
            | Opcode::LoadI
            | Opcode::JmpTrue
            | Opcode::JmpFalse
            | Opcode::LoopIncJmp
            | Opcode::Try
            | Opcode::IncJmpFalseLoop
            | Opcode::EqJmpTrue
            | Opcode::LtJmp
            | Opcode::EqJmpFalse
            | Opcode::IncAccJmp
            | Opcode::TestJmpTrue
            | Opcode::LteJmpLoop => Format::AsBx,

            Opcode::AddAccImm8
            | Opcode::IncAcc
            | Opcode::LoadThis
            | Opcode::Load0
            | Opcode::Load1
            | Opcode::AddAcc
            | Opcode::SubAcc
            | Opcode::MulAcc
            | Opcode::DivAcc
            | Opcode::LoadNull
            | Opcode::LoadTrue
            | Opcode::LoadFalse
            | Opcode::SubAccImm8
            | Opcode::MulAccImm8
            | Opcode::DivAccImm8
            | Opcode::AddStrAcc
            | Opcode::BitNot
            | Opcode::Neg
            | Opcode::Inc
            | Opcode::Dec
            | Opcode::ToPrimitive
            | Opcode::ArrayPushAcc
            | Opcode::NewObj
            | Opcode::NewArr
            | Opcode::NewClass
            | Opcode::GetUpval
            | Opcode::SetUpval
            | Opcode::GetScope
            | Opcode::SetScope
            | Opcode::ForIn
            | Opcode::IteratorNext
            | Opcode::Spread
            | Opcode::Destructure
            | Opcode::CreateEnv
            | Opcode::LoadClosure
            | Opcode::NewThis
            | Opcode::LoopHint
            | Opcode::Ret
            | Opcode::RetU
            | Opcode::RetReg
            | Opcode::Leave
            | Opcode::Yield
            | Opcode::Await
            | Opcode::EndTry
            | Opcode::Finally
            | Opcode::ProfileRet
            | Opcode::IcMiss
            | Opcode::OsrEntry
            | Opcode::ProfileHotLoop
            | Opcode::OsrExit
            | Opcode::JitHint
            | Opcode::SafetyCheck
            | Opcode::AssertValue
            | Opcode::AssertOk
            | Opcode::AssertFail
            | Opcode::AssertThrows
            | Opcode::AssertDoesNotThrow
            | Opcode::AssertRejects
            | Opcode::AssertDoesNotReject => Format::A,

            Opcode::Add
            | Opcode::Mod
            | Opcode::Eq
            | Opcode::Lt
            | Opcode::Lte
            | Opcode::AddStr
            | Opcode::GetPropAcc
            | Opcode::SetPropAcc
            | Opcode::StrictEq
            | Opcode::StrictNeq
            | Opcode::GetPropAccCall
            | Opcode::GetLengthIcCall
            | Opcode::AssertEqual
            | Opcode::AssertNotEqual
            | Opcode::AssertDeepEqual
            | Opcode::AssertNotDeepEqual
            | Opcode::AssertStrictEqual
            | Opcode::AssertNotStrictEqual
            | Opcode::AssertDeepStrictEqual
            | Opcode::AssertNotDeepStrictEqual
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
            | Opcode::Instanceof => Format::BC,

            Opcode::Call
            | Opcode::TailCall
            | Opcode::Construct
            | Opcode::CallIc
            | Opcode::CallIcSuper
            | Opcode::LoadThisCall
            | Opcode::LoadArgCall => Format::ABC, // Special: A B format

            Opcode::Typeof
            | Opcode::ToNum
            | Opcode::ToStr
            | Opcode::IsUndef
            | Opcode::IsNull
            | Opcode::LoadArg
            | Opcode::LoadAcc
            | Opcode::Keys
            | Opcode::Switch
            | Opcode::Enter
            | Opcode::Throw
            | Opcode::Catch
            | Opcode::CallVar
            | Opcode::CallIcVar
            | Opcode::ProfileType
            | Opcode::ProfileCall
            | Opcode::CheckType
            | Opcode::CheckStruct
            | Opcode::CheckIc
            | Opcode::IcInit
            | Opcode::IcUpdate
            | Opcode::AddAccImm8Mov
            | Opcode::AddStrAccMov
            | Opcode::MulAccMov => Format::ABC,

            // Superinstructions
            Opcode::RetIfLteI
            | Opcode::AddAccReg
            | Opcode::Call2SubIAdd
            | Opcode::Call1Add
            | Opcode::Call2Add
            | Opcode::LoadKAdd
            | Opcode::LoadKCmp
            | Opcode::CmpJmp
            | Opcode::GetPropCall
            | Opcode::CallRet => Format::ABC,
            // Specialized opcodes
            Opcode::AddI32Fast
            | Opcode::AddF64Fast
            | Opcode::SubI32Fast
            | Opcode::MulI32Fast
            | Opcode::EqI32Fast
            | Opcode::LtI32Fast
            | Opcode::JmpI32Fast
            | Opcode::GetPropMono
            | Opcode::CallMono => Format::ABC,
            // Call opcodes
            Opcode::Call0 | Opcode::Call1 | Opcode::Call2 | Opcode::Call3 => Format::ABC,
            Opcode::CallMethod1 | Opcode::CallMethod2 => Format::ABx,
            // New arithmetic superinstructions
            Opcode::LoadAdd
            | Opcode::LoadSub
            | Opcode::LoadMul
            | Opcode::LoadInc
            | Opcode::LoadDec => Format::ABC,
            // New comparison superinstructions
            Opcode::LoadCmpEq
            | Opcode::LoadCmpLt
            | Opcode::LoadJfalse
            | Opcode::LoadCmpEqJfalse
            | Opcode::LoadCmpLtJfalse
            | Opcode::LoadGetProp
            | Opcode::LoadGetPropCmpEq => Format::ABC,
            // Pareto 80% property access superinstructions with IC
            Opcode::GetProp2Ic
            | Opcode::GetProp3Ic
            | Opcode::GetElem
            | Opcode::SetElem
            | Opcode::GetPropElem
            | Opcode::CallMethodIc
            | Opcode::CallMethod2Ic => Format::ABC,
            Opcode::Reserved(_) => Format::ABC,
        };

        Self {
            pc,
            raw,
            opcode,
            format,
            a,
            b,
            c,
            bx,
            sbx,
        }
    }

    /// Format instruction as human-readable assembly
    pub fn to_asm(&self, constants: &[JSValue]) -> String {
        let pc_byte_offset = self.pc * 4;
        let opcode_str = self.opcode_to_mnemonic();

        match self.format {
            Format::ABC => {
                match self.opcode {
                    Opcode::Call
                    | Opcode::TailCall
                    | Opcode::Construct
                    | Opcode::CallIc
                    | Opcode::CallIcSuper
                    | Opcode::LoadThisCall
                    | Opcode::LoadArgCall => {
                        // A B format
                        format!(
                            "{:04X}: {} r{}, {}",
                            pc_byte_offset, opcode_str, self.a, self.b
                        )
                    }
                    Opcode::Call1SubI | Opcode::Call2SubIAdd => {
                        format!(
                            "{:04X}: {} r{}, r{}, {}",
                            pc_byte_offset, opcode_str, self.a, self.b, self.c as i8
                        )
                    }
                    Opcode::CallRet => {
                        format!(
                            "{:04X}: {} r{}, {}",
                            pc_byte_offset, opcode_str, self.a, self.b
                        )
                    }
                    Opcode::LoadInc | Opcode::LoadDec => {
                        format!(
                            "{:04X}: {} r{}, r{}",
                            pc_byte_offset, opcode_str, self.a, self.b
                        )
                    }
                    Opcode::LoadJfalse => {
                        let target_pc = (self.pc as isize + self.b as i8 as isize + 1) as usize;
                        let target_byte_offset = target_pc.saturating_mul(4);
                        format!(
                            "{:04X}: {} r{}, -> {:04X}",
                            pc_byte_offset, opcode_str, self.a, target_byte_offset
                        )
                    }
                    Opcode::LoadCmpEqJfalse | Opcode::LoadCmpLtJfalse => {
                        let target_pc = (self.pc as isize + self.c as i8 as isize + 1) as usize;
                        let target_byte_offset = target_pc.saturating_mul(4);
                        format!(
                            "{:04X}: {} r{}, r{}, -> {:04X}",
                            pc_byte_offset, opcode_str, self.a, self.b, target_byte_offset
                        )
                    }
                    Opcode::JmpEq
                    | Opcode::JmpNeq
                    | Opcode::JmpLt
                    | Opcode::JmpLtF64
                    | Opcode::JmpLte
                    | Opcode::JmpLteF64
                    | Opcode::JmpLteFalse
                    | Opcode::JmpLteFalseF64 => {
                        let target_pc = (self.pc as isize + self.c as i8 as isize + 1) as usize;
                        let target_byte_offset = target_pc.saturating_mul(4);
                        format!(
                            "{:04X}: {} r{}, r{}, -> {:04X}",
                            pc_byte_offset, opcode_str, self.a, self.b, target_byte_offset
                        )
                    }
                    _ => {
                        // Standard ABC format
                        format!(
                            "{:04X}: {} r{}, r{}, r{}",
                            pc_byte_offset, opcode_str, self.a, self.b, self.c
                        )
                    }
                }
            }
            Format::ABx => {
                match self.opcode {
                    Opcode::LoadK | Opcode::NewFunc => {
                        // Load constant
                        let const_idx = self.bx as usize;
                        let const_val = if const_idx < constants.len() {
                            format!("const[{}]", const_idx)
                        } else {
                            "const[out of bounds]".to_string()
                        };
                        format!(
                            "{:04X}: {} r{}, {}",
                            pc_byte_offset, opcode_str, self.a, const_val
                        )
                    }
                    Opcode::LoadGlobalIc
                    | Opcode::SetGlobalIc
                    | Opcode::GetGlobal
                    | Opcode::SetGlobal => {
                        format!(
                            "{:04X}: {} r{}, global[{}]",
                            pc_byte_offset, opcode_str, self.a, self.bx
                        )
                    }
                    Opcode::ResolveScope
                    | Opcode::LoadName
                    | Opcode::StoreName
                    | Opcode::InitName
                    | Opcode::TypeofName => {
                        format!(
                            "{:04X}: {} r{}, identifier[{}]",
                            pc_byte_offset, opcode_str, self.a, self.bx
                        )
                    }
                    Opcode::CallMethod1 => {
                        format!(
                            "{:04X}: {} r{}, property[{}], r{}",
                            pc_byte_offset,
                            opcode_str,
                            self.a,
                            self.bx,
                            self.a.saturating_add(1)
                        )
                    }
                    Opcode::CallMethod2 => {
                        format!(
                            "{:04X}: {} r{}, property[{}], r{}, r{}",
                            pc_byte_offset,
                            opcode_str,
                            self.a,
                            self.bx,
                            self.a.saturating_add(1),
                            self.a.saturating_add(2)
                        )
                    }
                    Opcode::LoadKAddAcc | Opcode::LoadKMulAcc | Opcode::LoadKSubAcc => {
                        let const_idx = self.bx as usize;
                        let const_val = if const_idx < constants.len() {
                            format!("const[{}]", const_idx)
                        } else {
                            "const[out of bounds]".to_string()
                        };
                        format!("{:04X}: {} {}", pc_byte_offset, opcode_str, const_val)
                    }
                    _ => {
                        format!(
                            "{:04X}: {} r{}, {}",
                            pc_byte_offset, opcode_str, self.a, self.bx
                        )
                    }
                }
            }
            Format::AsBx => {
                // Handle overflow in multiplication
                let target_pc = (self.pc as isize + self.sbx as isize + 1) as usize;
                let target_byte_offset = target_pc.saturating_mul(4);
                match self.opcode {
                    Opcode::Jmp => {
                        format!(
                            "{:04X}: {} -> {:04X}",
                            pc_byte_offset, opcode_str, target_byte_offset
                        )
                    }
                    Opcode::LoadI => {
                        format!(
                            "{:04X}: {} r{}, {}",
                            pc_byte_offset, opcode_str, self.a, self.sbx
                        )
                    }
                    Opcode::JmpTrue
                    | Opcode::JmpFalse
                    | Opcode::LoopIncJmp
                    | Opcode::Try
                    | Opcode::IncJmpFalseLoop
                    | Opcode::EqJmpTrue
                    | Opcode::LtJmp
                    | Opcode::EqJmpFalse
                    | Opcode::IncAccJmp
                    | Opcode::TestJmpTrue
                    | Opcode::LteJmpLoop => {
                        format!(
                            "{:04X}: {} r{}, -> {:04X}",
                            pc_byte_offset, opcode_str, self.a, target_byte_offset
                        )
                    }
                    _ => {
                        format!(
                            "{:04X}: {} r{}, {}",
                            pc_byte_offset, opcode_str, self.a, self.sbx
                        )
                    }
                }
            }
            Format::A => match self.opcode {
                Opcode::AddAccImm8
                | Opcode::SubAccImm8
                | Opcode::MulAccImm8
                | Opcode::DivAccImm8 => {
                    format!("{:04X}: {} {}", pc_byte_offset, opcode_str, self.b as i8)
                }
                Opcode::AddAcc
                | Opcode::SubAcc
                | Opcode::MulAcc
                | Opcode::DivAcc
                | Opcode::Neg
                | Opcode::Inc
                | Opcode::Dec
                | Opcode::ToPrimitive
                | Opcode::AddStrAcc => {
                    format!("{:04X}: {} r{}", pc_byte_offset, opcode_str, self.b)
                }
                Opcode::ArrayPushAcc
                | Opcode::GetUpval
                | Opcode::SetUpval
                | Opcode::GetScope
                | Opcode::SetScope
                | Opcode::ForIn
                | Opcode::IteratorNext
                | Opcode::Spread
                | Opcode::Destructure
                | Opcode::CreateEnv
                | Opcode::LoadClosure
                | Opcode::NewThis => {
                    format!("{:04X}: {} r{}", pc_byte_offset, opcode_str, self.a)
                }
                Opcode::IncAcc
                | Opcode::LoadThis
                | Opcode::Load0
                | Opcode::Load1
                | Opcode::LoadNull
                | Opcode::LoadTrue
                | Opcode::LoadFalse
                | Opcode::NewObj
                | Opcode::NewArr
                | Opcode::NewClass
                | Opcode::LoopHint
                | Opcode::Ret
                | Opcode::RetU
                | Opcode::Leave
                | Opcode::Yield
                | Opcode::Await
                | Opcode::EndTry
                | Opcode::Finally
                | Opcode::ProfileRet
                | Opcode::IcMiss
                | Opcode::OsrEntry
                | Opcode::ProfileHotLoop
                | Opcode::OsrExit
                | Opcode::JitHint
                | Opcode::SafetyCheck
                | Opcode::AssertValue
                | Opcode::AssertOk
                | Opcode::AssertFail
                | Opcode::AssertThrows
                | Opcode::AssertDoesNotThrow
                | Opcode::AssertRejects
                | Opcode::AssertDoesNotReject => {
                    format!("{:04X}: {}", pc_byte_offset, opcode_str)
                }
                _ => {
                    format!("{:04X}: {} r{}", pc_byte_offset, opcode_str, self.a)
                }
            },
            Format::BC => match self.opcode {
                Opcode::GetPropAccCall | Opcode::GetLengthIcCall => {
                    format!(
                        "{:04X}: {} r{}, r{}",
                        pc_byte_offset, opcode_str, self.b, self.c
                    )
                }
                _ => {
                    format!(
                        "{:04X}: {} r{}, r{}",
                        pc_byte_offset, opcode_str, self.b, self.c
                    )
                }
            },
        }
    }

    /// Convert opcode to mnemonic string (clean format without underscores)
    fn opcode_to_clean_mnemonic(&self) -> &'static str {
        match self.opcode {
            Opcode::Mov => "mov",
            Opcode::LoadK => "load_k",
            Opcode::Add => "add",
            Opcode::GetPropIc => "get_prop_ic",
            Opcode::Call => "call",
            Opcode::Jmp => "jmp",
            Opcode::LoadI => "load_i",
            Opcode::JmpTrue => "jmp_true",
            Opcode::JmpFalse => "jmp_false",
            Opcode::SetPropIc => "set_prop_ic",
            Opcode::AddAccImm8 => "add_acc_imm8",
            Opcode::IncAcc => "inc_acc",
            Opcode::LoadThis => "load_this",
            Opcode::Load0 => "load_0",
            Opcode::Load1 => "load_1",
            Opcode::Eq => "eq",
            Opcode::Lt => "lt",
            Opcode::Lte => "lte",
            Opcode::AddAcc => "add_acc",
            Opcode::SubAcc => "sub_acc",
            Opcode::MulAcc => "mul_acc",
            Opcode::DivAcc => "div_acc",
            Opcode::LoadNull => "load_null",
            Opcode::LoadTrue => "load_true",
            Opcode::LoadFalse => "load_false",
            Opcode::LoadGlobalIc => "load_global_ic",
            Opcode::SetGlobalIc => "set_global_ic",
            Opcode::Typeof => "typeof",
            Opcode::ToNum => "to_num",
            Opcode::ToStr => "to_str",
            Opcode::IsUndef => "is_undef",
            Opcode::IsNull => "is_null",
            Opcode::SubAccImm8 => "sub_acc_imm8",
            Opcode::MulAccImm8 => "mul_acc_imm8",
            Opcode::DivAccImm8 => "div_acc_imm8",
            Opcode::AddStrAcc => "add_str_acc",
            Opcode::AddI => "add_i",
            Opcode::SubI => "sub_i",
            Opcode::MulI => "mul_i",
            Opcode::DivI => "div_i",
            Opcode::ModI => "mod_i",
            Opcode::Mod => "mod",
            Opcode::Neg => "neg",
            Opcode::Inc => "inc",
            Opcode::Dec => "dec",
            Opcode::AddStr => "add_str",
            Opcode::ToPrimitive => "to_primitive",
            Opcode::GetPropAcc => "get_prop_acc",
            Opcode::SetPropAcc => "set_prop_acc",
            Opcode::GetIdxFast => "get_idx_fast",
            Opcode::SetIdxFast => "set_idx_fast",
            Opcode::LoadArg => "load_arg",
            Opcode::LoadAcc => "load_acc",
            Opcode::StrictEq => "strict_eq",
            Opcode::StrictNeq => "strict_neq",
            Opcode::BitAnd => "bit_and",
            Opcode::BitOr => "bit_or",
            Opcode::BitXor => "bit_xor",
            Opcode::BitNot => "bit_not",
            Opcode::Shl => "shl",
            Opcode::Shr => "shr",
            Opcode::Ushr => "ushr",
            Opcode::Pow => "pow",
            Opcode::LogicalAnd => "logical_and",
            Opcode::LogicalOr => "logical_or",
            Opcode::NullishCoalesce => "nullish_coalesce",
            Opcode::In => "in",
            Opcode::Instanceof => "instanceof",
            Opcode::GetLengthIc => "get_length_ic",
            Opcode::ArrayPushAcc => "array_push_acc",
            Opcode::NewObj => "new_obj",
            Opcode::NewArr => "new_arr",
            Opcode::NewFunc => "new_func",
            Opcode::NewClass => "new_class",
            Opcode::GetProp => "get_prop",
            Opcode::SetProp => "set_prop",
            Opcode::GetIdxIc => "get_idx_ic",
            Opcode::SetIdxIc => "set_idx_ic",
            Opcode::GetGlobal => "get_global",
            Opcode::SetGlobal => "set_global",
            Opcode::GetUpval => "get_upval",
            Opcode::SetUpval => "set_upval",
            Opcode::GetScope => "get_scope",
            Opcode::SetScope => "set_scope",
            Opcode::ResolveScope => "resolve_scope",
            Opcode::GetSuper => "get_super",
            Opcode::SetSuper => "set_super",
            Opcode::DeleteProp => "delete_prop",
            Opcode::HasProp => "has_prop",
            Opcode::Keys => "keys",
            Opcode::ForIn => "for_in",
            Opcode::IteratorNext => "iterator_next",
            Opcode::Spread => "spread",
            Opcode::Destructure => "destructure",
            Opcode::CreateEnv => "create_env",
            Opcode::LoadName => "load_name",
            Opcode::StoreName => "store_name",
            Opcode::InitName => "init_name",
            Opcode::LoadClosure => "load_closure",
            Opcode::NewThis => "new_this",
            Opcode::TypeofName => "typeof_name",
            Opcode::JmpEq => "jmp_eq",
            Opcode::JmpNeq => "jmp_neq",
            Opcode::JmpLt => "jmp_lt",
            Opcode::JmpLtF64 => "jmp_lt_f64",
            Opcode::JmpLte => "jmp_lte",
            Opcode::JmpLteF64 => "jmp_lte_f64",
            Opcode::LoopIncJmp => "loop_inc_jmp",
            Opcode::Switch => "switch",
            Opcode::LoopHint => "loop_hint",
            Opcode::Ret => "ret",
            Opcode::RetU => "ret_u",
            Opcode::RetReg => "ret_reg",
            Opcode::TailCall => "tail_call",
            Opcode::Construct => "construct",
            Opcode::CallVar => "call_var",
            Opcode::Enter => "enter",
            Opcode::Leave => "leave",
            Opcode::Yield => "yield",
            Opcode::Await => "await",
            Opcode::Throw => "throw",
            Opcode::Try => "try",
            Opcode::EndTry => "end_try",
            Opcode::Catch => "catch",
            Opcode::Finally => "finally",
            Opcode::CallIc => "call_ic",
            Opcode::CallIcVar => "call_ic_var",
            Opcode::ProfileType => "profile_type",
            Opcode::ProfileCall => "profile_call",
            Opcode::ProfileRet => "profile_ret",
            Opcode::CheckType => "check_type",
            Opcode::CheckStruct => "check_struct",
            Opcode::CheckIc => "check_ic",
            Opcode::IcInit => "ic_init",
            Opcode::IcUpdate => "ic_update",
            Opcode::IcMiss => "ic_miss",
            Opcode::OsrEntry => "osr_entry",
            Opcode::ProfileHotLoop => "profile_hot_loop",
            Opcode::OsrExit => "osr_exit",
            Opcode::JitHint => "jit_hint",
            Opcode::SafetyCheck => "safety_check",
            Opcode::GetPropIcCall => "get_prop_ic_call",
            Opcode::IncJmpFalseLoop => "inc_jmp_false_loop",
            Opcode::LoadKAddAcc => "load_k_add_acc",
            Opcode::AddMov => "add_mov",
            Opcode::EqJmpTrue => "eq_jmp_true",
            Opcode::GetPropAccCall => "get_prop_acc_call",
            Opcode::LoadKMulAcc => "load_k_mul_acc",
            Opcode::LtJmp => "lt_jmp",
            Opcode::GetPropIcMov => "get_prop_ic_mov",
            Opcode::GetPropAddImmSetPropIc => "get_prop_add_imm_set_prop_ic",
            Opcode::AddAccImm8Mov => "add_acc_imm8_mov",
            Opcode::CallIcSuper => "call_ic_super",
            Opcode::LoadThisCall => "load_this_call",
            Opcode::EqJmpFalse => "eq_jmp_false",
            Opcode::LoadKSubAcc => "load_k_sub_acc",
            Opcode::GetLengthIcCall => "get_length_ic_call",
            Opcode::AddStrAccMov => "add_str_acc_mov",
            Opcode::IncAccJmp => "inc_acc_jmp",
            Opcode::GetPropChainAcc => "get_prop_chain_acc",
            Opcode::TestJmpTrue => "test_jmp_true",
            Opcode::LoadArgCall => "load_arg_call",
            Opcode::MulAccMov => "mul_acc_mov",
            Opcode::LteJmpLoop => "lte_jmp_loop",
            Opcode::NewObjInitProp => "new_obj_init_prop",
            Opcode::ProfileHotCall => "profile_hot_call",
            Opcode::Call2SubIAdd => "call2_sub_i_add",
            Opcode::Call1SubI => "call1_sub_i",
            Opcode::JmpLteFalse => "jmp_lte_false",
            Opcode::JmpLteFalseF64 => "jmp_lte_false_f64",

            // Assertions
            Opcode::AssertValue => "assert_value",
            Opcode::AssertOk => "assert_ok",
            Opcode::AssertEqual => "assert_equal",
            Opcode::AssertNotEqual => "assert_not_equal",
            Opcode::AssertDeepEqual => "assert_deep_equal",
            Opcode::AssertNotDeepEqual => "assert_not_deep_equal",
            Opcode::AssertStrictEqual => "assert_strict_equal",
            Opcode::AssertNotStrictEqual => "assert_not_strict_equal",
            Opcode::AssertDeepStrictEqual => "assert_deep_strict_equal",
            Opcode::AssertNotDeepStrictEqual => "assert_not_deep_strict_equal",
            Opcode::AssertThrows => "assert_throws",
            Opcode::AssertDoesNotThrow => "assert_does_not_throw",
            Opcode::AssertRejects => "assert_rejects",
            Opcode::AssertDoesNotReject => "assert_does_not_reject",
            Opcode::AssertFail => "assert_fail",

            // Fast ops
            Opcode::AddI32 => "add_i32",
            Opcode::AddF64 => "add_f64",
            Opcode::SubI32 => "sub_i32",
            Opcode::SubF64 => "sub_f64",
            Opcode::MulI32 => "mul_i32",
            Opcode::MulF64 => "mul_f64",
            Opcode::LtF64 => "lt_f64",
            Opcode::LteF64 => "lte_f64",

            // Superinstructions
            Opcode::RetIfLteI => "ret_if_lte_i",
            Opcode::AddAccReg => "add_acc_reg",
            Opcode::Call1Add => "call1_add",
            Opcode::Call2Add => "call2_add",
            Opcode::LoadKAdd => "load_k_add",
            Opcode::LoadKCmp => "load_k_cmp",
            Opcode::CmpJmp => "cmp_jmp",
            Opcode::GetPropCall => "get_prop_call",
            Opcode::CallRet => "call_ret",

            // Specialized
            Opcode::AddI32Fast => "add_i32_fast",
            Opcode::AddF64Fast => "add_f64_fast",
            Opcode::SubI32Fast => "sub_i32_fast",
            Opcode::MulI32Fast => "mul_i32_fast",
            Opcode::EqI32Fast => "eq_i32_fast",
            Opcode::LtI32Fast => "lt_i32_fast",
            Opcode::JmpI32Fast => "jmp_i32_fast",
            Opcode::GetPropMono => "get_prop_mono",
            Opcode::CallMono => "call_mono",

            // Call variants
            Opcode::Call0 => "call0",
            Opcode::Call1 => "call1",
            Opcode::Call2 => "call2",
            Opcode::Call3 => "call3",
            Opcode::CallMethod1 => "call_method1",
            Opcode::CallMethod2 => "call_method2",

            // Arithmetic superinstructions
            Opcode::LoadAdd => "load_add",
            Opcode::LoadSub => "load_sub",
            Opcode::LoadMul => "load_mul",
            Opcode::LoadInc => "load_inc",
            Opcode::LoadDec => "load_dec",

            // Comparison superinstructions
            Opcode::LoadCmpEq => "load_cmp_eq",
            Opcode::LoadCmpLt => "load_cmp_lt",
            Opcode::LoadJfalse => "load_jfalse",
            Opcode::LoadCmpEqJfalse => "load_cmp_eq_jfalse",
            Opcode::LoadCmpLtJfalse => "load_cmp_lt_jfalse",
            Opcode::LoadGetProp => "load_get_prop",
            Opcode::LoadGetPropCmpEq => "load_get_prop_cmp_eq",

            // Pareto property access
            Opcode::GetProp2Ic => "get_prop2_ic",
            Opcode::GetProp3Ic => "get_prop3_ic",
            Opcode::GetElem => "get_elem",
            Opcode::SetElem => "set_elem",
            Opcode::GetPropElem => "get_prop_elem",
            Opcode::CallMethodIc => "call_method_ic",
            Opcode::CallMethod2Ic => "call_method2_ic",

            Opcode::Reserved(n) => match n {
                61..=63 => "reserved_61_63",
                123..=127 => "reserved_123_127",
                130..=159 => "reserved_130_159",
                174..=199 => "reserved_174_199",
                225..=239 => "reserved_225_239",
                243..=255 => "reserved_243_255",
                _ => "reserved",
            },
        }
    }
    /// Convert opcode to mnemonic string (with underscores)
    pub fn opcode_to_mnemonic(&self) -> &'static str {
        match self.opcode {
            Opcode::Mov => "mov",
            Opcode::LoadK => "load_k",
            Opcode::Add => "add",
            Opcode::GetPropIc => "get_prop_ic",
            Opcode::Call => "call",
            Opcode::Jmp => "jmp",
            Opcode::LoadI => "load_i",
            Opcode::JmpTrue => "jmp_true",
            Opcode::JmpFalse => "jmp_false",
            Opcode::SetPropIc => "set_prop_ic",
            Opcode::AddAccImm8 => "add_acc_imm8",
            Opcode::IncAcc => "inc_acc",
            Opcode::LoadThis => "load_this",
            Opcode::Load0 => "load_0",
            Opcode::Load1 => "load_1",
            Opcode::Eq => "eq",
            Opcode::Lt => "lt",
            Opcode::Lte => "lte",
            Opcode::AddAcc => "add_acc",
            Opcode::SubAcc => "sub_acc",
            Opcode::MulAcc => "mul_acc",
            Opcode::DivAcc => "div_acc",
            Opcode::LoadNull => "load_null",
            Opcode::LoadTrue => "load_true",
            Opcode::LoadFalse => "load_false",
            Opcode::LoadGlobalIc => "load_global_ic",
            Opcode::SetGlobalIc => "set_global_ic",
            Opcode::Typeof => "typeof",
            Opcode::ToNum => "to_num",
            Opcode::ToStr => "to_str",
            Opcode::IsUndef => "is_undef",
            Opcode::IsNull => "is_null",
            Opcode::SubAccImm8 => "sub_acc_imm8",
            Opcode::MulAccImm8 => "mul_acc_imm8",
            Opcode::DivAccImm8 => "div_acc_imm8",
            Opcode::AddStrAcc => "add_str_acc",
            Opcode::AddI => "add_i",
            Opcode::SubI => "sub_i",
            Opcode::MulI => "mul_i",
            Opcode::DivI => "div_i",
            Opcode::ModI => "mod_i",
            Opcode::Mod => "mod",
            Opcode::Neg => "neg",
            Opcode::Inc => "inc",
            Opcode::Dec => "dec",
            Opcode::AddStr => "add_str",
            Opcode::ToPrimitive => "to_primitive",
            Opcode::GetPropAcc => "get_prop_acc",
            Opcode::SetPropAcc => "set_prop_acc",
            Opcode::GetIdxFast => "get_idx_fast",
            Opcode::SetIdxFast => "set_idx_fast",
            Opcode::LoadArg => "load_arg",
            Opcode::LoadAcc => "load_acc",
            Opcode::StrictEq => "strict_eq",
            Opcode::StrictNeq => "strict_neq",
            Opcode::BitAnd => "bit_and",
            Opcode::BitOr => "bit_or",
            Opcode::BitXor => "bit_xor",
            Opcode::BitNot => "bit_not",
            Opcode::Shl => "shl",
            Opcode::Shr => "shr",
            Opcode::Ushr => "ushr",
            Opcode::Pow => "pow",
            Opcode::LogicalAnd => "logical_and",
            Opcode::LogicalOr => "logical_or",
            Opcode::NullishCoalesce => "nullish_coalesce",
            Opcode::In => "in",
            Opcode::Instanceof => "instanceof",
            Opcode::GetLengthIc => "get_length_ic",
            Opcode::ArrayPushAcc => "array_push_acc",
            Opcode::NewObj => "new_obj",
            Opcode::NewArr => "new_arr",
            Opcode::NewFunc => "new_func",
            Opcode::NewClass => "new_class",
            Opcode::GetProp => "get_prop",
            Opcode::SetProp => "set_prop",
            Opcode::GetIdxIc => "get_idx_ic",
            Opcode::SetIdxIc => "set_idx_ic",
            Opcode::GetGlobal => "get_global",
            Opcode::SetGlobal => "set_global",
            Opcode::GetUpval => "get_upval",
            Opcode::SetUpval => "setupval",
            Opcode::GetScope => "get_scope",
            Opcode::SetScope => "set_scope",
            Opcode::ResolveScope => "resolve_scope",
            Opcode::GetSuper => "get_super",
            Opcode::SetSuper => "set_super",
            Opcode::DeleteProp => "delete_prop",
            Opcode::HasProp => "has_prop",
            Opcode::Keys => "keys",
            Opcode::ForIn => "for_in",
            Opcode::IteratorNext => "iterator_next",
            Opcode::Spread => "spread",
            Opcode::Destructure => "destructure",
            Opcode::CreateEnv => "create_env",
            Opcode::LoadName => "load_name",
            Opcode::StoreName => "store_name",
            Opcode::InitName => "init_name",
            Opcode::LoadClosure => "load_closure",
            Opcode::NewThis => "new_this",
            Opcode::TypeofName => "typeof_name",
            Opcode::JmpEq => "jmp_eq",
            Opcode::JmpNeq => "jmp_neq",
            Opcode::JmpLt => "jmp_lt",
            Opcode::JmpLtF64 => "jmp_lt_f64",
            Opcode::JmpLte => "jmp_lte",
            Opcode::JmpLteF64 => "jmp_lte_f64",
            Opcode::LoopIncJmp => "loop_inc_jmp",
            Opcode::Switch => "switch",
            Opcode::LoopHint => "loop_hint",
            Opcode::Ret => "ret",
            Opcode::RetU => "ret_u",
            Opcode::RetReg => "ret_reg",
            Opcode::TailCall => "tail_call",
            Opcode::Construct => "construct",
            Opcode::CallVar => "call_var",
            Opcode::Enter => "enter",
            Opcode::Leave => "leave",
            Opcode::Yield => "yield",
            Opcode::Await => "await",
            Opcode::Throw => "throw",
            Opcode::Try => "try",
            Opcode::EndTry => "end_try",
            Opcode::Catch => "catch",
            Opcode::Finally => "finally",
            Opcode::CallIc => "call_ic",
            Opcode::CallIcVar => "call_ic_var",
            Opcode::ProfileType => "profile_type",
            Opcode::ProfileCall => "profile_call",
            Opcode::ProfileRet => "profile_ret",
            Opcode::CheckType => "check_type",
            Opcode::CheckStruct => "check_struct",
            Opcode::CheckIc => "check_ic",
            Opcode::IcInit => "ic_init",
            Opcode::IcUpdate => "ic_update",
            Opcode::IcMiss => "ic_miss",
            Opcode::OsrEntry => "osr_entry",
            Opcode::ProfileHotLoop => "profile_hot_loop",
            Opcode::OsrExit => "osr_exit",
            Opcode::JitHint => "jit_hint",
            Opcode::SafetyCheck => "safety_check",
            Opcode::GetPropIcCall => "get_prop_ic_call",
            Opcode::IncJmpFalseLoop => "inc_jmp_false_loop",
            Opcode::LoadKAddAcc => "load_k_add_acc",
            Opcode::AddMov => "add_mov",
            Opcode::EqJmpTrue => "eq_jmp_true",
            Opcode::GetPropAccCall => "get_prop_acc_call",
            Opcode::LoadKMulAcc => "load_k_mul_acc",
            Opcode::LtJmp => "lt_jmp",
            Opcode::GetPropIcMov => "get_prop_ic_mov",
            Opcode::GetPropAddImmSetPropIc => "get_prop_add_imm_set_prop_ic",
            Opcode::AddAccImm8Mov => "add_acc_imm8_mov",
            Opcode::CallIcSuper => "call_ic_super",
            Opcode::LoadThisCall => "load_this_call",
            Opcode::EqJmpFalse => "eq_jmp_false",
            Opcode::LoadKSubAcc => "load_k_sub_acc",
            Opcode::GetLengthIcCall => "get_length_ic_call",
            Opcode::AddStrAccMov => "add_str_acc_mov",
            Opcode::IncAccJmp => "inc_acc_jmp",
            Opcode::GetPropChainAcc => "get_prop_chain_acc",
            Opcode::TestJmpTrue => "test_jmp_true",
            Opcode::LoadArgCall => "load_arg_call",
            Opcode::MulAccMov => "mul_acc_mov",
            Opcode::LteJmpLoop => "lte_jmp_loop",
            Opcode::NewObjInitProp => "new_obj_init_prop",
            Opcode::ProfileHotCall => "profile_hot_call",
            Opcode::Call2SubIAdd => "call2_sub_i_add",
            Opcode::Call1SubI => "call1_subi",
            Opcode::JmpLteFalse => "jmp_lte_false",
            Opcode::JmpLteFalseF64 => "jmp_lte_false_f64",
            Opcode::AssertValue => "assert_value",
            Opcode::AssertOk => "assert_ok",
            Opcode::AssertEqual => "assert_equal",
            Opcode::AssertNotEqual => "assert_not_equal",
            Opcode::AssertDeepEqual => "assert_deep_equal",
            Opcode::AssertNotDeepEqual => "assert_not_deep_equal",
            Opcode::AssertStrictEqual => "assert_strict_equal",
            Opcode::AssertNotStrictEqual => "assert_not_strict_equal",
            Opcode::AssertDeepStrictEqual => "assert_deep_strict_equal",
            Opcode::AssertNotDeepStrictEqual => "assert_not_deep_strict_equal",
            Opcode::AssertThrows => "assert_throws",
            Opcode::AssertDoesNotThrow => "assert_does_not_throw",
            Opcode::AssertRejects => "assert_rejects",
            Opcode::AssertDoesNotReject => "assert_does_not_reject",
            Opcode::AssertFail => "assert_fail",
            Opcode::AddI32 => "add_i32",
            Opcode::AddF64 => "add_f64",
            Opcode::SubI32 => "sub_i32",
            Opcode::SubF64 => "sub_f64",
            Opcode::MulI32 => "mul_i32",
            Opcode::MulF64 => "mul_f64",
            Opcode::LtF64 => "lt_f64",
            Opcode::LteF64 => "lte_f64",
            // Superinstructions
            Opcode::RetIfLteI => "ret_if_lte_i",
            Opcode::AddAccReg => "add_acc_reg",
            Opcode::Call1Add => "call1_add",
            Opcode::Call2Add => "call2_add",
            Opcode::LoadKAdd => "load_k_add",
            Opcode::LoadKCmp => "load_k_cmp",
            Opcode::CmpJmp => "cmp_jmp",
            Opcode::GetPropCall => "get_prop_call",
            Opcode::CallRet => "call_ret",
            // Specialized opcodes
            Opcode::AddI32Fast => "add_i32_fast",
            Opcode::AddF64Fast => "add_f64_fast",
            Opcode::SubI32Fast => "sub_i32_fast",
            Opcode::MulI32Fast => "mul_i32_fast",
            Opcode::EqI32Fast => "eq_i32_fast",
            Opcode::LtI32Fast => "lt_i32_fast",
            Opcode::JmpI32Fast => "jmp_i32_fast",
            Opcode::GetPropMono => "get_prop_mono",
            Opcode::CallMono => "call_mono",
            // Call opcodes
            Opcode::Call0 => "call0",
            Opcode::Call1 => "call1",
            Opcode::Call2 => "call2",
            Opcode::Call3 => "call3",
            Opcode::CallMethod1 => "call_method1",
            Opcode::CallMethod2 => "call_method2",
            // New arithmetic superinstructions
            Opcode::LoadAdd => "load_add",
            Opcode::LoadSub => "load_sub",
            Opcode::LoadMul => "load_mul",
            Opcode::LoadInc => "load_inc",
            Opcode::LoadDec => "load_dec",
            // New comparison superinstructions
            Opcode::LoadCmpEq => "load_cmp_eq",
            Opcode::LoadCmpLt => "load_cmp_lt",
            Opcode::LoadJfalse => "load_jfalse",
            Opcode::LoadCmpEqJfalse => "load_cmp_eq_jfalse",
            Opcode::LoadCmpLtJfalse => "load_cmp_lt_jfalse",
            Opcode::LoadGetProp => "load_get_prop",
            Opcode::LoadGetPropCmpEq => "load_get_prop_cmp_eq",
            // Pareto 80% property access superinstructions with IC
            Opcode::GetProp2Ic => "get_prop2_ic",
            Opcode::GetProp3Ic => "get_prop3_ic",
            Opcode::GetElem => "get_elem",
            Opcode::SetElem => "set_elem",
            Opcode::GetPropElem => "get_prop_elem",
            Opcode::CallMethodIc => "call_method_ic",
            Opcode::CallMethod2Ic => "call_method2_ic",
            Opcode::Reserved(n) => match n {
                61..=63 => "reserved_61_63",
                123..=127 => "reserved_123_127",
                130..=159 => "reserved_130_159",
                174..=199 => "reserved_174_199",
                225..=239 => "reserved_225_239",
                243..=255 => "reserved_243_255",
                _ => "reserved",
            },
        }
    }

    /// Format instruction as clean human-readable assembly (no byte offsets, clean mnemonics)
    pub fn to_clean_asm(&self, constants: &[JSValue]) -> String {
        let opcode_str = self.opcode_to_clean_mnemonic();

        match self.format {
            Format::ABC => {
                match self.opcode {
                    Opcode::Call
                    | Opcode::TailCall
                    | Opcode::Construct
                    | Opcode::CallIc
                    | Opcode::CallIcSuper
                    | Opcode::LoadThisCall
                    | Opcode::LoadArgCall => {
                        // A B format
                        format!("{} r{}, {}", opcode_str, self.a, self.b)
                    }
                    Opcode::Call1SubI | Opcode::Call2SubIAdd => {
                        format!("{} r{}, r{}, {}", opcode_str, self.a, self.b, self.c as i8)
                    }
                    Opcode::CallRet => {
                        format!("{} r{}, {}", opcode_str, self.a, self.b)
                    }
                    Opcode::LoadInc | Opcode::LoadDec => {
                        format!("{} r{}, r{}", opcode_str, self.a, self.b)
                    }
                    Opcode::LoadJfalse => {
                        format!("{} r{}, -> L{}", opcode_str, self.a, self.b as i8)
                    }
                    Opcode::LoadCmpEqJfalse | Opcode::LoadCmpLtJfalse => {
                        format!(
                            "{} r{}, r{}, -> L{}",
                            opcode_str, self.a, self.b, self.c as i8
                        )
                    }
                    Opcode::JmpEq
                    | Opcode::JmpNeq
                    | Opcode::JmpLt
                    | Opcode::JmpLtF64
                    | Opcode::JmpLte
                    | Opcode::JmpLteF64
                    | Opcode::JmpLteFalse
                    | Opcode::JmpLteFalseF64 => {
                        format!(
                            "{} r{}, r{}, -> L{}",
                            opcode_str, self.a, self.b, self.c as i8
                        )
                    }
                    _ => {
                        // Standard ABC format
                        format!("{} r{}, r{}, r{}", opcode_str, self.a, self.b, self.c)
                    }
                }
            }
            Format::ABx => {
                match self.opcode {
                    Opcode::LoadK | Opcode::NewFunc => {
                        // Load constant
                        let const_idx = self.bx as usize;
                        let const_val = if const_idx < constants.len() {
                            format!("const[{}]", const_idx)
                        } else {
                            "const[out of bounds]".to_string()
                        };
                        format!("{} r{}, {}", opcode_str, self.a, const_val)
                    }
                    Opcode::LoadGlobalIc
                    | Opcode::SetGlobalIc
                    | Opcode::GetGlobal
                    | Opcode::SetGlobal => {
                        format!("{} r{}, global[{}]", opcode_str, self.a, self.bx)
                    }
                    Opcode::ResolveScope
                    | Opcode::LoadName
                    | Opcode::StoreName
                    | Opcode::InitName
                    | Opcode::TypeofName => {
                        format!("{} r{}, identifier[{}]", opcode_str, self.a, self.bx)
                    }
                    Opcode::CallMethod1 => {
                        format!(
                            "{} r{}, property[{}], r{}",
                            opcode_str,
                            self.a,
                            self.bx,
                            self.a.saturating_add(1)
                        )
                    }
                    Opcode::CallMethod2 => {
                        format!(
                            "{} r{}, property[{}], r{}, r{}",
                            opcode_str,
                            self.a,
                            self.bx,
                            self.a.saturating_add(1),
                            self.a.saturating_add(2)
                        )
                    }
                    Opcode::LoadKAddAcc | Opcode::LoadKMulAcc | Opcode::LoadKSubAcc => {
                        let const_idx = self.bx as usize;
                        let const_val = if const_idx < constants.len() {
                            format!("const[{}]", const_idx)
                        } else {
                            "const[out of bounds]".to_string()
                        };
                        format!("{} {}", opcode_str, const_val)
                    }
                    _ => {
                        format!("{} r{}, {}", opcode_str, self.a, self.bx)
                    }
                }
            }
            Format::AsBx => match self.opcode {
                Opcode::Jmp => {
                    format!("{} -> L{}", opcode_str, self.sbx)
                }
                Opcode::LoadI => {
                    format!("{} r{}, {}", opcode_str, self.a, self.sbx)
                }
                Opcode::JmpTrue
                | Opcode::JmpFalse
                | Opcode::LoopIncJmp
                | Opcode::Try
                | Opcode::IncJmpFalseLoop
                | Opcode::EqJmpTrue
                | Opcode::LtJmp
                | Opcode::EqJmpFalse
                | Opcode::IncAccJmp
                | Opcode::TestJmpTrue
                | Opcode::LteJmpLoop => {
                    format!("{} r{}, -> L{}", opcode_str, self.a, self.sbx)
                }
                _ => {
                    format!("{} r{}, {}", opcode_str, self.a, self.sbx)
                }
            },
            Format::A => match self.opcode {
                Opcode::AddAccImm8
                | Opcode::SubAccImm8
                | Opcode::MulAccImm8
                | Opcode::DivAccImm8 => {
                    format!("{} {}", opcode_str, self.b as i8)
                }
                Opcode::AddAcc
                | Opcode::SubAcc
                | Opcode::MulAcc
                | Opcode::DivAcc
                | Opcode::Neg
                | Opcode::Inc
                | Opcode::Dec
                | Opcode::ToPrimitive
                | Opcode::AddStrAcc => {
                    format!("{} r{}", opcode_str, self.b)
                }
                Opcode::ArrayPushAcc
                | Opcode::GetUpval
                | Opcode::SetUpval
                | Opcode::GetScope
                | Opcode::SetScope
                | Opcode::ForIn
                | Opcode::IteratorNext
                | Opcode::Spread
                | Opcode::Destructure
                | Opcode::CreateEnv
                | Opcode::LoadClosure
                | Opcode::NewThis => {
                    format!("{} r{}", opcode_str, self.a)
                }
                Opcode::IncAcc
                | Opcode::LoadThis
                | Opcode::Load0
                | Opcode::Load1
                | Opcode::LoadNull
                | Opcode::LoadTrue
                | Opcode::LoadFalse
                | Opcode::NewObj
                | Opcode::NewArr
                | Opcode::NewClass
                | Opcode::LoopHint
                | Opcode::Ret
                | Opcode::RetU
                | Opcode::Leave
                | Opcode::Yield
                | Opcode::Await
                | Opcode::EndTry
                | Opcode::Finally
                | Opcode::ProfileRet
                | Opcode::IcMiss
                | Opcode::OsrEntry
                | Opcode::ProfileHotLoop
                | Opcode::OsrExit
                | Opcode::JitHint
                | Opcode::SafetyCheck => opcode_str.to_string(),
                Opcode::RetReg => {
                    format!("{} r{}", opcode_str, self.a)
                }
                _ => {
                    format!("{} r{}", opcode_str, self.a)
                }
            },
            Format::BC => match self.opcode {
                Opcode::GetPropAccCall | Opcode::GetLengthIcCall => {
                    format!("{} r{}, r{}", opcode_str, self.b, self.c)
                }
                _ => {
                    format!("{} r{}, r{}", opcode_str, self.b, self.c)
                }
            },
        }
    }
}

/// Disassemble bytecode into human-readable assembly
pub fn disassemble(bytecode: &[u32], constants: &[JSValue]) -> Vec<String> {
    let mut result = Vec::new();

    for (pc, &raw) in bytecode.iter().enumerate() {
        let instr = AsmInstruction::decode(pc, raw);
        result.push(instr.to_asm(constants));
    }

    result
}

/// Disassemble bytecode into clean human-readable assembly (no byte offsets, clean mnemonics)
pub fn disassemble_clean(bytecode: &[u32], constants: &[JSValue]) -> Vec<String> {
    let mut result = Vec::new();

    for (pc, &raw) in bytecode.iter().enumerate() {
        let instr = AsmInstruction::decode(pc, raw);
        result.push(instr.to_clean_asm(constants));
    }

    result
}

/// Disassemble and print bytecode to stdout
pub fn disassemble_print(bytecode: &[u32], constants: &[JSValue]) {
    let asm = disassemble(bytecode, constants);
    for line in asm {
        println!("{}", line);
    }
}

/// Disassemble compiled bytecode into human-readable assembly.
pub fn disassemble_compiled(compiled: &CompiledBytecode) -> Vec<String> {
    disassemble(&compiled.bytecode, &compiled.constants)
}

/// Disassemble compiled bytecode into clean human-readable assembly.
pub fn disassemble_compiled_clean(compiled: &CompiledBytecode) -> Vec<String> {
    disassemble_clean(&compiled.bytecode, &compiled.constants)
}
