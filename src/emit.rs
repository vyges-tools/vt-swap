//! Emit a structural Verilog netlist (the resized design) — round-trips through the
//! `vyges-sta-si` / `vyges-loom` netlist parser.

use std::collections::HashSet;
use std::fmt::Write;

use vyges_sta_si::netlist::Netlist;

/// Render `nl` as structural Verilog: module header, `input`/`output`/`wire` declarations,
/// then one instance line per cell (`<cell> <inst> ( .<pin>(<net>), … );`).
pub fn to_verilog(nl: &Netlist) -> String {
    let mut s = String::new();
    let ports: Vec<&str> = nl
        .inputs
        .iter()
        .chain(&nl.outputs)
        .map(String::as_str)
        .collect();
    let _ = writeln!(s, "module {} ( {} );", nl.module, ports.join(", "));
    if !nl.inputs.is_empty() {
        let _ = writeln!(s, "  input {};", nl.inputs.join(", "));
    }
    if !nl.outputs.is_empty() {
        let _ = writeln!(s, "  output {};", nl.outputs.join(", "));
    }

    // wires = nets referenced by instance connections that aren't primary ports, in
    // first-seen order (deduped).
    let portset: HashSet<&str> = ports.iter().copied().collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut wires: Vec<&str> = Vec::new();
    for inst in &nl.insts {
        for (_, net) in &inst.conns {
            let net = net.as_str();
            if !portset.contains(net) && seen.insert(net) {
                wires.push(net);
            }
        }
    }
    if !wires.is_empty() {
        let _ = writeln!(s, "  wire {};", wires.join(", "));
    }

    for inst in &nl.insts {
        let conns: Vec<String> = inst
            .conns
            .iter()
            .map(|(p, n)| format!(".{p}({n})"))
            .collect();
        let _ = writeln!(s, "  {} {} ( {} );", inst.cell, inst.name, conns.join(", "));
    }
    let _ = writeln!(s, "endmodule");
    s
}
