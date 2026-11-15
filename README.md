# PCB Designer

A schematic and PCB editor written in Rust (egui + wgpu). Schematic changes show up on the
board immediately; projects are folders of JSON files. See [PLAN.md](PLAN.md) for the
architecture and roadmap.

```sh
cargo run --release -p app                 # opens a demo project
cargo run --release -p app -- my.pcbproj   # open a project folder
cargo run --release -p app -- --new        # empty project
cargo test --workspace
```

## Manufacturing files

File → Export Gerber files (⇧⌘E) writes Gerber X2 files (copper, solder mask, paste,
silkscreen, board outline) and Excellon drill files into `<project>.pcbproj/output/gerber/`,
replacing the previous export, plus `output/<name>-gerber.zip` for the manufacturer.
Zones are refilled and the design rules checked first; violations are reported but don't
stop the export. Solder mask openings and paste come from the pads (Board setup: mask
expansion, paste margin). Without a window: `pcb-designer export-gerber PROJECT_DIR`.
The `output` folder is generated, so you may want it in `.gitignore`.

The Project panel lists the files under Outputs › Gerber (marked *outdated* when the
design changed since). Double-click opens them read back from disk in a read-only
Gerber tab, stacked like the board in its colours (solder mask inverted, drills as
holes); double-clicking a single file shows only that one, the layers button toggles
them, and hovering names the file, aperture and its function. Open the PCB tab side by
side to compare. Agents can look too: `screenshot` with target `gerber`.

## Copy and paste

⌘C / ⌘X / ⌘V (Ctrl on Windows and Linux) copy, cut and paste the selection; ⌘D
duplicates it without touching the clipboard. A paste follows the pointer (R rotates,
X mirrors) until you click; Esc cancels it. Works in the schematic (symbols, wires,
buses, labels, texts; across sheets, and into other projects and app instances, since
clips carry their parts), on the board (tracks, vias, zones, keepouts, graphics, texts; they keep
their layers) and in the symbol and footprint editors (pins, pads, graphics, texts;
pins and pads get the next free numbers). Pasted components get the next free
reference unless theirs is free (after a cut). Clips are JSON text on the system
clipboard.

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

## Measuring

The layout editors (PCB, the part editor's footprint, the Gerber tab) have a Measure
tool (ruler button, **M**): click a start point, the ruler follows the pointer with its
length, ΔX, ΔY and angle in the current unit, a second click pins it, the next click
starts a new one. Ends snap to pad and via centres, track ends and corners (in the
Gerber tab: flash centres and line ends), else to the grid; hold Alt for a free point.
⌘C (Ctrl+C) copies the reading as text. Esc clears the measurement, a second Esc
returns to Select.

On macOS, `scripts/bundle-macos.sh` builds `target/release/bundle/PCB Designer.app`
(`--install` copies it to /Applications). Run from the bundle, the app is named
"PCB Designer" in the menu bar and Dock; run with `cargo run`, macOS uses the binary's
name, `pcb-designer`.

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
`run_drc`, `export_gerber`, `execute` (any editor command, as one undo step), `undo`, `redo`, `get_selection`, `set_selection`, `screenshot` (a schematic sheet, the board
or the whole window) and `save_project`. Sheets are added, renamed and deleted with
`execute` (`add_sheet`, `update`, `delete`); `get_project` lists their paths, and wherever
a sheet is asked for (`sheet`, `parent`), an id, a name or a path like `/Power/Regulator`
works. Agent edits go through the same commands as the
GUI, so they can be undone with ⌘Z, and the status bar shows what the agent just did.

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
  have no net, so copper of a net touching one is a short. The Built-in Library has an
  M3 mounting hole. Remove a part's last unit in the part form to make one.
- **Footprint variants** can be switched in Properties both for a symbol in the schematic
  and for a footprint on the board (the board lists only variants with a footprint).
- **Reference and value texts** on the board belong to each footprint: click one to
  select it, drag it away from the pads, R turns it; it moves and flips with its
  footprint. Properties sets its layer, size, offset and rotation, and hides it (the
  footprint's Properties have a visibility checkbox per text).
- **Built-in Library:** a few basic parts that ship with the app. It is read-only; "Save a
  copy as…" makes an editable library of your own from it.
- **External Libraries:** the project's library files (`.pcblib`).

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
labels everywhere. Power ports (Power tool, P) are drawn like KiCad's power symbols: pick
the glyph (bar, arrow, circle, ground, signal ground, chassis, protective earth) in the
tool settings or Properties, or leave it on Auto, which goes by the name (GND, AGND, … get
ground). Pins connect only through wires; there are no hidden pins. Messages lists pins
left unconnected; mark the ones open on purpose with a no-connect flag (tool bar, Q): an X
on the pin end, which moves and is deleted with its symbol. Hierarchical labels on a sheet become pins of its symbol on the parent
sheet (inputs on the left, the others on the right, in name order; the symbol grows to
fit them); wire them there like component pins. The Messages panel lists sheet pins left
unwired.

Libraries expand to Parts, Symbols and Footprints. Double-click (or Enter) opens an item
in its library tab. Right-click for more: place a part, copy it to the Project Library
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
rendered offscreen.

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
outline, design rules) as a new, unsaved project. A report window lists warnings, notes
on conversions, and what wasn't imported (e.g. dimensions, title block, user-layer
graphics). Parts come from the project itself, so no libraries are needed; they land in
the Project Library. Projects from KiCad 5 or older (`.sch`) are refused: open and save
them in a current KiCad first.

`pcb-designer PROJECT.kicad_pro` opens an import at startup.
`pcb-designer import-kicad PROJECT.kicad_pro OUT_DIR` imports without a window, saves
the project folder and prints the report. Agents use `import_kicad_project`; in the app
you're asked to save unsaved changes first, as with Open.

## Panels

Tool windows sit in narrow icon bars on the window edges, IntelliJ style. Click an icon
(or press its shortcut) to show its panel; click it again to hide it. Each side shows one
panel at a time.

| Bar | Panel | Shortcut |
| --- | --- | --- |
| left | Project: the project tree and its libraries (see below) | ⌘1 |
| left, bottom panel | Messages: schematic problems, board problems, DRC, library issues | ⌘3 |
| right | Properties | ⌘4 |
| right | Tool settings | ⌘5 |
| right | Board setup | ⌘6 |
| right | Overview (project or library statistics) | ⌘7 |

Properties, Tool settings and Overview follow the editor tab you used last. Clicking a
message shows its objects. While Messages is hidden, its icon counts new problems (a dot
means only problems you have already seen). Which panels are open, and their sizes, are
saved with the settings.

## Settings

Theme (dark or light), length units (mm, mil or inch), key bindings, the panel layout and recent projects
are saved in `settings.json` in the configuration folder (macOS:
`~/Library/Application Support/pcb-designer`). Edit them in **Edit → Settings…** (⌘,): click `+` next to an
action and press the new shortcut. `PCB_DESIGNER_CONFIG=DIR` uses another folder;
`--keymap FILE.json` loads key bindings for one session.

For development, `cargo run -p mcp --example mcp_call -- URL TOOL '{"json":"args"}'`
calls a tool from the command line (`TOOL = list` lists them).
