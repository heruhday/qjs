use codegen::Opcode;
use value::{JSValue, to_f64};

pub const ACC_REG: u8 = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedInst {
    pub raw: u32,
    pub pc: usize,
    pub opcode: Opcode,
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub bx: u16,
    pub sbx: i16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedSwitchCase {
    pub value: JSValue,
    pub target_pc: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedSwitchTable {
    pub case_count: usize,
    pub default_target_pc: usize,
    pub cases: Vec<DecodedSwitchCase>,
}

pub fn decode_word(pc: usize, raw: u32) -> DecodedInst {
    let opcode = Opcode::from((raw & 0xff) as u8);
    let a = ((raw >> 8) & 0xff) as u8;
    let b = ((raw >> 16) & 0xff) as u8;
    let c = ((raw >> 24) & 0xff) as u8;
    let bx = ((raw >> 16) & 0xffff) as u16;
    let sbx = bx as i16;

    DecodedInst {
        raw,
        pc,
        opcode,
        a,
        b,
        c,
        bx,
        sbx,
    }
}

pub fn decode_branch_target(inst: &DecodedInst) -> Option<usize> {
    let base = inst.pc as isize + 1;
    let offset = match inst.opcode {
        Opcode::Jmp
        | Opcode::JmpTrue
        | Opcode::JmpFalse
        | Opcode::LoopIncJmp
        | Opcode::Try
        | Opcode::IncJmpFalseLoop
        | Opcode::IncAccJmp
        | Opcode::TestJmpTrue => inst.sbx as isize,
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
        | Opcode::LoadCmpLtJfalse => inst.c as i8 as isize,
        Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
            inst.a as i8 as isize
        }
        Opcode::LoadJfalse => inst.b as i8 as isize,
        _ => return None,
    };

    usize::try_from(base + offset).ok()
}

pub fn decode_switch_table(
    constants: &[JSValue],
    table_index: usize,
    pc: usize,
) -> Option<DecodedSwitchTable> {
    let case_count = constants
        .get(table_index)
        .and_then(|value| to_f64(*value))? as usize;
    let default_offset = constants
        .get(table_index + 1)
        .and_then(|value| to_f64(*value))? as i16;
    let default_target_pc = usize::try_from((pc + 1) as isize + default_offset as isize).ok()?;

    let mut cases = Vec::with_capacity(case_count);
    for case_index in 0..case_count {
        let value_index = table_index + 2 + case_index * 2;
        let offset_index = value_index + 1;
        let value = *constants.get(value_index)?;
        let offset = constants
            .get(offset_index)
            .and_then(|entry| to_f64(*entry))? as i16;
        let target_pc = usize::try_from((pc + 1) as isize + offset as isize).ok()?;
        cases.push(DecodedSwitchCase { value, target_pc });
    }

    Some(DecodedSwitchTable {
        case_count,
        default_target_pc,
        cases,
    })
}
