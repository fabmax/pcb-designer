# PCB Designer

A schematic and PCB editor written in Rust (egui + wgpu). Schematic changes show up on the
board immediately; projects are folders of JSON files. See [PLAN.md](PLAN.md) for the
architecture and roadmap.

```sh
cargo run --release -p app                 # an empty project
cargo run --release -p app -- my.pcbproj   # open a project folder
cargo run --release -p app -- --demo       # the demo project
cargo test --workspace
```

## Manufacturing files

File → Export Gerber files (⇧⌘E) writes Gerber X2 files (copper, solder mask, paste,
silkscreen, board outline), Excellon drill files (one more per blind or buried via
span, e.g. `Demo-front-in1.drl`) and a Gerber job file (`Demo-job.gbrjob`) into
`<project>.pcbproj/output/gerber/`, replacing the previous export, plus
`output/<name>-gerber.zip` for the manufacturer. Copper carries net, pad and component
attributes. For assembly it writes a bill of materials (`Demo-bom.csv`: grouped by
value, footprint and properties such as an MPN) and component positions
(`Demo-positions.csv`: board mm, rotation, side) into `output/assembly/`; board parts
(mounting holes, logos) are left out, and so are components marked **Do not populate**
in Properties (their pads stay on the board; the schematic crosses them out and both
editors draw them faded). Board setup → Assembly files lays both out for an assembly
service: **Generic** (a column per property), **JLCPCB** (`Comment, Designator,
Footprint, LCSC Part #`, the number from an `LCSC` property; positions `Designator, Mid
X, Mid Y, Layer, Rotation`) or **PCBWay** (manufacturer and part number from
`Manufacturer` and `MPN`, SMD or THT; positions with the value as comment).
Zones are refilled and the design rules checked first; violations are reported but don't
stop the export. Solder mask openings and paste come from the pads (Board setup: mask
expansion, paste margin). The job file carries the board's stack-up (Board setup →
Stack-up: thickness, copper in µm or oz, surface finish, mask and silkscreen colours).
Without a window: `pcb-designer export-gerber PROJECT_DIR [--assembly jlcpcb]`.

**File → Export schematic PDF** writes `output/<name>-schematic.pdf`: a page per sheet
(root first, each followed by its sub-sheets) the size of its page, with frame and title
block, in the light colours, bookmarked by sheet path. Text is drawn as outlines (not
selectable). Without a window: `pcb-designer export-schematic-pdf PROJECT_DIR`.
The `output` folder is generated, so you may want it in `.gitignore`.

The Project panel lists the files under Outputs › Gerber and Outputs › Assembly (marked
*outdated* when the design changed since); the job file and the CSV files open in your
system's app, and so does Outputs › Schematic PDF (double-click; it exports the PDF when
there is none yet). Double-click opens them read back from disk in a read-only
Gerber tab, stacked like the board in its colours (solder mask inverted, drills as
holes); double-clicking a single file shows only that one, the layers button toggles
them, and hovering names the file, aperture and its function, and the pad and net. Open the PCB tab side by
side to compare. Agents can look too: `screenshot` with target `gerber`.

## Drawing wires and tracks

