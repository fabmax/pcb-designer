# PCB Designer – Simulation plan

A proposal for circuit simulation in the schematic editor: what other tools do, the
engine, the data model, the netlist, the UI and the milestones to a first useful version.
Once a milestone is built, its design moves into PLAN.md (a "Simulation (`sim`)" section)
and this file keeps only what is still ahead.

## Goals

- Simulate the schematic as drawn: no separate "simulation schematic". The same
  components, nets and labels, plus simulation-only parts (sources) that never reach the
  board.
- Out of the box for the everyday parts: resistors, capacitors, inductors, diodes, LEDs,
  transistors, MOSFETs and op amps from the Built-in library simulate without setup.
- Analyses: operating point, DC sweep, AC (frequency response) and transient.
- Results where the circuit is: click a wire to plot its voltage, click a pin to plot its
  current, operating point voltages written on the schematic.
- A plot view that copes with long transients (a million points), with Bode plots (log
  frequency, dB and phase), cursors and readouts.
- Agents can set up, run and read simulations over MCP like everything else.

Not goals for the first version: mixed-signal / digital simulation, board parasitics,
Monte Carlo, noise, RF (S-parameters).

## What other tools do

| Tool | Engine | How it works |
|---|---|---|
| KiCad (6+, much improved in 7–9) | ngspice, as a shared library | Model per symbol in fields (`Sim.Device`, `Sim.Type`, `Sim.Pins`, `Sim.Params`, `Sim.Library`, `Sim.Name`), inferred for R/C/L from the reference and value. Sources from the `Simulation_SPICE` symbol library (VDC, VSIN, VPULSE, VPWL, IDC, …), off the board through "exclude from board". Simulator window: one tab per analysis, signals list, cursors, measurements (min, max, RMS, frequency, …), a **probe** tool (click a wire for a voltage, a pin for a current) and a **tuner** (sliders for R/C/L values that re-run the simulation). Operating point values are written onto the schematic. SPICE directives can be text on the schematic. |
| LTspice | Own engine | Directives (`.tran 1m`, `.step param`) as text on the schematic. Click a wire to plot its voltage, a part to plot its current, Alt-click for power. Plot panes stacked in one window; cursors with Δ. `.step` sweeps overlay traces. Very fast to use, but SPICE syntax is visible everywhere. |
| Qucs-S | ngspice / Xyce / Qucsator | Simulations are blocks placed on the schematic (a "Transient simulation" box with fields); results in a separate data display page. |
| Altium, EasyEDA, Fusion | (X)SPICE-based | Like KiCad: a model per part, sources from a simulation library, an analysis setup dialog, a waveform window, probes placed on nets. |

What works well and is worth copying: models attached to parts with a sensible default
(R/C/L from the value); simulation sources as normal-looking parts that stay off the
board; click-to-probe; operating point on the schematic; the tuner; an analysis setup
that's a form rather than SPICE text, with an escape hatch for raw SPICE.

What to avoid: SPICE syntax as the main interface (LTspice), simulations as schematic
objects (Qucs-S: clutters the drawing and the print), and a separate modal window (KiCad's
simulator is its own frame; ours should be a tab that can sit beside the schematic).

## Engine: ngspice

**Decision: ngspice**, run as a separate process in batch mode, behind a small
`Backend` trait so the shared library can be added later.

- ngspice is the de-facto open SPICE: KiCad, Qucs-S and EasyEDA use it, so vendor
  models and KiCad projects work with it. BSD licence (core), so it can be shipped
  with the app later. Its device models (BSIM, VDMOS, diode, BJT Gummel–Poon, B-sources,
  XSPICE) took decades; writing our own engine (modified nodal analysis plus Newton
  iteration) would be quick for R/C/L but would never reach that.
- Alternatives considered: Xyce (GPL, heavy to build, no shared library on all
  platforms), writing our own (see above), a WASM ngspice (no gain for a desktop app).
