//! vyges-vt-swap CLI.
//!
//!   vyges-vt-swap run   JOB  [-o OUT] [--json] [--fail-on-violation]
//!   vyges-vt-swap check JOB
//!   vyges-vt-swap demo
//!
//! Common flags: -h/--help, -V/--version, -q/--quiet, -v/--verbose.
//! Exit codes: 0 ok · 1 runtime error · 2 usage/validation · 3 still-violating (--fail-on-violation).

use std::process::exit;

use vyges_sta_si::job::StaJob;
use vyges_vt_swap::engine::{self, VtResult};
use vyges_vt_swap::job::{parse_cfg, Objective, VtJob};

const USAGE: &str = "\
vyges-vt-swap — STA-driven threshold-voltage swapping (cut leakage / close setup, iso-footprint)

usage:
  vyges-vt-swap run   JOB  [-o OUT] [--json] [--fail-on-violation]   swap Vt -> resized netlist
  vyges-vt-swap check JOB                                            validate the job
  vyges-vt-swap demo                                                 swap a built-in example (no files)

flags:
  -o FILE              write the resized netlist to FILE (default: stdout)
  --json               emit the before/after report as JSON
  --fail-on-violation  exit 3 if the result still has negative setup slack (CI gate)
  -q, --quiet          suppress non-essential output
  -v, --verbose        extra detail on stderr
  -h, --help           show this help
  -V, --version        show version
  --bug-report         file a bug (central: vyges/community)
  --feature-request    request a feature (central)
  --sponsor            sponsor Vyges (github.com/sponsors/vyges-ip)
  --star               star this tool on GitHub ⭐
";

const BUG_URL: &str = "https://github.com/vyges/community/issues/new?template=bug_report_template.yaml";
const FEATURE_URL: &str = "https://github.com/vyges/community/issues/new?labels=enhancement";
const SPONSOR_URL: &str = "https://github.com/sponsors/vyges-ip";
const STAR_URL: &str = "https://github.com/vyges-tools/vt-swap";

fn link(label: &str, url: &str) {
    use std::io::IsTerminal;
    println!("{label}:\n  {url}");
    if std::io::stdout().is_terminal() {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(opener).arg(url).status();
    }
}

#[derive(Default)]
struct Cli {
    positionals: Vec<String>,
    out: Option<String>,
    json: bool,
    fail_on_violation: bool,
    quiet: bool,
    verbose: bool,
    help: bool,
    version: bool,
    bug_report: bool,
    feature_request: bool,
    sponsor: bool,
    star: bool,
}

fn parse_cli(args: &[String]) -> Cli {
    let mut c = Cli::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                c.out = args.get(i + 1).cloned();
                i += 1;
            }
            "--json" => c.json = true,
            "--fail-on-violation" => c.fail_on_violation = true,
            "-q" | "--quiet" => c.quiet = true,
            "-v" | "--verbose" => c.verbose = true,
            "-h" | "--help" => c.help = true,
            "-V" | "--version" => c.version = true,
            "--bug-report" => c.bug_report = true,
            "--feature-request" => c.feature_request = true,
            "--sponsor" => c.sponsor = true,
            "--star" => c.star = true,
            other => c.positionals.push(other.to_string()),
        }
        i += 1;
    }
    c
}

// leakage Watts -> microwatts for a friendlier report.
fn uw(w: f64) -> f64 {
    w * 1e6
}

fn render_report(r: &VtResult) -> String {
    let met = |w: f64| if w >= 0.0 { "MET" } else { "VIOLATED" };
    let saved = if r.before_leak_w > 0.0 {
        100.0 * (r.before_leak_w - r.after_leak_w) / r.before_leak_w
    } else {
        0.0
    };
    let mut s = String::new();
    s.push_str("vyges-vt-swap — threshold-voltage swap\n");
    s.push_str(&format!(
        "  mode:    {}\n",
        if r.eco { "post-place ECO (SPEF interconnect)" } else { "pre-place (ideal interconnect)" }
    ));
    s.push_str(&format!("  setup:   WNS {:.4} -> {:.4} ns [{}]\n", r.before_wns, r.after_wns, met(r.after_wns)));
    s.push_str(&format!(
        "  leakage: {:.4} -> {:.4} uW  ({saved:.1}% saved)\n",
        uw(r.before_leak_w),
        uw(r.after_leak_w)
    ));
    s.push_str(&format!("  changed: {} cell(s)\n", r.changed.len()));
    for (inst, old, new) in &r.changed {
        s.push_str(&format!("    {inst}: {old} -> {new}\n"));
    }
    s
}

fn report_json(r: &VtResult) -> String {
    let changes: Vec<String> = r
        .changed
        .iter()
        .map(|(i, o, n)| format!("{{\"inst\":\"{i}\",\"old\":\"{o}\",\"new\":\"{n}\"}}"))
        .collect();
    format!(
        "{{\"eco\":{},\"before_wns\":{},\"after_wns\":{},\"before_leak_w\":{},\"after_leak_w\":{},\"changed\":[{}]}}",
        r.eco, r.before_wns, r.after_wns, r.before_leak_w, r.after_leak_w, changes.join(",")
    )
}