The Wire, Bus and Route tools draw one leg per click. Each leg bends once: the first part
follows the way the pointer leaves the start point (horizontal or vertical, or straight
or diagonal at 45°); move back to the start to choose again, or press **/** to flip it.
The tool settings fix it if you prefer. Corner styles (tool settings, or Shift+Space to
cycle): 90°, 45° and free angle for wires and buses; 45°, 90°, arc 45° and arc 90° for
tracks (the arc styles round the corner as widely as the legs allow). Edit → Round track
corners rounds the corners between the selected tracks with the Route tool's corner
radius.

**Net classes** (Board setup → Net classes) give nets their own clearance, track width
and via size; values left at *default* are the board's (the Default class). A class lists
its nets by name or by pattern (`GND, +*V`, `/USB_D?`); a name beats a pattern, else the
first class matching wins. The Nets panel shows each net's class, and its context menu
puts a net into one. New tracks and vias take their net's class sizes (unless the Route
tool has a width of its own), and copper keeps the larger clearance of the two classes
in routing checks, zone fills and DRC. KiCad imports keep their net classes.

While routing and placing vias, the design rules are checked: clearance to other nets'
pads, tracks and vias, keepouts and the board outline. A track or via that breaks them is
drawn in the warning colour with a marker where it clashes, the hint bar says why, and
the click is refused. Shift+click adds it anyway (Shift+L for the via while routing), and
"Allow violations" in the tool settings always does; DRC then flags it. On boards
with inner layers, vias can be blind or buried: From/To layers in the Via tool and in a
via's properties, and "Via span" (through, or just the two layers) for the vias L adds.

## Selecting and dragging

Dragging keeps things connected, in the schematic and on the board. Wires attached to a
moved symbol follow it, and horizontal and vertical wires stay so. A wire end that sat on
something that stays put (a pin, a label, a branch) stays there, with a short wire joining
it. A single horizontal or vertical wire segment only moves across itself, and its
neighbours stretch. On the board, tracks follow a moved footprint or via, and a dragged
track segment moves across itself; track ends slide along the next track to keep 45° and
90° corners. Collinear wires a drag leaves end to end become one. Esc cancels a drag.

Double-click a wire to select its whole run up to branches and pins, or a track up to
pads, vias and branches; with ⇧ or ⌘ it's added to the selection.

## Copy and paste

⌘C / ⌘X / ⌘V (Ctrl on Windows and Linux) copy, cut and paste the selection; ⌘D
duplicates it without touching the clipboard. A paste follows the pointer (R rotates,
X mirrors) until you click; Esc cancels it. Works in the schematic (symbols, wires,
buses, labels, texts, sheet symbols; across sheets, and into other projects and app
instances, since clips carry their parts), on the board (tracks, vias, zones, keepouts,
graphics, texts, and board parts such as mounting holes; they keep their layers) and in
the symbol and footprint editors (pins, pads, graphics, texts; pins and pads get the
next free numbers). Pasted components get the next free reference unless theirs is free
(after a cut). A copied sheet symbol copies its whole sheet with its sub-sheets, as an
independent copy named amp2, amp3, …; their components get new references. Footprints of
schematic parts aren't copied on the board: copy their symbols in the schematic. Clips
are JSON text on the system clipboard (clips of older versions are refused).

Right-click the canvas for a menu with these and more. **Move to sheet…** (Edit menu or
the right-click menu; no default key, bind one in the settings) moves the selected
schematic objects to another sheet you pick from a list: they keep their identity
(references, footprints and routing stay), take their reference and value texts and
no-connect flags along, and follow the pointer on the new sheet until you click (Esc
undoes the move). Wires you didn't select stay, so the status bar tells how many nets
were renamed by the move. A moved sheet symbol takes its sheet along as a sub-sheet of
the target; in the Project tree you can also drag a sub-sheet onto another sheet.

## Zones and keepouts

The Zone tool (**Z**) draws a copper zone corner by corner (click the first or last corner
again to close it). Its net comes from the copper under the first corner or from the tool
settings. Zones fill around other nets with the clearance and refill on every change.
In Properties, choose how pads of the zone's net connect (thermal reliefs, reliefs for
through-hole pads only, solid, or not at all), override the clearance and thermal sizes,
and choose which unconnected islands are removed (always, never, or below an area). An
empty net makes isolated copper. Zones can have holes that only that zone leaves empty.

The Keepout tool (**K**) draws an area, on all copper layers or one, that zone fills leave
out. DRC reports tracks, vias, pads and, if enabled, footprint courtyards inside it.
Properties set what it keeps out and its layers.

The Cutout tool (tool bar, no default key) cuts a polygon out of a zone or keepout: start
inside it on the active layer and click the corners. Inside, the cutout becomes a hole
(merged with holes it overlaps); across the edge, it trims the outline. A cut that would
split the area in two is refused.

Select one zone, keepout or board drawing to edit its corners on the canvas: drag a
corner, drag an edge's midpoint to add a corner, and Delete (or double-click) a corner
under the pointer to remove it. Corners snap to the grid and to pads, track ends and
other corners; Alt drags freely, Shift keeps the edges at 0°, 45° or 90°. A rectangle
(like the board outline) resizes by its corners and edges; right-click → Convert to
polygon makes its corners editable one by one.
A circle has a radius handle; an arc has its two ends (they slide round the circle) and a
midpoint that changes the radius keeping the ends in place.

## Part editors

The symbol and footprint editors draw lines, rectangles, circles, arcs (**A**: click
the centre, the start, then the end follows the pointer; **/** goes round the other way)
and polygons (**G**: click the corners, close on the first or last one). Drawing snaps to
the grid and to pins, pads and corners; Alt snaps to the grid only. A selected drawing
has the same handles as board drawings.

**Create array…** (⇧⌘D, right-click or Edit menu) copies the one selected pad or pin:
in a line, two rows (SOIC, DIP; numbered counterclockwise), a grid (BGA; rows A, B, …
without I, O, Q, S, X, Z) or four sides (QFP, QFN; the end pads turned by 90°). The
selected pad is the first and numbers go on from its own. The settings are in the Tool
settings panel and the copies show on the canvas while you set them up; Enter (or Create)
makes them, Esc cancels. Names that are taken already are listed but allowed. Pins make
lines only.

The footprint editor shows each pad's solder mask opening and paste on the Solder mask
and Paste layers. A pad's Properties have Solder mask and Solder paste rows: the checkbox
turns the opening off, and the margin overrides the board's rule for this pad (it shows
the rule until you type, ↺ goes back to it). In the Project Library the project's rules
apply, elsewhere the defaults; board rules apply when the part is placed.