- **Process, not shared library, first:**
  - `ngspice -b -r out.raw circuit.cir`: write the netlist, read the raw file back. The
    raw format is simple (text header, then binary doubles, point-major; complex for AC).
  - Crash isolation: a model that crashes ngspice (it happens) doesn't take the app and
    the unsaved design with it. Stop = kill the process.
  - No global state: libngspice is one instance per process (KiCad works around that), a
    process each lets the tuner or a sweep run several at once.
  - Headless and testable: tests, MCP and the CLI run it the same way.
  - Cost: process start (~20–50 ms) and no live streaming of a transient while it runs.
    Both matter only for the tuner and for long runs; the shared library
    (`ngSpice_Init`, `SendData` callbacks, `bg_run`, `bg_halt`, loaded with `libloading`
    so the app still builds and runs without it) is a later backend behind the same trait.
- **Finding ngspice:** a path in Settings, else `NGSPICE` in the environment, else
  `ngspice` on `PATH` and the usual install places (Homebrew `/opt/homebrew/bin`,
  `/usr/local/bin`, `C:\Program Files\Spice64\bin`). Not found: the Simulation panel says
  so and how to install it (`brew install ngspice`, `apt install ngspice`, the Windows
  zip), with a "Choose…" button. Bundling ngspice in release builds is in **Later**.
- **Tests:** netlist generation is pure and tested always. Tests that run ngspice are
  skipped when it isn't found (like the KiCad corpus tests with `KICAD_CLI`), so
  `cargo test --workspace` stays green on a fresh machine.

## Data model (`model`)

### Simulation models on parts

A part says how it simulates. It's part content (copied into the project with the part,
covered by `content_hash`, so "Update from library" brings model changes along).

```rust
pub struct Part {
    …
    /// How the part simulates; `None`: it has no model (left out with a warning).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sim: Option<SimModel>,
}

pub struct SimModel {
    pub device: SimDevice,
    /// SPICE node order → the part's pins (unit, pin). Empty: by pin number 1, 2, 3 …
    pub pins: Vec<SimPin>,
    /// Parameters, by name, as typed ("Is" = "2.52n"). Component `sim_params` override.
    pub params: BTreeMap<String, String>,
}

pub enum SimDevice {
    /// Not simulated, silently (connectors, test points, mounting holes, logos).
    Ignore,
    /// R, C, L: the value is the component's value ("10k", "100n").
    Resistor, Capacitor, Inductor,
    /// A primitive with a `.model` card (D, NPN, PNP, NJF, PJF, NMOS, PMOS, VDMOS): the
    /// model text is stored here (`.model 1N4148 D(Is=2.52n N=1.752 …)`).
    Primitive { kind: PrimitiveKind, model: String },
    /// A subcircuit: the `.subckt` text, its ports mapped by `pins`.
    Subckt { text: String },
    /// Independent sources; the waveform is per component (`Component::source`).
    VoltageSource, CurrentSource,
    /// Escape hatch: one SPICE line with `${REF}`, `${VALUE}` and `${N1}`… for the nodes.
    Spice { template: String },
}
```

- Model text is **embedded**, not referenced by file: a project stays self-contained,
  like parts are copied into it. `.include` of external model files is in **Later**
  (KiCad's `Sim.Library`); the import below embeds what it can read.
- Multi-unit parts (LM358: two op amps and a power unit) get one model for the whole
  component, its pins listed by `(unit, pin)`. Unplaced units' pins are open (see
  "Floating nodes" below).
- `Library::issues` gains checks: a pin listed twice, a pin that doesn't exist, a
  subcircuit whose port count differs from `pins`, model text that doesn't parse as a
  `.model` / `.subckt` card.

### Per component

