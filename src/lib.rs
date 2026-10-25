//! vyges-vt-swap — STA-driven threshold-voltage (Vt) swapping.
//!
//! A sibling of `vyges-resize`. Both swap a cell for another with the **same logic function
//! and footprint** and score the change on the [`vyges-sta-si`](https://github.com/vyges-tools/sta-si)
//! timer — but where resize trades *drive strength* (area/timing), vt-swap trades *threshold
//! voltage* (leakage/timing). A higher-Vt flavor of the same gate is slower but leaks far
//! less; a lower-Vt flavor is faster but leakier.
//!
//! Two objectives, the inverse of each other:
//!   - **leakage** (the headline): on a timing-met design, push every cell with positive slack
//!     to the highest-Vt (lowest-leakage) flavor that still meets timing — recover leakage for
//!     free. This is the standard post-closure ECO.
//!   - **timing**: on the critical path, drop to a faster (lower-Vt) flavor to close setup.
//!
//! Inputs/outputs are files: a `.vtswap` job (a superset of a `.sta` job) + Liberty in, a
//! resized netlist + a before/after timing **and leakage** report out. Pure std, unit-tested
//! offline. Sign-off is still the golden timer; these numbers are a fast, license-free guide.

// The crate's own name inside it: the command-line entry ([`cli_main`]) is built into the
// library too, and its `vyges_vt_swap::…` paths resolve here.
extern crate self as vyges_vt_swap;

pub mod emit;
pub mod engine;
pub mod job;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COPYRIGHT: &str = "© 2026 Vyges. All Rights Reserved.  https://vyges.com";

/// The `vyges-vt-swap` command-line entry (the binary's `main`), built into the library too so a
/// suite binary can link every engine and run the one it is invoked as.
#[cfg(feature = "suite")]
#[doc(hidden)]
#[path = "main.rs"]
pub mod cli_main;