## Measuring

The layout editors (PCB, the part editor's footprint, the Gerber tab) have a Measure
tool (ruler button, **M**): click a start point, the ruler follows the pointer with its
length, ΔX, ΔY and angle in the current unit, a second click pins it, the next click
starts a new one. Ends snap to pad and via centres, track ends and corners (in the
Gerber tab: flash centres and line ends), from anywhere inside a pad, via or flash to its
centre, else to the grid; hold Alt for a free point. ⌘C (Ctrl+C) copies the reading as
text. Esc clears the measurement, a second Esc returns to Select.

On the board, **E** (or Tool settings) switches the Measure tool to clearances: click two
objects (copper on the active layer, a drill hole or the board edge) and it shows the gap
between their edges, with the clearance the design rules ask for; a gap below it is drawn
in the warning colour.

## Texts

The Text tool (schematic **T**, board **⇧T**) places a free text: click where it goes and
type. Its size, alignment and rotation (R) are in the tool settings; on the board also its
layer (Top Silkscreen by default; F switches the side) and weight (Bold by default, as
thin silkscreen doesn't fabricate well). Texts on copper have no net. Double-click a text to edit it in place: free texts, labels, sheet names and
reference and value texts. Arrows, Home/End, ⌥ (Ctrl) for words, ⌘ for lines, ⇧ to
select, ⌘A, the clipboard, and clicks, drags and double-clicks on the text work as in a
text field. Shift+Enter starts a new line in a free text, Enter finishes as one undo
step, Esc cancels, and clicking elsewhere or switching tools finishes too. An emptied free
text is deleted; labels and references can't be empty, and a reference already used is
refused. Properties edits free texts in a multi-line field. Footprint texts stay upright,
in the Gerber files too.

Texts are set in [JetBrains Mono](https://www.jetbrains.com/lp/mono/) (Regular and Bold),
which ships with the app under the SIL Open Font License 1.1
(`crates/render/fonts/OFL.txt`). The text size is the height of capitals; KiCad imports
keep their sizes but not their exact widths (KiCad's own font differs). In the Gerber
files texts are filled regions.

Many UI icons are [Tabler icons](https://tabler.io/icons) (MIT,
`crates/app/icons/tabler/LICENSE`).

Builds for macOS (Apple Silicon), Linux and Windows (x86_64) come from GitHub Actions:
the **Release** workflow keeps them as workflow artifacts when run by hand, and a
version tag (`v0.2.0`) publishes them as a GitHub Release. Every push to `main` and
every pull request runs the tests on all three (**CI**).

The macOS app is only ad-hoc signed (not notarized), so macOS refuses to open it at
first: open it once, then click **Open Anyway** in **System Settings → Privacy &
Security** and confirm. (Right-click → Open no longer does this since macOS 15.) A
downloaded archive can instead be cleared with
`xattr -dr com.apple.quarantine "PCB Designer.app"`.

On macOS, `scripts/bundle-macos.sh` builds `target/release/bundle/PCB Designer.app`
(`--install` copies it to /Applications). Run from the bundle, the app is named
"PCB Designer" in the menu bar and Dock; run with `cargo run`, macOS uses the binary's
name, `pcb-designer`.

## Checking the design (ERC and DRC)

The schematic is checked as you edit (Messages, ⌘3); the board's design rules when you
press **Run DRC**. Errors are listed before warnings, and errors and warnings are marked
on the canvas (View → ERC markers turns the schematic's off).

- **Severities:** each rule is an error, a warning or ignored: ERC rules and the pin
  conflict matrix (which pin kinds may share a net, defaults as in KiCad) in the
  Schematic setup panel, DRC rules in Board setup. Click a matrix cell to cycle it.
- **Exclusions:** right-click a finding → **Exclude** (with an optional comment). It moves
  into the collapsed *Excluded* list and no longer counts; **Include again** brings it
  back. Exclusions are saved with the project and follow the objects when they move.
  One that matches nothing any more shows as unused; **Remove unused** clears those.
- **Power flags** (⇧P): a power input fed from a connector or another passive pin
  needs a power source. Put a power flag on its net (on a wire or a pin) to say the
  net is powered, like KiCad's PWR_FLAG.

## Simulation

Circuits simulate with [ngspice](https://ngspice.sourceforge.io/), which you install
yourself (`brew install ngspice`, `apt install ngspice`, or the Windows download); it is
found on `PATH` and the usual install places, or set its path in **Settings →
Simulation**. When ngspice's shared library is installed too (Homebrew's `libngspice`,
`libngspice0` on Debian/Ubuntu, Spice64's `ngspice.dll`, or the one KiCad bundles), runs
go through it: they start at once, and transient, AC and DC sweep traces grow while
they run. A model that crashes ngspice then takes the app down with it; **Settings →
Simulation → Separate process only** runs everything in a process instead (parameter
sweeps always do). It runs in its LTspice/PSpice compatibility mode (as in KiCad), so
vendor models written for those simulators work. What's planned next is in
[simulation-plan.md](simulation-plan.md).

- **Simulation panel** (⌘2, or the **Simulation** menu): one tab per analysis
  (operating point, transient, AC, DC sweep; **+** adds one, right-click renames,
  duplicates or deletes it) with its settings. Values take SI prefixes (`10m`, `1.5k`,
  `1meg`); text that isn't a value turns red and isn't applied. **Run** (⌘R) runs the
  analysis shown (with none, it adds an operating point; drag the tabs to reorder them),
  **Stop** (⌘.) stops it and keeps what it computed so far ("Stopped at 3.2 ms"); the
  status bar shows a running simulation and, with the shared library, how far it is. The result table lists the named nets (all of
  them with **Show all nets**) and the measured currents; click a net to select it. An
  operating point measures the current into every pin (`I(R1.1)`) and each part's
  power (`P(R1)`). When the schematic changes in a
  way that changes the netlist, the results dim with "Changed since the run".
  **Circuit** holds the ground net, supplies, temperature and extra SPICE lines, and
  shows the SPICE netlist and ngspice's log. Power nets that no source or supply powers
  are listed with the voltage their name suggests (`+3V3` → 3.3 V) and an **Add**
  button. Every change undoes like any other edit.
- **Simulation tab:** the first transient, AC or DC sweep that finishes opens a plot
  of the analysis the panel shows (also **Plot** in the panel, or **Simulation → Show
  simulation plot**; once you close it, runs don't reopen it). It plots the signals
  ticked in the panel's result table, or the named nets while none are ticked. AC
  shows the magnitude in dB and, dashed on the right axis, the phase. Wheel zooms x
  around the pointer (Shift+wheel: y), middle or right drag pans, left drag zooms into
  a box, double-click or **V** fits. Click places cursor A, Shift+click cursor B
  (drag them to move, Esc removes them); the table under the plot shows each trace's
  value at A and B and the difference, and a click on a row hides or shows its trace.
  Hovering a row highlights its net or pin on the schematic; right-click → **Remove
  probe**. A probe without data says why (run again, net gone, part not simulated).
  **Export CSV…** saves the traces shown. **Measure** turns the table into measurements
  between cursors A and B (else over the visible range): min, max, average, RMS,
  peak-to-peak, frequency, rise and fall time (10–90 %); for AC the peak, the −3 dB
  points and the phase margin (loop gain at 0 dB). **Copy** copies them as text. Volts,
  amperes and watts each get an axis (a third one on the far right). An operating point
  shows a sortable table.
- **Tuner** (under the plot): try values without changing the schematic. Right-click
  a part → **Tune R1**, or **Tune…** lists every part value and source setting
  (amplitude, frequency, AC level, …). A row has a slider (log over a decade either
  way for values, snapped to E24 for resistors and E12 for capacitors and inductors;
  0 to twice the value for source settings), the value, the slider's range and
  snapping. Moving a slider runs the analysis again on release (while dragging when
  runs are quick); plot and schematic values show the tuned run. **Apply** writes a
  value into the schematic (one undo step), the header's **Apply** all of them,
  **Reset** goes back. The rows are saved with the project, the tuned values aren't.
- **Parameter sweeps:** tick **Parameter sweep** under an analysis' settings, pick a
  part value or source setting and its values (a list like `1k, 2.2k, 4.7k`, linear
  from/to/step, or log per decade; at most 64). Each value runs as its own ngspice
  process, several at once. The plot shows a trace per signal and step ("V(OUT) · R1 =
  4.7 kΩ"; one signal: a colour per step, several: each signal's steps in fading
  shades), the table groups them under the signal (click it to hide all its steps). An
  operating point shows a column per step; click a column to sort by it and see its
  values on the schematic.
- **Probe tool** (**O**, in the tool bar's Simulation group, shown with the panel or
  tab open): click a wire, label or junction to plot its net's voltage, drag from one
  net to another for the voltage between them (`V(OUT,IN)`), click the body of a pin
  (not its end) to plot the current into it, Alt-click a part for the power it takes
  (`P(R1)`, negative when it delivers power); click a probed one to remove it. The
  tip names what a click probes. Probes go to the analysis the panel shows if it's a
  transient, AC or DC sweep, else to the first one. Probed wires and pins show in
  their trace colours.
- **Values on the schematic** (**View → Simulation values**, with the panel or tab
  open): after an operating point run, each net shows its voltage once per sheet, and
  two-pin parts their current with an arrow in its direction; hover a bigger part for
  each pin's voltage and current and its power. They dim when the schematic changed
  since the run.
- **Test bench:** keep sources on a sheet of their own ("Testbench" in the demo): a
  source stands in for a connector's supply or signal, and has no footprint, so the
  board doesn't change.
- Problems (no ground, values that aren't numbers, parts without model, ngspice's
  errors) show in Messages' **Simulation** tab; click one to see its parts.
- **Models of your parts:** the part form's **Simulation** section picks the device
  (resistor, diode, BJT, MOSFET, op amp subcircuit, source, a raw SPICE line, or Not
  simulated), takes the model text (a `.model` card or a `.subckt`, pasted from a
  datasheet; the line under it says what it parsed), maps the model's ports to the
  part's pins (**Auto-map** matches names like A/K, C/B/E, D/G/S, + and −) and sets
  parameters. Model problems show in the library's issues.
- **A component's model:** Properties → **Simulation**: **Exclude from simulation**,
  parameter overrides (empty: the part's value, shown greyed), and for sources the
  waveform with its fields and a preview.
- **Parts:** every Built-in part has a simulation model (resistors, capacitors and
  inductors take their value: `10k`, `4k7`, `100n`, `1M` is mega). **VSource** and
  **ISource** (category Simulation) are sources for simulation only and never reach the
  board; their waveform (DC, sine, pulse, piecewise linear, exponential, plus an AC
  magnitude) is set on the component, and their value shows it ("SIN 1 V 1 kHz").
- **Ground** is the net of the GND power ports. **Supplies** in the simulation setup
  power nets like `+5V` without drawing a source.
- `pcb-designer simulate PROJECT_DIR [--analysis NAME] [--csv FILE] [--netlist]` runs an
  analysis of the project's setup (operating point, DC sweep, AC, transient; the first,
  or an operating point when there is none) and prints the values, or writes every point
  as CSV; `--netlist` prints the SPICE netlist. Agents set the setup up with the
  `SetSimulation` command and run it with the `simulate` MCP tool.

## Letting an agent use the app (MCP)

The app can run an [MCP](https://modelcontextprotocol.io) server so AI agents can inspect
and edit the open project while you watch. It is off by default.

1. Enable it with **Tools → Agent access (MCP server)**, or start the app with `--mcp`
   (port 7340) or `--mcp-port PORT`.
2. Connect your agent to `http://127.0.0.1:7340/mcp`. Tools → "Copy Claude Code command"
   copies `claude mcp add --transport http pcb-designer http://127.0.0.1:7340/mcp`;
   "Copy JSON config" gives an `mcpServers` entry for other clients.

The server only listens on 127.0.0.1 and rejects requests from web pages (browser
`Origin`) and foreign `Host` names. While it is enabled, any local program can edit the
open project, so only turn it on when you need it.

Tools: `get_project`, `list_library_parts`, `import_parts`, `update_parts`,
`search_kicad`, `import_kicad`, `download_kicad_libraries`, `import_kicad_project`, `list_objects`, `get_objects`, `get_component`,
`get_netlist` (nets, buses and their members, schematic and ERC problems), `get_routing`,
`run_drc`, `exclude_findings` (exclude or include ERC/DRC findings by key, with a
comment), `set_check_settings` (severities and the pin conflict matrix), `measure_clearance`, `export_gerber` (its `assembly` picks the BOM and position
layout), `export_schematic_pdf`, `execute` (any editor command, as one undo step, e.g.
`add_text` on a sheet or a board layer), `undo`, `redo`, `get_selection`, `set_selection`, `screenshot` (a schematic sheet, the board,
the last Gerber export, a simulation's plot or the whole window), `simulate` (runs an
analysis and answers its values: an operating point's voltages, currents and powers, or
per signal of a sweep (`OUT`, `V(OUT,IN)`, `R1.1`, `P(R1)`) up to 200 points, the values
at given x and the measurements of the Measure table; `values` tries part values and
source settings for that run only; the run shows in the app), `get_spice_netlist` (the netlist an analysis would run,
with node and device names and problems) and `save_project`. Sheets are added, renamed and deleted with
`execute` (`add_sheet`, `update`, `delete`), and `move_to_sheet` moves objects (or
re-parents a sheet) while `duplicate` copies objects in one step and `add_cutout` cuts into a zone or keepout; `get_project` lists their paths, and wherever
a sheet is asked for (`sheet`, `parent`), an id, a name or a path like `/Power/Regulator`
works. Agent edits go through the same commands as the
GUI, so they can be undone with ⌘Z, and the Agent activity panel lists what the agent did
(its button has a green dot while the server is on; the panel starts it when it's off).

Library tools let an agent make parts nobody has, e.g. from a datasheet: `list_libraries`,
`create_library`, `open_library`, `remove_library`, `get_library_items`, `edit_library`
(add or change symbols, footprints and parts; ids optional, references by name, pads
mapped to pins by number), `render_library_item` (a PNG of a symbol or footprint) and
`save_library`. Created and opened libraries are added to the project and open in a tab,
and `import_parts` puts their parts into the project. Library edits are undone in the
library's tab. `list_libraries` lists the Project Library first (kind `project`); edits
to it go through `execute` like any project change.

## Project panel and libraries

The Project panel (⌘1) shows a tree, IntelliJ style:

- **The project:** the Schematic with its sheets, the PCB (double-click to show them;
  their tabs can be closed like any other, and the tree opens them again) and the **Project
  Library**, the project's own copies of the parts the design uses. It is editable like
  any library, in its own tab: edits are project changes, so they share the project's
  undo history and update the schematic and board right away. Each part shows its state:
  "library changed" (Update takes the new version), "modified" (edited here), "both
  changed" (Update asks first, because it drops the local edit), "library not loaded",
  "removed from library", or "local" (made here). Parts nothing uses are dimmed, and
  "Remove unused parts" (right-click the Project Library) deletes them with the symbols
  and footprints only they needed. A part, unit or variant in use can't be removed, and
  neither can a symbol or footprint a used part needs.
- **Parts without a footprint** (power flags, logos): in the part form, a variant's
  footprint can be "None (not on board)". Components using such a variant stay in the
  schematic and its netlist but have no footprint on the board. A part can offer it next
  to real footprints; switching a component to a variant with a footprint places it in
  the staging area beside the board.
- **Board parts** (mounting holes, logos, fiducials) have no symbol: a part without units.
  They show a mounting-hole icon in the tree, and "Place on board" (or P) places them on
  the PCB, where R rotates and F flips the side; Delete removes them there. Their pads
  have no net, so copper of a net touching one is a short. Copy, paste and ⌘D work on
  them on the board, and Properties edits their reference and value (as for every
  footprint). The Built-in Library has mounting holes (M2 to M4) and a fiducial. Remove a part's last unit in the part form to make one.
- **Footprint variants** can be switched in Properties both for a symbol in the schematic
  and for a footprint on the board (the board lists only variants with a footprint).
- **Reference and value texts** belong to each symbol in the schematic and each footprint
  on the board: click one to select it, drag it away, R turns it; it moves (and on the
  board flips) with its owner, which is highlighted with a line to the text while it is
  selected. They stay upright whatever the owner's rotation. Properties sets the size,
  offset and rotation (and on the board the layer), hides it, and "Reset to library
  position" puts it back (the owner's Properties have a visibility checkbox per text).
  Library changes don't move placed texts.
- **Built-in Library:** a standard library of generic parts that ships with the app:
  resistors, capacitors (also polarised), inductors, ferrite beads, fuses, diodes
  (Schottky, Zener, TVS), LEDs, NPN/PNP transistors, N/P MOSFETs, crystals, test points, a
  push button, connectors `Conn_01x01`…`Conn_01x40` and `Conn_02x01`…`Conn_02x20` (pin
  headers and sockets; single rows also JST PH/XH/SH and screw terminals), a few ICs
  (op amps, 74HC00, regulators) and board parts (mounting holes M2 to M4, a fiducial).
  Each part offers its common packages, SMD and through-hole, as footprint variants;
  set the value per component. It is read-only; "Save a copy as…" makes an editable
  library of your own from it.
- **External Libraries:** the project's library files (`.pcblib`).
- **Categories:** parts, symbols and footprints can have a category: plain text such as
  "Passives" or "SMD", set in the part form or in a symbol's or footprint's Properties
  (with nothing selected). The ▼ menu next to the field lists the categories already
  in use. Each library's Parts, Symbols and Footprints show a folder per category,
  followed by the items without one. Right-click a folder → "Rename category…" to
  rename it on all of its items in one step (an empty name removes the category). The
  tree search also finds the items of a category by its name. The Built-in Library's
  parts are sorted into categories, and its footprints into "SMD", "Through hole" and
  "Mechanical". KiCad imports have no category.

**Sheets** work as in KiCad: the Schematic node is the root sheet, with its sub-sheets
nested below it. "New sheet…" (+ button, or "New sub-sheet…" on a sheet) asks for a
name; then the new sheet's symbol follows the cursor on its parent sheet until you click.
Double-click a sheet symbol (or the sheet in the tree) to open the sheet in its own tab.
Rename or delete sheets with right-click; deleting takes everything on the sheet and its
sub-sheets along (after asking). Reference designators are unique across all sheets.
Each sheet is drawn on a page with a frame: a zone grid and a title block. With nothing
selected, Properties shows the sheet's page (A5–A0, US and ANSI sizes, custom, or None
for no frame; landscape or portrait) and the project's title block (title, date,
revision, company and a comment), which every sheet's frame shows with its page number.
The comment field wraps in the panel; only its line breaks start new lines in the frame.
Local labels connect on their sheet only (nets are named `/Sheet/NAME`), global and power
labels everywhere. Power ports (Power tool, ⌘P) are drawn like KiCad's power symbols: pick
the glyph (bar, arrow, circle, ground, signal ground, chassis, protective earth) in the
tool settings or Properties, or leave it on Auto, which goes by the name (GND, AGND, … get
ground). Pins connect only through wires; there are no hidden pins. Messages lists pins
left unconnected; mark the ones open on purpose with a no-connect flag (tool bar, Q): an X
on the pin end, which moves and is deleted with its symbol. Hierarchical labels on a sheet become pins of its symbol on the parent
sheet (inputs on the left, the others on the right, in name order; the symbol grows to
fit them); wire them there like component pins. The Messages panel lists sheet pins left
unwired.

Libraries expand to Parts, Symbols and Footprints. A click (or the arrow keys) shows an
item in the preview tab (in italics), which the next item replaces; double-click (or
Enter) opens it in a tab of its own, as does editing it or double-clicking the preview's
tab. Past the most library tabs (Settings → Libraries, 10 by default) the tabs used
longest ago close; nothing is lost, edits stay in the library until it's saved. Right-click for more: place a part, copy it to the Project Library
(where it can be changed for this design only),
create, duplicate, rename or delete items, and save, reveal or remove libraries.

- **Keys:** ↑/↓ move, ←/→ collapse and expand, P places the selected part, Delete
  deletes an item.
- **Search:** type, or click the search button, to filter the tree. ↑/↓ step through the
  matches and Esc ends the search.
- **+ button:** New sheet…, New library… (asks for its file right away), Add library…,
  Import KiCad parts… (the library entries are also on right-click of External Libraries
  and in the File menu).

The project lists its library files in `project.json`, relative to the project folder,
and they load with the project. A file that can't be read shows as missing, with Locate…
and Remove. Adding a copy of a library that is already there is refused, because a copied
file keeps the library's id. `--library FILE` adds a library to the project (as an
unsaved change).

### Headless mode

Agents can also use the editor without a window: `pcb-designer --mcp-stdio [PROJECT_DIR]`
serves MCP on stdin/stdout. A folder that doesn't exist yet becomes a new project, saved
there by `save_project`; `--demo` starts from the demo project. For Claude Code:

```sh
claude mcp add pcb-designer -- /path/to/pcb-designer --mcp-stdio ~/boards/my.pcbproj
```

(Tools → "Copy Claude Code command (headless)" fills in the paths.) Screenshots are
rendered offscreen; `simulate` finds ngspice like the app (the Settings paths and
choice, `PATH`, the usual places).

## KiCad libraries

**File → Import KiCad parts** imports symbols (as parts with a variant per checked
footprint, or a "No footprint" variant for parts that stay off the board) and footprints
from KiCad's libraries. A symbol's footprint list shows those matching its filters whose
pads fit its pins, exact fits first; footprints missing pins are hidden unless you ask
for them, and "Use" in the footprint search adds any footprint. The lists work with the
keyboard: ↑/↓ (also from the search field) select and preview, Enter or a double click
adds a result or toggles a footprint (Space too), Esc returns to the search field. It uses an installed KiCad's libraries, or downloads KiCad's
official symbol and footprint libraries (about 24 MB, 450 MB on disk, no 3D models) into
the configuration folder, e.g. `~/Library/Application Support/pcb-designer/kicad/10.0.8/`
on macOS. The window offers the download when no libraries are found, and "Check for
updates" afterwards; the app only goes online when you click one of these. Agents use
`download_kicad_libraries`. `--kicad DIR` adds more library folders.

Power symbols aren't imported: they are power ports here. Symbols with hidden power pins
(older parts that connect VCC and GND implicitly) are refused; KiCad's current libraries
have none.

KiCad's libraries are licensed CC-BY-SA 4.0, with an exception that allows using them in
your designs without restrictions (see the KiCad library license).

## KiCad projects

**File → Import KiCad project…** picks a `.kicad_pro` (KiCad 6 or newer) and opens its
schematic (with all sheets) and board (footprints, tracks, vias, zones, keepouts,
outline, design rules and net classes) as a new, unsaved project. A report window lists warnings, notes
on conversions, and what wasn't imported (e.g. dimensions, title block, user-layer
graphics). Parts come from the project itself, so no libraries are needed; they land in
the Project Library. Projects from KiCad 5 or older (`.sch`) are refused: open and save
them in a current KiCad first.

Simulation settings come along: KiCad's simulation fields become part models and
source waveforms (models from `Sim.Library` files are copied into the parts), and
SPICE directive texts (`.tran`, `.ac`, `.dc`, `.op`, `.param`, `.include`, …) become
the Simulation panel's analyses and extra lines. What has no equivalent (IBIS, XSPICE
digital models, switches, transmission lines) is listed in the report and left out
of the simulation.

`pcb-designer PROJECT.kicad_pro` opens an import at startup.
`pcb-designer import-kicad PROJECT.kicad_pro OUT_DIR` imports without a window, saves
the project folder and prints the report. Agents use `import_kicad_project`; in the app
you're asked to save unsaved changes first, as with Open.

## Panels

Tool windows sit in narrow icon bars on the window edges, IntelliJ style. Click an icon
(or press its shortcut) to show its panel; click it again to hide it. Each bar has an
upper and a lower group (the left bar's foot holds the bottom panels); a group shows one
panel at a time, and when both groups of a side show one they stack, with a draggable
split. Drag an icon to reorder it or move it to another group; **View → Reset panel
layout** puts them back.

| Bar | Panel | Shortcut |
| --- | --- | --- |
| left | Project: the project tree and its libraries (see below) | ⌘1 |
| left, bottom panel | Messages: schematic problems, board problems with the DRC, library issues, the last simulation's problems, one tab each | ⌘3 |
| right | Properties | ⌘4 |
| right | Tool settings | ⌘5 |
| right | Board setup (with DRC severities) | ⌘6 |
| right | Overview (project or library statistics) | ⌘7 |
| right | Schematic setup: ERC severities and the pin conflict matrix | ⌘8 |
| right, lower | Nets: every net with its pins, sheets and routing state; click selects it, double-click zooms to it | ⌘9 |
| right, lower | Parts: the parts of all libraries the active editor places (see below) | ⌘0 |
| right, lower | Simulation: analyses, Run/Stop, results (see Simulation) | ⌘2 |
| left, bottom panel | Agent activity: the agent's calls this session, their arguments and results | – |

The **Parts** panel lists the parts of the Project Library, the loaded libraries and the
Built-in Library, grouped by library and category: schematic parts while the schematic is the active
editor, board parts (mounting holes, logos) while the board is. Categories start closed;
click a library or category row to open or close it. Search matches every word in the name, category, description and
properties (an MPN, say), and shows every category with matches open. Click a part to preview its symbol
or footprint; double-click it (or Enter) to place it, copying it into the Project Library
first. **P** on the schematic or the board opens the panel with the search field focused:
type, ↓, Enter, then click on the canvas. Esc clears the search, a second Esc goes back to
the canvas. Its context menu also opens a part in its library or copies it to the Project
Library. A link under the search field counts the matches in the KiCad libraries ("79 matches in the KiCad
libraries", or "Import KiCad libraries" without a search); clicking it opens the KiCad
import with the same search.

Properties, Tool settings and Overview follow the editor tab you used last. Messages
has a tab for the schematic, the board, the libraries and the simulation in its title bar; each tab's
badge shows a check when all is fine, else the number of problems (red with errors,
amber with only warnings). Clicking a message shows its objects. While Messages is hidden, its icon counts new problems (a dot
means only problems you have already seen). Which panels are open, and their sizes, are
saved with the settings.

## Settings

**Edit → Settings…** (⌘,) lists the categories on the left; type in the search field
(⌘F) to find a setting by name or keyword, or a shortcut by action or key. Changes
apply at once; each page has a **Reset to defaults**.

- **Appearance:** theme (dark or light), UI scale (80–200 %), length units (mm, mil or
  inch).
- **Layer colours:** the board layers' colours, separately for each theme (with alpha).
- **Keymap:** click `+` next to an action and press the new shortcut, × removes one.
- **Tools:** the drawing tools' style choices (corner styles, widths, sizes, layers,
  array layouts) are remembered across sessions; rotation, the next name and the
  routing layer are not. Reset forgets them.
- **Agent access:** start or stop the MCP server, its port, starting it on launch, and
  the commands and config to connect a client.
- **Libraries:** the most library tabs open at once (1–50, 10 by default), and where
  KiCad's libraries are downloaded to.
- **Simulation:** shared library when found or a separate process only; where ngspice
  and its library are (empty: found automatically); shows what's in use with its
  version, or how to install it.

These, the panel layout, the window's size and position and the recent projects are
saved in `settings.json` in the configuration folder (macOS:
`~/Library/Application Support/pcb-designer`). `PCB_DESIGNER_CONFIG=DIR` uses another
folder; `--keymap FILE.json` loads key bindings and `--mcp-port PORT` the port for one
session.

Each saved project remembers its workspace: open tabs and splits, the zoom and position
of each sheet, the board and each library tab, hidden layers and the routing layer. It's kept in the
project folder in `.pcb-designer/`, which ignores itself for git.

For development, `cargo run -p mcp --example mcp_call -- URL TOOL '{"json":"args"}'`
calls a tool from the command line (`TOOL = list` lists them).

## License

PCB Designer is licensed under the [MIT License](LICENSE). Bundled third-party assets
keep their own licenses (JetBrains Mono: SIL OFL 1.1; Tabler icons: MIT).
