//! The Vt-swap loop, driving the `vyges-sta-si` [`Timer`]. Two passes, both checkpoint-and-keep
//! speculative moves over the timer (the logic of the design never changes):
//!   - **timing**: rank the critical path, try a *faster* (lower-Vt) flavor on its instances,
//!     keep the best non-hold-breaking improvement — until setup is met or no move helps.
//!   - **leakage**: for every positive-slack instance, try the *next slower* (higher-Vt, lower-
//!     leakage) flavor and keep it if timing stays met and hold is not worsened.

use std::collections::{HashMap, HashSet};

use vyges_sta_si::job::StaJob;
use vyges_sta_si::liberty::Lib;
use vyges_sta_si::netlist;
use vyges_sta_si::spef::Spef;
use vyges_sta_si::sta::Timer;

use crate::emit;
use crate::job::{glob_match, Objective, VtCfg, VtJob};

/// Outcome of a Vt-swap run.
#[derive(Debug, Clone)]
pub struct VtResult {
    pub before_wns: f64,
    pub after_wns: f64,
    /// total cell leakage (Watts) before / after — the figure of merit for the leakage objective.
    pub before_leak_w: f64,
    pub after_leak_w: f64,
    /// `(instance, old_cell, new_cell)` for every committed swap, in order.
    pub changed: Vec<(String, String, String)>,
    /// The resized netlist as structural Verilog.
    pub netlist_v: String,
    /// Whether timing was scored against real interconnect parasitics (a SPEF was supplied).
    pub eco: bool,
}

/// cell name → leakage (Watts), from the Liberty `cell_leakage_power`.
fn leakage_map(lib: &Lib) -> HashMap<String, f64> {
    lib.cells
        .iter()
        .map(|(n, c)| (n.clone(), c.leakage_w))
        .collect()
}

/// Total leakage of the current netlist (cells absent from the map contribute 0).
fn total_leakage(timer: &Timer, leak: &HashMap<String, f64>) -> f64 {
    timer
        .netlist()
        .insts
        .iter()
        .map(|i| leak.get(&i.cell).copied().unwrap_or(0.0))
        .sum()
}

/// Build a [`Timer`] (+ leakage map) from a job, reading the netlist / Liberty / SPEF it names.
fn build(sta: &StaJob) -> Result<(Timer, HashMap<String, f64>, bool), String> {
    let nl = netlist::load(&sta.resolve(&sta.netlist)).map_err(|e| e.to_string())?;
    let mut lib = Lib::default();
    for l in &sta.libs {
        let one = Lib::load(&sta.resolve(l)).map_err(|e| e.to_string())?;
        lib.cells.extend(one.cells);
    }
    if lib.cells.is_empty() {
        return Err("no cells in any .lib".into());
    }
    let spef = match &sta.spef {
        Some(p) => Some(Spef::load(&sta.resolve(p)).map_err(|e| e.to_string())?),
        None => None,
    };
    let leak = leakage_map(&lib);
    let timer = Timer::build(&nl, &lib, sta, spef.as_ref()).map_err(|e| e.to_string())?;
    Ok((timer, leak, spef.is_some()))
}

/// Run a Vt-swap job loaded from disk.
pub fn run(job: &VtJob) -> Result<VtResult, String> {
    let (timer, leak, eco) = build(&job.sta)?;
    optimize(timer, &job.cfg, &leak, eco)
}

/// Run on already-parsed inputs (the `demo` path; ideal interconnect, no SPEF).
pub fn run_inputs(
    nl_text: &str,
    lib_text: &str,
    sta: &StaJob,
    cfg: &VtCfg,
) -> Result<VtResult, String> {
    let nl = netlist::parse(nl_text).map_err(|e| e.to_string())?;
    let lib = Lib::parse(lib_text).map_err(|e| e.to_string())?;
    let leak = leakage_map(&lib);
    let timer = Timer::build(&nl, &lib, sta, None).map_err(|e| e.to_string())?;
    optimize(timer, cfg, &leak, false)
}

