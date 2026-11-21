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

## Built so far (S1–S8)

The engine and everything headless are built; their design and gotchas are in
PLAN.md, "Simulation (`sim`)":

- ngspice as a process per run (`sim::Ngspice`, a `Backend`), found on `PATH`, the usual
  install places or the `ngspice` setting; the raw reader; `sim::simulate`.
- S8: ngspice as a shared library (`sim::Libngspice`), loaded at run time with
  `libloading`, the default when found (`sim::Engine`; Settings → Simulation: "Shared
  library when found" or "Separate process only", both paths, what's in use). Single
  runs and the tuner use it, sweeps run in parallel processes. A running analysis
  streams its points (`sim::Live`): traces grow over the whole x range, "Running…
  42 %" and a progress bar; Stop keeps what it has ("Stopped at 3.2 ms").
- The model: `Part::sim` (`SimModel`: device, pins in SPICE order, parameters),
  `Component::sim` (`ComponentSim`: parameter overrides, `exclude`, a source's
  `Waveform`), `Schematic::simulation` (`Simulation`: analyses with their probes,
  ground, supplies, temperature, extra lines) changed by `SetSimulation`.
- The netlister (`sim::netlist`) with name maps and `SimProblem`s; SI values (`model::si`).
- `VSource` / `ISource` and a model for every Built-in part.
- `pcb-designer simulate PROJECT [--analysis NAME] [--csv FILE] [--netlist]`
  (`sim::report`); the demo has a +5V supply and an operating point.
- S2: `sim::Session` (worker thread, results per analysis, stale by netlist text, the
  last attempt's problems and log); the Simulation panel (⌘2) with analysis tabs and
  forms, Run (⌘R) / Stop (⌘.), a result table, ground, supplies, temperature, extra
  lines, the netlist and log window; Messages' Simulation tab; the Simulation menu;
  the Settings page with the ngspice path; the running time in the status bar.
- S3: the Simulation tab (`Tab::Simulation`): the plot of the panel's analysis with
  log/linear x, dB and phase for AC, volts and amperes on two axes, wheel/box zoom,
  pan, fit, cursors A and B with a readout table under the plot (which is the legend:
  click a row to hide a trace), hover values, sample dots when zoomed in, stale
  dimming, CSV export; an operating point shows a sortable table. Check boxes in the
  panel's result table choose the plotted signals. The demo has a test bench sheet.
  `--show simulation` and a `run [ANALYSIS]` script step.
- S4: the Probe tool (O; a Simulation group with Probe and Run on the schematic tool
  bar while the panel or tab is open): a wire, label or junction probes its net's
  voltage, a pin's body the current into it, a probed one is removed; probes go to the
  panel's analysis if it's a sweep, else the first sweep. Probed nets' wires and pins
  drawn in their trace colours; an operating point's voltages (a tag per net and sheet)
  and the currents of two-pin parts (an arrow and the magnitude) on the schematic, a
  tooltip with pin voltages, currents and power for bigger parts (View → Simulation
  values). Operating points measure every pin's current; the table has `I(R1.1)` and
  `P(R1)` rows. Probes resolve by object, then name; missing ones show why. Hovering
  a readout or operating point row highlights it on the schematic; right-click removes
  a probe. A `zoom F` script step.
- S5: the part form's Simulation section (device combo, monospace model text with a
  parse line, the port → pin table with Auto-map, parameters, model problems in
  `Library::issues`); a component's Simulation rows in Properties (Exclude, parameter
  overrides with the model's values as hints, a source's waveform with its fields, PWL
  text and a preview plot; Exclude for a multi-selection). MCP `simulate` (through the
  app's session; answers values, decimated points, statistics and samples at
  `points`; unprobed currents measured for that run only), `get_spice_netlist`, and
  `screenshot` target `simulation` (the plot drawn as a render scene offscreen). The
  headless host has its own session. A `scroll DY` script step.
- S6a: differential voltage probes (drag from net to net), power probes (Alt-click a
  part), a third y axis, the Measure table (`sim::measure`, also in `simulate`'s
  replies with `from` / `to`), supply suggestions, reordering analysis tabs, a `hold`
  script step.
- S6b: the tuner (`sim::tune`): rows in the setup, slider values as temporary
  overrides of the runs' netlists, live re-runs, Apply and Reset, "Tune R1" on a
  part; `simulate` takes `values`.
- S6c: parameter sweeps (`Analysis::step`): a netlist per value, run in parallel
  processes; traces per signal and step on one x axis, an operating point's column per
  step; `simulate` answers per step.

Still to come in the areas S1 built:

- Plot: overlays of earlier runs, PNG export from the GUI (MCP's screenshot renders it
  offscreen already), the zoom saved in the workspace.
- Schematic values: placement that avoids other texts (tags can overlap a part's
  reference or another tag), values at the cursor of a sweep (scrubbing), currents of
  bigger parts as tags.
