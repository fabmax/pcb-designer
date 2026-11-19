# PCB Designer – Simulation plan

The plan for circuit simulation in the schematic editor: what other tools do, what's
built so far, the UI and the milestones to a first useful version.
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

## Built so far (S1)

The engine and everything headless are built; their design and gotchas are in
PLAN.md, "Simulation (`sim`)":

- ngspice as a process per run (`sim::Ngspice`, a `Backend`), found on `PATH`, the usual
  install places or the `ngspice` setting; the raw reader; `sim::simulate`.
- The model: `Part::sim` (`SimModel`: device, pins in SPICE order, parameters),
  `Component::sim` (`ComponentSim`: parameter overrides, `exclude`, a source's
  `Waveform`), `Schematic::simulation` (`Simulation`: analyses with their probes,
  ground, supplies, temperature, extra lines) changed by `SetSimulation`.
- The netlister (`sim::netlist`) with name maps and `SimProblem`s; SI values (`model::si`).
- `VSource` / `ISource` and a model for every Built-in part.
- `pcb-designer simulate PROJECT [--analysis NAME] [--csv FILE] [--netlist]`
  (`sim::report`); the demo has a +5V supply and an operating point.

Still to come in the areas S1 built:

- **Shared library (S8):** `ngSpice_Init` with the `SendChar` / `SendStat` /
  `SendData` / `BGThreadRunning` callbacks, `ngSpice_Circ` to load the netlist,
  `bg_run` / `bg_halt`, vectors read through `ngGet_Vec_Info` (or streamed by
  `SendData`). Loaded at runtime with `libloading`, so the app builds and runs without
  it. Gotchas to design for: one instance per process (runs are serialised; parallel
  sweeps fall back to processes), a crash in a model takes the app down with unsaved
  work (a "Run in a separate process" setting stays available for that), and
  `ControlledExit` must be handled instead of letting ngspice end the app.
- `Library::issues` for models (S5): a pin listed twice or missing, a subcircuit whose
  port count differs from its pins, model text without a `.model` / `.subckt` line
  (today the netlister reports these per component).
- Differential voltage probes (`Probe::Voltage` gets a reference, S6).
- The demo gets a "Testbench" sub-sheet with a sine source on its signal (S3, when the
  plot can show it); README documents the test bench sheet pattern then.
- Supplies suggested from power port names (S6).

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
- The CLI's `simulate` exists (S1); agents get the same summary from `sim::report`.

## KiCad import

KiCad keeps simulation data in symbol fields, which the import already carries into
`Component::properties` (everything but Reference, Value, Footprint, Description,
Datasheet). Mapping them:

- `Sim.Device` (`R`, `C`, `L`, `D`, `NPN`, `PNP`, `NMOS`, …, `V`, `I`, `SUBCKT`, `SPICE`),
  `Sim.Type` (`SIN`, `PULSE`, `PWL`, `DC`, …), `Sim.Params` (`dc=5 ampl=1 f=1k`),
  `Sim.Pins` (`1=+ 2=-` or `1=A 2=K`) → the part's `SimModel` and the component's
  `sim.source` / `sim.params`; `Sim.Enable=0` / `exclude_from_sim` → `sim.exclude`.
- `Sim.Library` + `Sim.Name`: the file is read relative to the KiCad project and the
  model or subcircuit embedded; missing files are reported.
- Legacy (KiCad 5) `Spice_Primitive`, `Spice_Model`, `Spice_Node_Sequence` likewise.
- R/C/L without fields are inferred from the reference prefix, as KiCad does.
- KiCad's `Simulation_SPICE` symbols (VDC, VSIN, VPULSE, …) map to `VSource` /
  `ISource` by `lib_id`, the waveform from `Sim.Type` / `Sim.Params`.
- The `.kicad_sch` text items with SPICE directives (`.tran …`) become analyses and
  `extra` lines.
- Verified like the netlist import: with `KICAD_CLI` set, KiCad's simulation demo
  projects netlisted by `kicad-cli sch export netlist --format spice` and by us give the
  same results through ngspice (same operating point within tolerance).

## Milestones

Each is a branch, tests, PLAN.md update and commit, as in AGENTS.md. S1 is done; S2–S4
give a usable simulator, S5–S8 make it comfortable, compatible and fast.

### S2: Running from the app

- `sim::Session` in the app and headless host: worker thread, stale tracking, stop.
- Simulation panel: status and install hint, analysis tabs and forms (transient
  length from `netlist::suggested_stop`), Run/Stop, the netlist view and log. Settings
  page: the ngspice path (`Settings::ngspice`, in `settings.json` today).
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

### S8: Shared-library backend

- `Backend` for libngspice (see "Built so far"), found like the executable (a path in
  Settings, Homebrew's `libngspice`, `libngspice.so.0`, `ngspice.dll`), the app's
  default when found.
- Live transient: traces grow while it runs (`SendData`), progress from `SendStat`,
  Stop through `bg_halt`.
- The tuner re-runs in-process (no process start), parallel sweeps still use processes.
- The same tests as the process backend, run against both when both are found.

## Later

- Bundling ngspice with release builds (macOS dylib signing, Windows DLL, Linux: the
  distribution's package).
- Source symbols that show the selected waveform (sine, square, ramp glyph), which needs
  symbols drawn from component data.
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

## Decisions taken

- Process backend first to get started; the shared library is the eventual target (S8).
- One source part per kind (`VSource`, `ISource`), waveform picked in Properties; the
  symbol doesn't follow the waveform yet (Later).
- Sources stay ordinary components; a test bench sub-sheet keeps them out of a real
  design's main sheets.
- The setup lives in `schematic.json` (`Schematic::simulation`, like the ERC settings),
  not a file of its own: it rides in the change set's `Setup` with no new plumbing.
