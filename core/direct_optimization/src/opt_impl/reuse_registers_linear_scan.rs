use super::*;

pub fn reuse_registers_linear_scan(
    bytecode: Vec<u32>,
    constants: Vec<JSValue>,
) -> (Vec<u32>, Vec<JSValue>) {
    let mut insts = decode_program(&bytecode);
    let Some(liveness) = analyze_liveness(&insts, &constants) else {
        return (bytecode, constants);
    };

    let mut map = [0u8; REG_COUNT];
    for (index, slot) in map.iter_mut().enumerate() {
        *slot = index as u8;
    }

    let mut available_regs = Vec::new();
    for reg in 1..ACC {
        if !liveness.pinned[reg as usize] {
            available_regs.push(reg);
        }
    }

    let mut intervals = liveness
        .intervals
        .iter()
        .flatten()
        .copied()
        .filter(|interval| {
            interval.reg != 0 && interval.reg != ACC && !liveness.pinned[interval.reg as usize]
        })
        .collect::<Vec<_>>();
    intervals.sort_by_key(|interval| (interval.start, interval.end, interval.reg));

    let mut active = Vec::<(usize, u8)>::new();

    for interval in intervals {
        active.retain(|(end, _)| *end >= interval.start);

        let mut occupied = [false; REG_COUNT];
        for &(_, reg) in &active {
            occupied[map[reg as usize] as usize] = true;
        }

        let physical = available_regs
            .iter()
            .copied()
            .find(|&candidate| !occupied[candidate as usize])
            .unwrap_or(interval.reg);

        map[interval.reg as usize] = physical;
        active.push((interval.end, interval.reg));
    }

    if map
        .iter()
        .enumerate()
        .all(|(index, &reg)| reg == index as u8)
    {
        return (bytecode, constants);
    }

    for inst in &mut insts {
        rewrite_instruction_registers(inst, &map);
    }

    let (rewritten, constants) = encode_program(&insts, constants);
    if rewritten == bytecode {
        return (bytecode, constants);
    }
    (rewritten, constants)
}
