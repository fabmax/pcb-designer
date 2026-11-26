# Simulation

The schematic simulates as drawn, with [ngspice](https://ngspice.sourceforge.io/): the same
components, nets and labels, plus simulation-only sources that never reach the board.
Analyses: operating point, DC sweep, AC (frequency response) and transient.

## ngspice

ngspice comes with the app; there's nothing to install. Runs start at once, and
transient, AC and DC sweep traces grow while they run. A model that crashes ngspice takes
the app down with it, though; **Settings → Simulation → Run simulations → In a separate
process** runs every simulation in a process of its own instead (parameter sweeps always
do, several at once).

To use another ngspice, choose its shared library (libngspice, `ngspice.dll` on Windows)
in **Settings → Simulation**, or set the `NGSPICE_LIBRARY` environment variable. Built
from source, the app uses the ngspice that `scripts/build-ngspice.sh` builds, else one
that's installed (Homebrew's `libngspice`, `libngspice0` on Debian/Ubuntu).

ngspice runs in its LTspice/PSpice compatibility mode (as in KiCad), so vendor models
written for those simulators work.

## A first run

1. Place a source (**VSource** or **ISource**, category Simulation in the
   [Parts panel](libraries.md#the-parts-panel)) and wire it in, or let a **supply** power
   a net (below). Ground is the net of the GND power ports.
2. Add a simulation (an analysis): right-click **Simulation** in the
   [Project panel](libraries.md#the-project-panel) (or use its **+** button) → **Add
   simulation**. The **Simulation panel** (⌘2) shows its settings.
3. **Run** (⌘R). Its output opens in a tab: the plot of a transient, AC or DC sweep, the
   table of an operating point, whose values also show on the schematic.
4. Click wires and pins with the **Probe** tool (**O**) to plot more.
5. Change a value and run again; results of an older schematic dim until then.

The [example projects](user-interface.md#example-projects) have test bench sheets and
analyses to try: the LED demo an operating point, a transient and AC; the audio amplifier
its bias point, the output into an 8 Ω load (a SPICE line in Extra) and its frequency
response.

## Simulations in the Project panel

The project's **Simulation** node, below the PCB, lists its simulations: operating point,
transient, AC for a frequency response, DC sweep for a transfer curve. Right-click the node
(or use the panel's **+** button) → **Add simulation** to add one; right-click a simulation
to open, run, rename, duplicate or delete it (Delete deletes the selected one too). Names
are free text: the audio amplifier example calls its AC analysis "Frequency response".

Each row says how its simulation stands: "42 %" while it runs, "outdated" when the
schematic changed since its run, its error count when it failed. The node counts the
outdated ones. **Run all simulations** (⇧⌘R, also on the node's menu and in the panel)
runs every simulation in turn; the status bar says how many ran and failed.

A click on a simulation shows its output in the preview tab (in italics), which the next
click replaces (a library item's preview too); double-click (or Enter) opens it in a tab
of its own, as does double-clicking the preview's tab.

## Simulation panel

The panel shows the settings of the simulation selected last in the Project panel (or
whose tab you clicked last); it keeps showing it while you select other things. Selecting
the **Simulation** node shows the **Circuit** settings that all simulations share. Values
take SI prefixes (`10m`, `1.5k`, `1meg`); text that isn't a value turns red and isn't
applied.

- **Run** (⌘R) runs the simulation shown (with none in the project, it adds an operating
  point) and brings its tab to the front, or shows it in the preview tab. **Run all**
  (⇧⌘R) runs all of them without opening tabs. **Stop** (⌘.) stops a run (and the rest of
  Run all) and keeps what it computed so far ("Stopped at 3.2 ms"); the status bar shows a
  running simulation and, with the shared library, how far it is.
- The **result table** lists the named nets (all of them with **Show all nets**) and the
  measured currents; click a net to select it. An operating point measures the current
  into every pin (`I(R1.1)`) and each part's power (`P(R1)`), and shows their values.
  After a sweep, tick **Plot** to plot a signal, and **Logic** to show a net as a logic
  level (it plots the net too; transient and DC sweep).
- When the schematic changes in a way that changes the netlist, the results dim with
  "Changed since the run".
- **Circuit** holds the ground net, supplies, temperature and extra SPICE lines, and shows
  the SPICE netlist and ngspice's log. Power nets whose name says their voltage (`+5V`,
  `+3V3`) get a supply automatically when no source drives them; they're listed there.
  Type another voltage to change one, or click ✕ to leave the net without a supply (when
  something else powers it). Power nets without a source (`VCC`, a regulator's output,
  or one you left out) are listed below with an **Add** button.

Every change undoes like any other edit.

## Simulation tabs (the plot)

Each simulation has a tab of its own (from the Project panel, a run, **Show** in the panel,
or **Simulation → Show simulation output**), so several can be side by side. It plots the
signals ticked in the panel's result table, or the named nets while none are ticked. Until
the simulation has run, the tab says so and has a **Run** button.

- AC shows the magnitude in dB and, dashed on the right axis, the phase. Volts, amperes and
  watts each get an axis (a third one on the far right).
- Wheel zooms x around the pointer (Shift+wheel: y), middle or right drag pans, left drag
  zooms into a box, double-click or **V** fits.
- Click places cursor A, Shift+click cursor B (drag them to move; Esc, or right-click →
  **Clear cursors**, removes them). The table under the plot shows each trace's value at
  A and B and the difference; a click on a row hides or shows its trace. Hovering a row highlights its net or pin on the
  schematic; right-click → **Remove probe**. A probe without data says why (run again, net
  gone, part not simulated).
- **Measure** turns the table into measurements between cursors A and B (else over the
  visible range): min, max, average, RMS, peak-to-peak, frequency, rise and fall time
  (10–90 %), and over time the THD of a periodic signal; for AC the peak, the −3 dB
  points and the phase margin (loop gain at 0 dB). **Copy** copies them as text.
- Drag the handle between the plot and the table to give either more room;
  double-click it for the automatic size.
- Tick **Logic** in the result table, or right-click a voltage probe's row → **Show as
  logic**, to show it as a logic level
  instead: a lane under the plot, high or low, like a logic analyser (transient and DC
  sweep). It switches halfway between the trace's lowest and highest voltage, so it
  works at any supply. Its values read 0 or 1. With only logic traces, the lanes fill
  the plot. The setting is saved with the probe.
- **Export CSV…** saves the traces shown (a spectrum when it shows one).
- An operating point shows a sortable table.

## Distortion

A transient's **Spectrum** (tool bar) shows the traces between cursors A and B, else the
time you were looking at, as levels in dB (of 1 V or 1 A, RMS) over log frequency. Set the
cursors in the time plot first, then switch. **Measure** shows the distortion of each
trace there:

- **Fundamental** and its **Level** (RMS), found from the signal's crossings.
- **THD**: the harmonics (up to the 10th) over the fundamental.
- **THD+N**: everything but DC and the fundamental. Above the THD it's noise: in a
  simulation, the numerical error and the coarseness of the time steps.
- **H2 … H5** in dB of the fundamental (dBc). A strong H2 means asymmetry (the halves of
  a push-pull stage differ); odd harmonics mean symmetric distortion: crossover or
  clipping.

The harmonics are measured over the last whole periods of the window, where the circuit
has settled the most, so let it settle before the window starts (**Start saving at** in
the transient). For a clean result:

- **Max. step**: at most a hundredth of the period, less for low distortion. The THD+N
  of the input shows the floor the steps allow (about 0.015 % at 100 steps a period).
- Ten periods or more.
- For distortion far below 0.01 %, tighter tolerances help: add
  `.options reltol=1e-5` in the setup's **Extra** lines.

The audio amplifier example shows it: run its Transient and switch to Spectrum.

## Probing

The **Probe** tool (**O**, in the tool bar's Simulation group, shown with the panel or tab
open):

- click a wire, label or junction to plot its net's voltage;
- drag from one net to another for the voltage between them (`V(OUT,IN)`);
- click the body of a pin (not its end) to plot the current into it;
- Alt-click a part for the power it takes (`P(R1)`, negative when it delivers power);
- click a probed one to remove it.

The tip names what a click probes. Probes go to the simulation the panel shows if it's a
transient, AC or DC sweep, else to the first sweep. Probed wires and pins show in their trace
colours.

**Values on the schematic** (with the panel or tab open):
after an operating point run, each net shows its voltage once per sheet (blue), and two-pin
parts their current with an arrow in its direction (magenta), placed clear of texts and
each other where there's room; hover a bigger part for each pin's voltage and current
and its power. They dim when the schematic changed since the run.

Probes and values are on the schematic's **Simulation** layer: hide it in the layer menu
to see the schematic without them. Prints and PDFs never show it.

## Tuner

Under the plot, the tuner tries values without changing the schematic. Right-click a part →
**Tune R1**, or **Tune…** lists every part value and source setting (amplitude, frequency,
AC level, …). A row has a slider (log over a decade either way for values, snapped to E24
for resistors and E12 for capacitors and inductors; 0 to twice the value for source
settings), the value, the slider's range and snapping. Moving a slider runs the analysis
again on release (while dragging when runs are quick); plot and schematic values show the
tuned run. **Apply** writes a value into the schematic (one undo step), the header's
**Apply** all of them, **Reset** goes back. The rows are saved with the project, the tuned
values aren't.

## Parameter sweeps

Tick **Parameter sweep** under an analysis' settings, pick a part value or source setting
and its values (a list like `1k, 2.2k, 4.7k`, linear from/to/step, or log per decade; at
most 64). Each value runs in its own process, several at once. The plot shows a
trace per signal and step ("V(OUT) · R1 = 4.7 kΩ"; one signal: a colour per step, several:
each signal's steps in fading shades), the table groups them under the signal (click it to
hide all its steps). An operating point shows a column per step; click a column to sort by
it and see its values on the schematic.

## Sources, supplies and test benches

- **VSource** and **ISource** (category Simulation) are sources for simulation only and
  never reach the board. Their waveform (DC, sine, pulse, piecewise linear, exponential,
  plus an AC magnitude) is set on the component, and their value shows it
  ("SIN 1 V 1 kHz").
- **Resistor_Sim**, **Capacitor_Sim** and **Inductor_Sim** (category Simulation) are
  passives without footprint, for loads, a source's impedance or parasitics that belong
  in the simulation but not on the board.
- **Supplies** power nets like `+5V` without drawing a source: automatic for power ports
  whose name says the voltage, unless a regulator's output is on the net.
- **Test bench:** keep sources on a sheet of their own ("Testbench" in the demo): a source
  stands in for a connector's supply or signal, and has no footprint, so the board doesn't
  change.

## Models

### Parts that simulate out of the box

Every Built-in part has a simulation model. Resistors, capacitors and inductors take their
value: `10k`, `4k7`, `100n`, `1M` is mega.

The 74HC logic parts simulate as digital logic (ngspice's XSPICE), which stays fast
with many gates. Their inputs switch at half the supply (the 74HC14's at about 60 % going
up and 40 % going down), and their outputs swing to the supply through 50 Ω. Delays
are typical 5 V values whatever the supply. Their supply current isn't modelled: a
power analysis shows them taking none.

### Models of your parts

The part form's **Simulation** section:

- picks the device (resistor, diode, BJT, MOSFET, op amp subcircuit, source, a raw SPICE
  line, or Not simulated);
- takes the model text (a `.model` card or a `.subckt`, pasted from a datasheet; the line
  under it says what it parsed);
- maps the model's ports to the part's pins (**Auto-map** matches names like A/K, C/B/E,
  D/G/S, + and −);
- sets parameters.

Model problems show in the library's issues.

### A component's model

Properties → **Simulation** (closed until you open it; its title says what's set:
*excluded*, a source's waveform, the number of overrides): **Exclude from simulation**,
parameter overrides (empty: the part's value, shown greyed), and for sources the waveform
with its fields and a preview.

## Problems

Problems (no ground, values that aren't numbers, parts without model, ngspice's errors)
show in Messages' **Simulation** tab; click one to see its parts.

## KiCad projects

KiCad's simulation fields and SPICE directives come along with an imported project: see
[KiCad](kicad.md#simulation-settings).

## Command line and agents

```sh
pcb-designer simulate PROJECT_DIR [--analysis NAME] [--csv FILE] [--netlist]
```

runs an analysis of the project's setup (the first, or an operating point when there is
none) and prints the values, or writes every point as CSV; `--netlist` prints the SPICE
netlist.

Agents set the simulation up with the `SetSimulation` command and run it with the
`simulate` MCP tool: see [Agents](agents.md).