// ---- built-in demo: two inverters on a met clock, with a fast (leaky) and slow (low-leak)
// Vt flavor of the inverter. The leakage objective swaps both to the high-Vt flavor. ----
const DEMO_NL: &str = "module top ( a, y ); input a; output y; wire n1;\n\
                       INV u1 ( .A(a), .Y(n1) ); INV u2 ( .A(n1), .Y(y) ); endmodule";
const DEMO_LIB: &str = r#"
library (d) {
  leakage_power_unit : 1nW;
  cell (INV) {
    cell_leakage_power : 4.0;
    pin (A) { direction : input; capacitance : 0.0015; }
    pin (Y) { direction : output;
      timing () { related_pin : "A";
        cell_rise (t)       { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.05, 0.12", "0.07, 0.16" ); }
        cell_fall (t)       { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.045, 0.11", "0.065, 0.15" ); }
        rise_transition (t) { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.02, 0.05", "0.025, 0.06" ); }
        fall_transition (t) { index_1 ("0.01, 0.08"); index_2 ("0.001, 0.01"); values ( "0.02, 0.045", "0.025, 0.055" ); } } }
  }
  cell (INV_HVT) {
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
const DEMO_JOB: &str = "design: demo\nnetlist: x\nlib: x\nclock: clk 1.0\ninput_slew: 0.02\noutput_load: 0.005\n";

fn run_demo() -> Result<VtResult, String> {
    let sta = StaJob::parse(DEMO_JOB, "").map_err(|e| e.to_string())?;
    // fastest -> slowest (low-Vt -> high-Vt)
    let cfg = parse_cfg("group: INV INV_HVT\nobjective: leakage\neffort: medium\n")?;
    engine::run_inputs(DEMO_NL, DEMO_LIB, &sta, &cfg)
}

fn write_netlist(text: &str, out: &Option<String>, quiet: bool) {
    match out {
        Some(path) => match std::fs::write(path, text) {
            Ok(_) => {
                if !quiet {
                    eprintln!("wrote {path}");
                }
            }
            Err(e) => {
                eprintln!("error: {path}: {e}");
                exit(1);
            }
        },
        None => print!("{text}"),
    }
}

fn finish(r: VtResult, cli: &Cli) {
    if cli.json {
        println!("{}", report_json(&r));
        if cli.out.is_some() {
            write_netlist(&r.netlist_v, &cli.out, cli.quiet);
        }
    } else {
        write_netlist(&r.netlist_v, &cli.out, cli.quiet);
        if !cli.quiet {
            eprint!("{}", render_report(&r));
        }
    }
    if cli.fail_on_violation && r.after_wns < 0.0 {
        exit(3);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = parse_cli(&args);

    if cli.bug_report {
        return link("Report a bug (central — vyges/community)", BUG_URL);
    }
    if cli.feature_request {
        return link("Request a feature (central — vyges/community)", FEATURE_URL);
    }
    if cli.sponsor {
        return link("Sponsor Vyges", SPONSOR_URL);
    }
    if cli.star {
        return link("Star vyges-vt-swap on GitHub ⭐", STAR_URL);
    }
    if cli.version {
        println!("vyges-vt-swap {} ({})", vyges_vt_swap::VERSION, env!("VYGES_GIT_SHA"));
        println!("{}", vyges_vt_swap::COPYRIGHT);
        return;
    }
    let cmd = cli.positionals.first().cloned().unwrap_or_default();
    if cli.help || cmd.is_empty() {
        print!("{USAGE}");
        exit(if cmd.is_empty() && !cli.help { 2 } else { 0 });
    }

    match cmd.as_str() {
        "demo" => match run_demo() {
            Ok(r) => finish(r, &cli),
            Err(e) => {
                eprintln!("error: {e}");
                exit(1);
            }
        },
        "check" => {
            let Some(path) = cli.positionals.get(1) else {
                eprintln!("usage: vyges-vt-swap check JOB");
                exit(2);
            };
            match VtJob::load(path) {
                Ok(j) => {
                    let obj = match j.cfg.objective {
                        Objective::Leakage => "leakage",
                        Objective::Timing => "timing",
                    };
                    println!(
                        "OK  design={} groups={} objective={obj} effort={} dont_touch={}",
                        j.sta.design,
                        j.cfg.groups.len(),
                        j.cfg.effort,
                        j.cfg.dont_touch.len()
                    );
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(2);
                }
            }
        }
        "run" => {
            let Some(path) = cli.positionals.get(1) else {
                eprintln!("usage: vyges-vt-swap run JOB [-o OUT]");
                exit(2);
            };
            let job = match VtJob::load(path) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(2);
                }
            };
            if cli.verbose {
                eprintln!("vt-swapping {} ({} group(s), effort {})", job.sta.design, job.cfg.groups.len(), job.cfg.effort);
            }
            match engine::run(&job) {
                Ok(r) => finish(r, &cli),
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(1);
                }
            }
        }
        other => {
            eprintln!("vyges-vt-swap: unknown command {other:?}\n");
            print!("{USAGE}");
            exit(2);
        }
    }
}
