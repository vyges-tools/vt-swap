# vyges-vt-swap

**STA-driven threshold-voltage swapping**: a gate-level netlist in, a **resized netlist** out —
each cell moved to the Vt flavor that cuts leakage (or closes setup), scored by a real
static-timing engine.

> **Vyges open EDA tools.** Commercial-grade silicon optimization on open standards and plain
> file formats. `vyges-vt-swap` is the sibling of [`vyges-resize`](https://github.com/vyges-tools/resize):
> same idea — swap a cell for one with the **same logic and footprint**, scored by the timer —
> but it trades **threshold voltage** (leakage / speed) rather than **drive strength** (area /
> speed). A higher-Vt flavor of a gate is slower but leaks far less; a lower-Vt flavor is faster
> but leakier.

## What it does

`vyges-vt-swap` reads a netlist + Liberty + constraints and picks a **Vt flavor for each cell**:

- **leakage** (default) — on a timing-met design, push every cell with positive slack to the
  highest-Vt (lowest-leakage) flavor that *still meets timing*. Free leakage recovery — the
  standard post-closure ECO.
- **timing** — drop critical-path cells to a faster (lower-Vt) flavor to close setup, then
  recover leakage on whatever slack remains.

```text
  netlist + .lib + constraints ──[ vyges-vt-swap ]──►  resized netlist  (+ before/after timing & leakage)
```

Every candidate is scored by the [`vyges-sta-si`](https://github.com/vyges-tools/sta-si) timer — it's
**pure Rust**, so you can experiment with GPUs too via [rust-gpu](https://rust-gpu.github.io/). It picks **cells, not locations**: same footprint, so
placement/routing are untouched — run it as a post-place ECO and hand the netlist back to the flow.

## The job

A `.vtswap` file is a superset of a `.sta` timing job, plus the swap knobs:

```text
design:     top
netlist:    top.v
lib:        multi_vt.lib
spef:       top.spef               # optional — score against real interconnect (post-place)
clock:      clk 1.2
input_slew: 0.02
output_load: 0.01
group:      INV_LVT INV INV_HVT    # iso-footprint Vt flavors, FASTEST -> SLOWEST (repeatable)
group:      NAND2_LVT NAND2 NAND2_HVT
objective:  leakage                # leakage | timing  (default: leakage)
effort:     medium                 # low | medium | high
dont_touch: clk_* *scan*           # instance-name globs to leave alone
```

Group order is **fastest → slowest** (low-Vt → high-Vt) — the inverse of resize's
weakest→strongest, because here "go faster" means a lower Vt and "save leakage" means a higher
one. The legal flavors come entirely from your `.lib` (delay, transition, and
`cell_leakage_power` per flavor); nothing is foundry-confidential.

## Use it

```sh
cargo build --release            # std-only (depends on the open vyges-sta-si timer)

vyges-vt-swap run   top.vtswap -o swapped.v        # swap -> resized netlist
vyges-vt-swap run   top.vtswap --json              # before/after timing + leakage as JSON
vyges-vt-swap run   top.vtswap --fail-on-violation # exit 3 if still violating (CI gate)
vyges-vt-swap check top.vtswap                     # validate the job
vyges-vt-swap demo                                 # swap a built-in example (no files)
# common flags: -o FILE · --json · -q/--quiet · -v/--verbose · -h/--help · -V/--version
```

See [`examples/inv.vtswap`](examples/inv.vtswap) for a runnable example.

## Domain coverage

`vyges-vt-swap` operates on the **standard-cell digital abstraction** — it swaps **standard-cell
Vt flavors** (the iso-footprint low-/high-Vt families you declare) to trade leakage against
setup, each candidate scored by the digital `vyges-sta-si` timer. That makes it a **digital
optimization** engine: it applies wherever a design is built from characterized standard cells
with multi-Vt variants in the Liberty. It does **not** apply to analog / mixed-signal blocks —
they have no standard-cell Vt flavors and no Liberty-arc analogue for the timer to score. For
analog / mixed-signal physical and integrity coverage, reach for the analog-capable Vyges
engines — [`lvs`](https://github.com/vyges-tools/lvs), [`layout`](https://github.com/vyges-tools/layout),
[`em-ir`](https://github.com/vyges-tools/em-ir), [`thermal`](https://github.com/vyges-tools/thermal),
and [`extract`](https://github.com/vyges-tools/extract).

## Status & bounds

v0 swaps a netlist → netlist over the Vt families you declare; `leakage` recovers leakage while
holding setup (and not worsening hold), `timing` closes setup by going faster. It is **not** a
place-and-route tool — it decides flavors and hands physical realization back to the flow. With a
`spef:` it scores against real interconnect (post-place); without, ideal interconnect. Sign-off is
still the golden timer — `vyges-vt-swap`'s numbers are a fast, license-free guide.