- The values only show while the Simulation panel or tab is open; a pinned "always"
  might be wanted.
- Expressions of signals (`V(OUT)/V(IN)`, `abs(I(R1.1))`); complex power for AC.
- Per-unit models for multi-unit parts (a dual op amp as one subcircuit instance per
  unit): today a model covers every unit's pins at once.
- The model text field has no syntax highlighting; `simulate` takes no inline analysis
  (agents add one with `SetSimulation` first).

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
- Stop kills the process (or halts the library's run) and keeps the points so far. A
  run is never started twice for the same analysis: Run while running restarts.

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

Built in S4 (`view::sim_overlay`, design in PLAN.md): the Probe tool's targets, and one
overlay item per sheet with the probes in trace colours, the operating point's values
and the hovered target. Differential probes (click a second net with a modifier) are
in S6.

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

- `simulate { analysis?, signals?, points? }` (built, S5): runs an analysis of the
  setup (by name; no inline kinds) through the app's session and answers when it ends
  (120 s at most): an operating point's values, or for sweeps each signal decimated to
  ≤ 200 points plus min, max, average and end, and its values at `points`.
- `get_spice_netlist { analysis }` (built): the text, node and device maps, problems.
- Setup through `execute` with `SetSimulation` and component `Update`s (sources, model
  overrides) — no special tools. Part models through `edit_library` like other part
  content.
- `screenshot` target `simulation` (built): the plot of the last run, offscreen.
- The CLI's `simulate` exists (S1); agents get the same summary from `sim::report`.

## KiCad import

Built in S7 (design and gotchas in PLAN.md, "Libraries and KiCad"): `Sim.*` and KiCad
5's `Spice_*` fields become part models (parts split where models differ) and component
settings, inferred R/C/L/V/I as KiCad infers them, library models embedded, KiCad's
SPICE directive texts become analyses, temperature and extra lines. KiCad's
`Simulation_SPICE` symbols need no special case: their fields say what they are. Checked
against `kicad-cli` on KiCad's simulation demos (same operating point on every device
pin).

## Milestones

Each is a branch, tests, PLAN.md update and commit, as in AGENTS.md. S1–S5 are done and
give a usable simulator; S6–S8 make it comfortable, compatible and fast (all done).

### S6: Comfort (done)

Probes between nets and of a part's power, measurements, supply suggestions, the
tuner and parameter sweeps; see "Built so far" and PLAN.md.

### S7: KiCad simulation import (done)

The mapping in "KiCad import" above, directives, and the `kicad-cli` comparison test.

### S8: Shared-library backend (done)

libngspice as the default backend, live traces and progress, Stop keeping the partial
run, the tuner in-process; the circuit tests and the KiCad comparison run against both
backends. See "Built so far" and PLAN.md.

## Later

- ngspice built from source and linked statically instead of found installed (see
  PLAN.md, Later: Releases).
- Our own simulation engine in Rust beside ngspice, chosen in Settings: modified nodal
  analysis, Newton–Raphson, trapezoidal/Gear integration, a sparse solver; R, C, L,
  sources, diode, BJT, MOS level 1 first. A port of ngspice isn't realistic (~730k
  lines of C; the subset we use is 100–150k, and matching its convergence behaviour
  and the quirks vendor models rely on is the hard part), so it would be simpler and
  less compatible: ngspice stays for vendor models and KiCad projects.
- Streaming for sweeps and processes, the library for sweeps, crash isolation of the
  library (see PLAN.md, Later).
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
- Sweeps: two parameters (nested), a measurement plotted over the stepped value (the
  −3 dB point against R1), steps of a model parameter or the temperature; the
  Simulation panel's result table shows the first step only.
- Tuner: model parameter overrides as rows; agents have no tuner tool (`simulate`'s
  `values` do the same for one run).
- Probes and op annotations on sub-sheet instances when sheets are reused (not
  supported by the schematic today: every sheet is one instance).
- Board-aware simulation (track resistance, parasitic capacitance from the layout).
- KiCad export of simulation fields (with the KiCad project export).
- KiCad import: IBIS, XSPICE, switches, transmission lines, `K` couplings,
  potentiometers, the workbook's plotted signals (see PLAN.md, Later).

## Decisions taken

- Process backend first to get started; the shared library became the default in S8
  (the process stays for sweeps, as the fallback, and as the safe setting).
- One source part per kind (`VSource`, `ISource`), waveform picked in Properties; the
  symbol doesn't follow the waveform yet (Later).
- Sources stay ordinary components; a test bench sub-sheet keeps them out of a real
  design's main sheets.
- The setup lives in `schematic.json` (`Schematic::simulation`, like the ERC settings),
  not a file of its own: it rides in the change set's `Setup` with no new plumbing.