/// The optimizer over a built [`Timer`].
pub fn optimize(
    mut timer: Timer,
    cfg: &VtCfg,
    leak: &HashMap<String, f64>,
    eco: bool,
) -> Result<VtResult, String> {
    // cell name -> (group index, position fastest..slowest)
    let mut pos: HashMap<String, (usize, usize)> = HashMap::new();
    for (gi, g) in cfg.groups.iter().enumerate() {
        for (pi, c) in g.iter().enumerate() {
            pos.insert(c.clone(), (gi, pi));
        }
    }
    let before_wns = timer.wns();
    let before_leak_w = total_leakage(&timer, leak);
    let mut changed: Vec<(String, String, String)> = Vec::new();

    let dont = |inst: &str| cfg.dont_touch.iter().any(|p| glob_match(p, inst));
    let cell_of = |t: &Timer, inst: &str| -> Option<String> {
        t.netlist()
            .insts
            .iter()
            .find(|i| i.name == inst)
            .map(|i| i.cell.clone())
    };

    // ---- timing: drop the critical path to faster (lower-Vt) flavors until setup is met ----
    if cfg.objective == Objective::Timing {
        for _ in 0..cfg.effort {
            if timer.wns() >= 0.0 {
                break;
            }
            let (base_wns, base_whs) = (timer.wns(), timer.whs());
            let mut cands: Vec<(String, String, String)> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            for node in timer.worst_path() {
                let Some((inst, _)) = node.label.split_once('/') else {
                    continue;
                };
                if !seen.insert(inst.to_string()) || dont(inst) {
                    continue;
                }
                let Some(cur) = cell_of(&timer, inst) else {
                    continue;
                };
                if let Some(&(gi, pi)) = pos.get(&cur) {
                    if pi > 0 {
                        // a faster flavor is the *previous* entry (groups are fast → slow).
                        cands.push((inst.to_string(), cur, cfg.groups[gi][pi - 1].clone()));
                    }
                }
            }
            if cands.is_empty() {
                break;
            }
            let mut best: Option<(f64, (String, String, String))> = None;
            for cand in &cands {
                let ck = timer.checkpoint();
                timer.resize(&cand.0, &cand.2);
                timer.update().map_err(|e| e.to_string())?;
                let (w, h) = (timer.wns(), timer.whs());
                timer.restore(ck);
                if w > base_wns + 1e-12 && h >= base_whs - 1e-9 {
                    let better = best.as_ref().map(|(bw, _)| w > *bw).unwrap_or(true);
                    if better {
                        best = Some((w, cand.clone()));
                    }
                }
            }
            match best {
                Some((_, (inst, old, new))) => {
                    timer.resize(&inst, &new);
                    timer.update().map_err(|e| e.to_string())?;
                    changed.push((inst, old, new));
                }
                None => break,
            }
        }
    }

    // ---- leakage recovery: push slack cells to the next slower (higher-Vt) flavor while
    // timing stays met (one greedy pass; runs for both objectives) ----
    if timer.wns() >= 0.0 {
        let insts: Vec<(String, String)> = timer
            .netlist()
            .insts
            .iter()
            .map(|i| (i.name.clone(), i.cell.clone()))
            .collect();
        for (inst, cur) in insts {
            if dont(&inst) {
                continue;
            }
            let Some(&(gi, pi)) = pos.get(&cur) else {
                continue;
            };
            if pi + 1 >= cfg.groups[gi].len() {
                continue; // already the slowest / highest-Vt
            }
            let slower = cfg.groups[gi][pi + 1].clone();
            // only worth it if the slower flavor actually leaks less.
            if leak.get(&slower).copied().unwrap_or(f64::INFINITY)
                > leak.get(&cur).copied().unwrap_or(0.0)
            {
                continue;
            }
            let base_whs = timer.whs();
            let ck = timer.checkpoint();
            timer.resize(&inst, &slower);
            timer.update().map_err(|e| e.to_string())?;
            if timer.wns() >= 0.0 && timer.whs() >= base_whs - 1e-9 {
                changed.push((inst, cur, slower)); // accept: timing still met, lower leakage
            } else {
                timer.restore(ck); // reject: would violate setup (slower cell)
            }
        }
    }

    Ok(VtResult {
        before_wns,
        after_wns: timer.wns(),
        before_leak_w,
        after_leak_w: total_leakage(&timer, leak),
        changed,
        netlist_v: emit::to_verilog(timer.netlist()),
        eco,
    })
}
