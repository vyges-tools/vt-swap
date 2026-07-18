//! The `.vtswap` job: the timing setup (reused from `vyges-sta-si`'s job parser) plus the
//! vt-swap knobs. A `.vtswap` file is a superset of a `.sta` file — it carries the same
//! `design`/`netlist`/`lib`/`clock`/`spef`/… keys (read by [`StaJob`]) and adds:
//!
//! ```text
//! group:      INV_LVT INV INV_HVT    # iso-footprint Vt flavors, FASTEST -> SLOWEST (repeatable)
//! group:      NAND2_LVT NAND2 NAND2_HVT
//! objective:  leakage                # leakage | timing  (default: leakage)
//! effort:     medium                 # low | medium | high  (iteration budget)
//! dont_touch: clk_* *scan*           # instance-name globs to leave alone
//! ```
//!
//! Note the group order is **fastest → slowest** (low-Vt → high-Vt): the inverse intuition
//! from resize's weakest→strongest, because here "go faster" means a *lower* Vt and "save
//! leakage" means a *higher* Vt.

use vyges_sta_si::job::StaJob;

/// What to optimize for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Objective {
    /// Recover leakage: push positive-slack cells to the highest-Vt flavor that still meets
    /// timing. The standard post-closure use.
    Leakage,
    /// Close setup WNS: drop critical-path cells to a faster (lower-Vt) flavor, then recover
    /// leakage on the remaining slack.
    Timing,
}

/// The vt-swap-specific configuration (everything beyond the timing setup).
#[derive(Debug, Clone)]
pub struct VtCfg {
    /// Iso-footprint Vt families, each ordered fastest (low-Vt) → slowest (high-Vt).
    pub groups: Vec<Vec<String>>,
    pub objective: Objective,
    /// Iteration budget (derived from `effort:`).
    pub effort: usize,
    /// Instance-name globs (supporting a leading/trailing `*`) to never modify.
    pub dont_touch: Vec<String>,
}

/// A loaded vt-swap job: the timing job + the vt-swap config.
#[derive(Debug, Clone)]
pub struct VtJob {
    pub sta: StaJob,
    pub cfg: VtCfg,
}

impl VtJob {
    pub fn load(path: &str) -> Result<VtJob, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let sta = StaJob::load(path).map_err(|e| e.to_string())?;
        let cfg = parse_cfg(&text)?;
        Ok(VtJob { sta, cfg })
    }
}

/// Parse the vt-swap-only keys out of the job text.
pub fn parse_cfg(text: &str) -> Result<VtCfg, String> {
    let mut groups = Vec::new();
    let mut objective = Objective::Leakage;
    let mut effort_word = "medium".to_string();
    let mut dont_touch = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_lowercase(), v.trim());
        match k.as_str() {
            "group" => {
                let g: Vec<String> = v.split_whitespace().map(str::to_string).collect();
                if g.len() >= 2 {
                    groups.push(g);
                }
            }
            "objective" => {
                objective = match v.to_lowercase().as_str() {
                    "leakage" => Objective::Leakage,
                    "timing" => Objective::Timing,
                    other => {
                        return Err(format!("objective must be leakage|timing, got {other:?}"))
                    }
                };
            }
            "effort" => effort_word = v.to_lowercase(),
            "dont_touch" => {
                dont_touch.extend(
                    v.split([',', ' '])
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                );
            }
            _ => {}
        }
    }
    let effort = match effort_word.as_str() {
        "low" => 20,
        "medium" => 100,
        "high" => 500,
        other => return Err(format!("effort must be low|medium|high, got {other:?}")),
    };
    Ok(VtCfg {
        groups,
        objective,
        effort,
        dont_touch,
    })
}

/// A tiny glob matcher: supports a single leading and/or trailing `*` (e.g. `clk_*`,
/// `*scan*`, `*_reg`). Exact match otherwise.
pub fn glob_match(pat: &str, s: &str) -> bool {
    match (pat.strip_prefix('*'), pat.strip_suffix('*')) {
        (Some(_), Some(_)) => s.contains(pat.trim_matches('*')),
        (Some(suf), None) => s.ends_with(suf),
        (None, Some(pre)) => s.starts_with(pre),
        (None, None) => s == pat,
    }
}