```rust
pub struct Component {
    …
    /// Parameter overrides of the part's model ("BF" = "200").
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sim_params: BTreeMap<String, String>,
    /// Left out of the simulation (KiCad's "Exclude from simulation").
    #[serde(default, skip_serializing_if = "is_false")]
    pub sim_exclude: bool,
    /// Waveform of a source part (`SimDevice::VoltageSource` / `CurrentSource`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

pub struct Source {
    pub waveform: Waveform,
    /// Small-signal AC magnitude and phase (degrees) for AC analysis; 0: not an AC input.
    pub ac: f64,
    pub ac_phase: f64,
}

pub enum Waveform {
    Dc { value: f64 },
    Sine { offset: f64, amplitude: f64, frequency: f64, delay: f64, damping: f64, phase: f64 },
    Pulse { low: f64, high: f64, delay: f64, rise: f64, fall: f64, width: f64, period: f64 },
    Pwl { points: Vec<(f64, f64)>, repeat: bool },
    Exp { v1: f64, v2: f64, delay1: f64, tau1: f64, delay2: f64, tau2: f64 },
}
```

- Waveform numbers are plain SI `f64` (volts, amperes, seconds, hertz), not strings: the
  forms parse "1k", "10us" and show them back in engineering notation. R/C/L values stay
  the component's `value` string, which is already what users type and what the BOM
  shows.
- A source's displayed value is generated from its waveform ("SIN 1 V 1 kHz", "DC 5 V",
  "PULSE 0–3.3 V 1 kHz"), written into `value` by the command that changes the waveform,
  so the existing `${VALUE}` field shows it without special drawing.
- All of these change through `Update` of the component, so undo, MCP and the
  Properties panel need nothing new.

### Simulation setup

