//! End-to-end vt-swap tests — fully offline (the sta-si timer is pure std, no simulator).

use vyges_sta_si::job::StaJob;
use vyges_sta_si::netlist;
use vyges_vt_swap::engine::run_inputs;
use vyges_vt_swap::job::{glob_match, parse_cfg};

// A fast/leaky LVT inverter and a slow/low-leak HVT inverter, same footprint.
const LIB: &str = r#"
library (d) {
  leakage_power_unit : 1nW;
  cell (INV) {                 // low-Vt: fast, leaky
    cell_leakage_power : 4.0;
    pin (A) { direction : input; capacitance : 0.0015; }
    pin (Y) { direction : output;
      timing () { related_pin : "A";
        cell_rise (t)       { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.05, 0.12", "0.07, 0.16" ); }
        cell_fall (t)       { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.045, 0.11", "0.065, 0.15" ); }
        rise_transition (t) { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.02, 0.05", "0.025, 0.06" ); }
        fall_transition (t) { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.02, 0.045", "0.025, 0.055" ); } } }
  }
  cell (INV_HVT) {             // high-Vt: slow, low-leak
    cell_leakage_power : 1.0;
    pin (A) { direction : input; capacitance : 0.0015; }
    pin (Y) { direction : output;
      timing () { related_pin : "A";
        cell_rise (t)       { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.10, 0.24", "0.14, 0.32" ); }
        cell_fall (t)       { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.09, 0.22", "0.13, 0.30" ); }
        rise_transition (t) { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.04, 0.10", "0.05, 0.12" ); }
        fall_transition (t) { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.04, 0.09", "0.05, 0.11" ); } } }
  }
}
"#;

const NL: &str = "module top ( a, y ); input a; output y; wire n1;\n\
                  INV u1 ( .A(a), .Y(n1) ); INV u2 ( .A(n1), .Y(y) ); endmodule";

fn sta(period: f64) -> StaJob {
    StaJob::parse(
        &format!("design: t\nnetlist: x\nlib: x\nclock: clk {period}\ninput_slew: 0.02\noutput_load: 0.005\n"),
        "",
    )
    .unwrap()
}

#[test]
fn leakage_objective_swaps_to_high_vt_when_slack_allows() {
    // loose clock -> both inverters have slack -> both move to the low-leak HVT flavor.
    let cfg = parse_cfg("group: INV INV_HVT\nobjective: leakage\neffort: high\n").unwrap();
    let r = run_inputs(NL, LIB, &sta(1.0), &cfg).unwrap();
    assert!(r.after_wns >= 0.0, "timing must stay met");
    assert!(
        r.after_leak_w < r.before_leak_w,
        "leakage should drop: {} -> {}",
        r.before_leak_w,
        r.after_leak_w
    );
    assert_eq!(r.changed.len(), 2, "both inverters move to HVT");
    assert!(r
        .changed
        .iter()
        .all(|(_, old, new)| old == "INV" && new == "INV_HVT"));
    // round-trips and carries the high-Vt cells.
    let nl2 = netlist::parse(&r.netlist_v).unwrap();
    assert!(nl2.insts.iter().all(|i| i.cell == "INV_HVT"));
}

#[test]
fn leakage_objective_keeps_critical_cells_low_vt() {
    // a tight clock: there is no slack to give away, so swapping to slow HVT would violate
    // -> the leakage pass must leave the cells on the fast flavor.
    let cfg = parse_cfg("group: INV INV_HVT\nobjective: leakage\neffort: high\n").unwrap();
    let r = run_inputs(NL, LIB, &sta(0.20), &cfg).unwrap();
    assert!(r.before_wns >= 0.0, "fast cells meet 0.20 ns");
    assert!(
        r.changed.is_empty(),
        "no slack -> keep low-Vt, no leakage swap"
    );
    assert_eq!(r.after_leak_w, r.before_leak_w);
}

#[test]
fn timing_objective_speeds_up_with_low_vt() {
    // start on the slow HVT flavor at a period it cannot meet; timing mode drops to fast INV.
    let nl_hvt = "module top ( a, y ); input a; output y; wire n1;\n\
                  INV_HVT u1 ( .A(a), .Y(n1) ); INV_HVT u2 ( .A(n1), .Y(y) ); endmodule";
    let cfg = parse_cfg("group: INV INV_HVT\nobjective: timing\neffort: high\n").unwrap();
    let r = run_inputs(nl_hvt, LIB, &sta(0.28), &cfg).unwrap();
    assert!(
        r.before_wns < 0.0,
        "HVT should violate at 0.28 ns: {}",
        r.before_wns
    );
    assert!(
        r.after_wns > r.before_wns,
        "low-Vt swap should improve setup"
    );
    assert!(
        r.changed.iter().any(|(_, _, new)| new == "INV"),
        "expected a swap to fast INV"
    );
}

#[test]
fn dont_touch_blocks_the_swap() {
    let cfg =
        parse_cfg("group: INV INV_HVT\nobjective: leakage\neffort: high\ndont_touch: u1 u2\n")
            .unwrap();
    let r = run_inputs(NL, LIB, &sta(1.0), &cfg).unwrap();
    assert!(r.changed.is_empty(), "every instance is dont_touch");
}

#[test]
fn globs() {
    assert!(glob_match("clk_*", "clk_a"));
    assert!(glob_match("*_reg", "x_reg"));
    assert!(glob_match("*scan*", "u_scan_0"));
    assert!(!glob_match("u1", "u2"));
}