A new project file `simulation.json` (`Project::simulation: Simulation`), changed by a
`SetSimulation { simulation }` command that replaces it whole (like `SetErcSettings`; it
rides in the change set's `Setup`, views ignore it). Everything in it is undoable and
visible to agents. Zoom, cursors and plot scroll positions are workspace state, not
project content.

```rust
pub struct Simulation {
    /// The analyses, in tab order; the active one is workspace state.
    pub analyses: Vec<Analysis>,
    /// Net that is SPICE node 0; `None`: automatic (see Netlist).
    pub ground: Option<String>,
    /// Supplies for power nets without a source: "+5V" → 5 V DC (see Netlist).
    pub supplies: Vec<Supply>,
    pub temperature: f64,
    /// Extra SPICE lines (`.param`, `.options`, `.include` later), as typed.
    pub extra: String,
}

pub struct Analysis {
    pub id: Id,
    pub name: String,               // "Transient", "Bode of filter"
    pub kind: AnalysisKind,
    /// What its plot shows.
    pub probes: Vec<Probe>,
}

pub enum AnalysisKind {
    Op,
    Dc { source: Id /* component */, start: f64, stop: f64, step: f64 },
    Ac { start: f64, stop: f64, points_per_decade: u32 },
    Tran { stop: f64, step: Option<f64>, start: f64, max_step: Option<f64> },
}

pub enum Probe {
    /// Voltage of the net of this wire, label or pin (to ground), or between two.
    Voltage { at: ProbePoint, reference: Option<ProbePoint>, net: String },
    /// Current into a pin of a component.
    Current { symbol: Id, pin: Id },
    /// Free expression in ngspice syntax ("v(/OUT)*i(V1)"); later.
    Expr { expr: String },
}
```

- A voltage probe keeps the object it was placed on *and* the net name at that time:
  it resolves by object first (survives net renames), then by name (survives wire
  merges, where `rubber_band` may drop the clicked wire's id). Neither found: the trace
  is listed as missing, never removed silently. Same idea as zones binding to net names,
  but with the object as the primary key.

### Simulation-only parts

New Built-in parts in a **Simulation** category, each with only a "No footprint"
variant, so they are off the board by the existing rule (no sync changes, no BOM entry,
no board warnings):

- `V_DC`, `V_Sine`, `V_Pulse`, `V_PWL` and `I_DC`, `I_Sine`, `I_Pulse`: a source circle
  with `+`/arrow and a glyph for the waveform (bar, sine, square, ramp). They differ only
  in the default `Source`; the Properties panel can switch the waveform (the glyph then
  stays the part's — acceptable, and the value text shows the truth).
- No ground part: ground is a GND power port as usual (see Netlist).

Pins are `Passive`, so the ERC matrix needs no changes: a source on a net with power
ports or an input is fine, and the power-input rule is already satisfied by the power
port or power flag the real design has there.

For a design meant for a real board, a **test bench sheet** keeps stimuli out of the way:
a sub-sheet "Testbench" with the sources, connected by global labels. That works with
today's hierarchy; the plan only documents it (and the demo shows it). An "exclude this
sheet from printing" flag can come with printing.

### Built-in library models

Every generated and hand-made Built-in part gets a `sim`:

| Part | Model |
|---|---|
| Resistor, Capacitor, Capacitor_Polarized, Inductor | `Resistor` / `Capacitor` / `Inductor`, value from the component |
| FerriteBead, Fuse | `Resistor` with a small default value (0.1 Ω) — documented as a simplification |
| Diode | `.model` of a 1N4148; Schottky: BAT54; Zener: breakdown from the value ("5V1" → `BV=5.1`) |
| LED | a generic red LED model (≈1.8 V at 10 mA); value colour names pick a few presets later |
| NPN / PNP | 2N3904 / 2N3906 Gummel–Poon cards |
| NMOS / PMOS | 2N7002 / BSS84 (VDMOS level) |
| OpAmp, LM358 | a generic behavioural op amp subcircuit (open-loop gain, GBW, slew rate, output clamped to the rails by a B-source); LM358-like numbers for the LM358 |
| LM7805, AMS1117-3.3, AP2112K-3.3 | behavioural regulator subcircuits (output = min(set voltage, Vin − dropout), quiescent current) |
| 74HC00 | behavioural analog NAND (B-sources with `tanh` edges) until digital simulation exists |
| Conn_*, TestPoint, SW_Push, MountingHole_*, Fiducial | `Ignore` (a switch model is in **Later**) |
| Crystal | `Subckt` with the Butterworth–Van Dyke equivalent (generic values) |

Vendor macromodels are not bundled: their licences usually forbid redistribution.
Users paste them into the part's model text (Part editor, below).

## Netlist (`sim` crate)

A new crate `sim` (`model` → `editor-core` → **`sim`** → `view`), GUI-independent:
value parsing, the netlister, the backend, raw-file reading and the plot maths, all
unit-tested headless.

`sim::netlist::build(project, &Connectivity, &Simulation, analysis) -> Netlist`, with
`Netlist { text, nodes: net name ↔ SPICE node, devices: component ↔ SPICE name,
problems: Vec<Problem> }`. Problems use the existing `Problem` shape (objects, position),
so Messages can zoom to them.

- **Nodes:** one per net with pins, from `Connectivity::netlist()`. Ground net → `0`.
  Others get sanitised, readable names (`/OUT` → `_OUT`, `Net-(R1-1)` → `NET_R1_1`,
  unique), kept in the map so results go back to nets and objects.
- **Ground:** the net in `Simulation::ground`, else the net of power ports drawn as
  ground (`PowerStyle` Ground / SignalGround, also by name: `GND`, `AGND`, `0`). Several
  ground-styled nets (GND, AGND, PGND) are joined to `0` with a note. None at all is an
  error ("No ground: place a GND power port").
- **Devices:** per component (not per symbol: units are merged), skipping `sim_exclude`
  and `Ignore`. The SPICE name is the device letter, then the refdes unless it already
  starts with that letter (`R1`, `D1`, `Q1`, `XU1`; a resistor named `FB1` → `RFB1`).
  The source parts use the prefixes `V` and `I`, so theirs stay `V1`, `I1`.
  Parts without `sim` are left out with a warning listing them once ("Not simulated, no
  model: J1, Y1"), never a hard error, so a real design can simulate its analog corner.
- **Values:** `sim::units::parse` reads `10k`, `4k7`, `2R2`, `100n`, `10µ`/`10u`,
  `1M` (mega, as users mean it), `1m`, `10 kΩ`, `100nF 50V` (first number wins, units
  and extras ignored). The netlist always writes plain exponents (`1e7`), never SPICE
  suffixes. **Gotcha:** SPICE reads `M` as milli and ignores unknown letters (`1F` is
  1 femto); writing exponents avoids both traps. A value that doesn't parse is an error
  on that component.
- **Floating nodes:** pins left open (unplaced units, unwired pins, flagged no-connect
  pins) each get a node of their own, and `.options rshunt=1e12` ties every node to
  ground through 1 TΩ so ngspice doesn't stop on a singular matrix. A net without any DC
  path to ground otherwise (a capacitor-only node) works the same way.
- **Supplies:** power nets (named by a power port) with no source on them and a
  `Supply` entry get a `V` source to ground. The Simulation panel suggests entries from
  the names it can parse (`+5V`, `+3V3`, `3.3V`, `-12V`, `VCC` → asks), so a real board
  design simulates without drawing supply sources. A power net with neither is a warning
  ("+5V has no supply").
- **Currents:** ngspice saves only voltage-source branch currents by default. Current
  probes on primitives use `@R1[i]`-style vectors (`.save` lists only what the probes
  need, plus all node voltages); a current into a subcircuit pin gets a 0 V sense source
  in series (`VSENSE_U1_3`). Saving everything (`.options savecurrents`) is slower on big
  circuits and is not the default.
- **Analysis line** from `AnalysisKind` (`.op`, `.dc V1 0 5 0.01`, `.ac dec 100 1 10meg`,
  `.tran 1u 10m 0 1u`), plus `.temp`, then `Simulation::extra` as typed. Transient
  defaults: stop time 5 periods of the slowest periodic source, step stop / 1000.
- The netlist is shown read-only in the Simulation panel ("Show SPICE netlist") and given
  to agents; it's the first thing to look at when a simulation misbehaves.

## Running (`sim`)

- `Backend::run(netlist) -> Result<Results, SimError>` on a worker thread. The app keeps
  one `sim::Session` (in `PcbApp`, and in the headless host for MCP): the running job, the
  last results per analysis, the log.
- Each run records the document revision it started from. When the schematic changes
  afterwards, results are marked **stale** (dimmed traces, a "Changed since the run –
  Run again" hint), never thrown away. Operating point may re-run automatically on change
  (a setting, on by default when the last run took under 100 ms).
- `Results`: the scale vector (time, frequency or the swept value), one vector per node
  voltage and probed current (real, or complex for AC), mapped back to nets and pins
  through the `Netlist` maps. ngspice's stdout/stderr go into the log; its errors and
  warnings ("singular matrix", "timestep too small", unknown model parameters) become
  Messages entries, with the component if the line names one.
- Stop kills the process. A run is never started twice for the same analysis: Run while
  running restarts.

## UI

### The flow

1. Place a source from the Parts panel's **Simulation** category (or the Simulation tool
   bar), wire it in. Ground is a GND power port, as in any schematic.
2. Open the **Simulation** panel. It has one row per analysis (tabs like Messages:
   OP, Transient, AC, DC sweep; "+" adds another), and a form for the active one with
   defaults that work (transient length from the sources, AC 1 Hz–10 MHz).
3. Press **Run** (button and a key; Stop while running). The Simulation tab opens next
   to the schematic (split side by side if the schematic tab is alone).
4. With no probes yet, the plot shows the named nets (labelled, at most 8) so there is
   always something to look at, and a hint: "Click a wire to plot its voltage".
5. With the **Probe** tool (active right after a run, Esc leaves it), clicking a wire or
   label adds its voltage to the active analysis's plot, clicking a pin adds the current
   into it. Probed wires are drawn in their trace's colour on the schematic. Clicking a
   probed wire again removes it.
6. Change a value, run again; old results dim until the new ones arrive.

### Simulation panel (dock)

A new dock panel (`Panel::Simulation`, default in the right bottom group), sections:

- **Status line:** engine found / not found (with the install hint), running (with a
  spinner and Stop), last run time, stale.
- **Analysis:** tabs per analysis, the form of the active one. Fields take SI input
  ("10m", "1meg", "100k", spaces and units allowed) and show back engineering notation.
  The DC sweep picks its source from the sources in the design.
- **Signals:** the probes of the analysis with their colour swatch, visibility check box
  and a remove button; their value at cursor A and B and the difference. Missing probes
  (net gone) shown greyed with the reason.
- **Supplies and ground** (collapsed): the ground net, the supply table with suggestions.
- **Advanced** (collapsed): temperature, extra SPICE lines, "Show SPICE netlist", the log.

The panel's fields are edits of `Simulation` through `SetSimulation`, grouped into one
undo step per field edit like the other setup panels.

### Simulation tab (waveform viewer)

A tab type next to `Tab::Gerber`: `Tab::Simulation`, one tab showing the active analysis
(switching the analysis in the panel switches the plot). Read-only, like the Gerber tab:
it has its own focus (`Focus::Simulation`) for its keys.

- **Plot widget** (our own, in egui's painter, logic in `sim::plot`):
  - Linear or log x (log by default for AC), up to two y axes: volts left, amperes right
    (transient, DC), or magnitude in dB left and phase in degrees right (AC; the phase
    traces dashed in the same colour). Nice tick values with SI prefixes on the labels.
  - Wheel zooms x around the pointer, Shift+wheel y, drag pans, box zoom with a modifier,
    double-click fits. Zoom is workspace state.
  - Hover shows a readout of every trace at the pointer's x.
  - Cursors A and B (click / Shift+click to place, drag to move), snapping to samples;
    Δx, 1/Δx and Δy per trace in the Signals list.
  - Legend in a corner (click toggles a trace).
  - Colours: a categorical palette per theme in `view/src/style.rs`; a trace keeps its
    colour while it exists (by probe), so the schematic highlight matches.
  - Long transients: per trace and zoom, min/max per pixel column (`sim::plot::decimate`,
    binary search for the visible range), so a million points draw as ~2 × width
    vertices. Cached until zoom, size or results change.
- **Operating point:** no plot. The tab shows a sortable table of node voltages and
  device currents (and power per device), and the schematic shows the values.
- Export: CSV of the visible traces, and the plot as PNG (also for MCP screenshots).
- `egui_plot` was considered: it lacks log axes, a second y axis and per-column
  decimation, and its look doesn't follow the app's palettes. A small widget of our own
  is simpler than working around it.

### On the schematic (`view`)

- **Operating point annotations:** after an OP run, each net gets one value tag ("2.48 V")
  at its first label, else near the middle of its longest wire, and each component a
  current tag near its first pin, drawn as an overlay layer in the scene like the ERC
  markers (`View::set_sim_annotations`, palette colours, hidden from the View menu,
  dimmed when stale). Sizes follow the zoom like texts.
- **Probe tool** (`view::sim_tools`): highlights the net under the pointer (as hover
  does), shows a pin hit when over a pin end, and on click returns a `Probe` for the app
  to add. Differential probes (click a second net with a modifier) are in S6.
- **Probe highlights:** the wires of a probed net drawn in the trace colour, slightly
  thicker (an overlay like cross-probing, not a restyle of the scene).
- Cross-probing from the plot: hovering a trace in the legend highlights its net.

### Part editor and Properties

- **Part form** gets a **Simulation** section: the device type (No model / Not simulated /
  Resistor / Capacitor / Inductor / Diode / BJT / MOSFET / … / Subcircuit / Voltage /
  Current source / SPICE line), the model text (a monospace multi-line field; pasting a
  `.subckt` fills the port list), and a **pin table** like the pad mapping: SPICE port
  (from the `.subckt` line, or the primitive's fixed ports: D = anode, cathode; Q =
  C, B, E; M = D, G, S) → part pin, filled by number on first use. Parameters as a
  name/value grid.
- **Properties** of a component: "Simulation" rows: excluded (check box), parameter
  overrides (rows for the model's parameters, empty = the part's), and for sources the
  waveform picker with its fields and the AC magnitude/phase.

### Keys, tool bar, menus

- Actions (`keymap.rs`): Run simulation, Stop simulation, Probe tool, Show simulation
  panel; defaults picked so they don't clash (Run on F5 / ⌘R are candidates).
- Schematic tool bar: a Simulation group (Probe, Run) shown when the Simulation panel or
  tab is open, so schematics that never simulate aren't cluttered. Icons: Tabler
  `player-play`, `player-stop`, `wave-sine`, plus a probe icon of our own.
- Menu: a "Simulation" menu (Run, Stop, Probe, Analyses…, Show netlist).

### Messages

A **Simulation** category in the Messages panel: netlist problems (no ground, unparsable
values, missing models, missing probe targets) with their objects, and ngspice's errors
and warnings. Its badge counts like the others; it is empty until the first run.

## MCP and CLI

- `simulate { analysis: name | inline AnalysisKind, probes?: [...] }`: runs (synchronously
  for the agent, the app's UI stays live) and returns the operating point as a table, or
  for sweeps the vectors decimated to ≤ 200 points per signal plus min, max, average and
  final value, so answers stay small. `points: [x…]` samples signals at given x values.
- `get_spice_netlist { analysis }`: the netlist text and the node map.
- Setup through `execute` with `SetSimulation` and component `Update`s (sources, model
  overrides) — no special tools. Part models through `edit_library` like other part
  content.
- `screenshot` gains `simulation` to render the plot (offscreen, same widget logic).
- CLI: `pcb-designer simulate PROJECT [--analysis NAME] [--csv OUT]` for scripts and CI.

## KiCad import

KiCad keeps simulation data in symbol fields, which the import already carries into
`Component::properties` (everything but Reference, Value, Footprint, Description,
Datasheet). Mapping them:

- `Sim.Device` (`R`, `C`, `L`, `D`, `NPN`, `PNP`, `NMOS`, …, `V`, `I`, `SUBCKT`, `SPICE`),
  `Sim.Type` (`SIN`, `PULSE`, `PWL`, `DC`, …), `Sim.Params` (`dc=5 ampl=1 f=1k`),
  `Sim.Pins` (`1=+ 2=-` or `1=A 2=K`) → the part's `SimModel` and the component's
  `source` / `sim_params`; `Sim.Enable=0` / `exclude_from_sim` → `sim_exclude`.
- `Sim.Library` + `Sim.Name`: the file is read relative to the KiCad project and the
  model or subcircuit embedded; missing files are reported.
- Legacy (KiCad 5) `Spice_Primitive`, `Spice_Model`, `Spice_Node_Sequence` likewise.
- R/C/L without fields are inferred from the reference prefix, as KiCad does.
- KiCad's `Simulation_SPICE` symbols map to our source parts by `lib_id`.
- The `.kicad_sch` text items with SPICE directives (`.tran …`) become analyses and
  `extra` lines.
- Verified like the netlist import: with `KICAD_CLI` set, KiCad's simulation demo
  projects netlisted by `kicad-cli sch export netlist --format spice` and by us give the
  same results through ngspice (same operating point within tolerance).

## Milestones

Each is a branch, tests, PLAN.md update and commit, as in AGENTS.md. The first four give
a usable simulator; S5–S7 make it comfortable and compatible.

### S1: Engine and netlist (headless)

- `model`: `SimModel`, `Component::{sim_params, sim_exclude, source}`, `Simulation`
  (`simulation.json`, `Project::simulation`), `SetSimulation`.
- `sim` crate: `units` (parse and format SI values), `netlist::build` (ground, nodes,
  R/C/L/D/Q/M/X/V/I, supplies, rshunt, `.save`), `ngspice` backend (find, spawn, kill,
  log), `raw` reader (binary and ASCII, real and complex, several plots per file),
  `Results`.
- Built-in library: the Simulation parts and `sim` for every part in the table above
  (`gen_builtin` for generated ones, `edit`/`take` for the hand-made ones).
- CLI `simulate`.
- Tests: value parsing (incl. `M`/`meg`, `4k7`, units), golden netlists of small
  circuits, the raw reader against fixtures checked in (so it's tested without
  ngspice), and with ngspice: RC low-pass (−3 dB at 1/(2πRC) in AC, 63 % at τ in
  transient), a diode's forward drop, an op amp non-inverting amplifier's gain, a
  regulator's output. The demo project gets a small analog corner that simulates.

### S2: Running from the app

- `sim::Session` in the app and headless host: worker thread, stale tracking, stop.
- Simulation panel: status and install hint, analysis tabs and forms, Run/Stop, the
  netlist view and log. Settings: ngspice path.
- Messages "Simulation" category.
- Keymap actions and the Simulation menu.
- Screenshots: the panel in both themes, not-found state.

### S3: Waveform viewer

- `Tab::Simulation` and `Focus::Simulation`; opening it on run (side by side when
  possible).
- `sim::plot`: ticks (linear, log, SI labels), decimation, cursor interpolation,
  transforms (dB, phase unwrap). Tested headless.
- The plot widget: axes, two y axes, traces, legend, hover readout, zoom/pan/fit,
  cursors A/B, stale dimming; OP table. Trace palettes in `style.rs`.
- Signals list in the panel. CSV export.
- `--show simulation` for screenshots; a `--script` step to run a simulation.

### S4: Probing and results on the schematic

- Probe tool with net and pin hover, add/remove probes (`SetSimulation`), default probes
  for runs without any.
- Probe highlights in trace colours; legend hover highlights the net.
- Operating point annotations; View menu toggle.
- Probe resolution (object, then name), missing-probe display.

### S5: Models in the editors, and MCP

- Part form Simulation section (device, model text, port table, parameters), library
  issues for models.
- Properties: excluded, overrides, source waveform editor.
- MCP `simulate`, `get_spice_netlist`, simulation screenshots; README tool list.

### S6: Comfort

- **Tuner:** a slider row per chosen R/C/L/source parameter (range from the value ±
  a decade, E-series snapping as an option), re-running on release (on drag for runs
  under ~100 ms). Results of the tuned run replace the plot; "Apply" writes the value
  to the component (one undo step).
- Differential voltage probes; device power (`V × I`) probes; expressions.
- Measurements per trace: min, max, average, RMS, peak-to-peak, frequency, rise/fall
  time, −3 dB point, phase margin (AC loop gain).
- Parameter sweeps: one parameter stepped (a component value or `.param`), overlaid
  traces with a legend per step (ngspice `.control` loop or one run per step in
  parallel processes).
- Supplies suggestions from power port names.

### S7: KiCad simulation import

- The mapping in "KiCad import" above, the `Simulation_SPICE` parts, directives, and the
  `kicad-cli` comparison test.

## Later

- ngspice shared-library backend (live transient streaming, faster tuner), loaded at
  runtime; bundling ngspice with release builds (macOS dylib signing, Windows DLL,
  Linux: the distribution's package).
- `.include` / `.lib` of external model files (relative to the project), a project
  model library browser, vendor model import (pasting a whole `.lib` with many
  `.subckt`s and picking one).
- Noise, FFT of a transient, pole-zero, sensitivity, distortion, Monte Carlo with part
  tolerances, temperature sweeps, worst case.
- Digital and mixed-signal (XSPICE digital models for 74HC parts, A/D bridges), logic
  traces in the viewer.
- Switch models (voltage/time-controlled) for `SW_Push` and relays, transformer and
  coupled inductors (`K` lines), transmission lines.
- Several plot panes per analysis (stacked, shared x), saved plot layouts.
- Probes and op annotations on sub-sheet instances when sheets are reused (not
  supported by the schematic today: every sheet is one instance).
- Board-aware simulation (track resistance, parasitic capacitance from the layout).
- KiCad export of simulation fields (with the KiCad project export).

## Open questions

- **Process vs. shared library first.** The plan starts with the process for robustness
  and testability; if live streaming of long transients matters early, the shared
  library could come with S3 instead.
- **One source part with a waveform picker vs. one part per waveform.** The plan has
  separate parts (clear in the Parts panel, a matching glyph) with a switchable
  waveform. A single `V_Source` whose glyph follows the waveform would need symbols that
  change with component data, which nothing does today.
- **Sources in the drawing.** Sources are ordinary components on the sheet (KiCad,
  LTspice). If they should stay out of a real design's printed schematic, the test bench
  sub-sheet covers it; a per-sheet "simulation only" flag would make it explicit.
