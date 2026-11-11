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
`search_kicad`, `import_kicad`, `download_kicad_libraries`, `list_objects`, `get_objects`, `get_component`,
`get_netlist` (nets, buses and their members, schematic problems), `get_routing`,
`run_drc`, `execute` (any editor command, as one undo step), `undo`, `redo`, `get_selection`, `set_selection`, `screenshot` (a schematic sheet, the board
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
- **Built-in Library:** a few basic parts that ship with the app. It is read-only; "Save a
  copy as…" makes an editable library of your own from it.
- **External Libraries:** the project's library files (`.pcblib`).

**Sheets** work as in KiCad: the Schematic node is the root sheet, with its sub-sheets
nested below it. "New sheet…" (+ button, or "New sub-sheet…" on a sheet) asks for a
name; then the new sheet's symbol follows the cursor on its parent sheet until you click.
Double-click a sheet symbol (or the sheet in the tree) to open the sheet in its own tab.
Rename or delete sheets with right-click; deleting takes everything on the sheet and its
sub-sheets along (after asking). Reference designators are unique across all sheets.
Local labels connect on their sheet only (nets are named `/Sheet/NAME`), global and power
labels everywhere. Hierarchical labels on a sheet become pins of its symbol on the parent
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
  Import from KiCad… (the library entries are also on right-click of External Libraries
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

**File → Import from KiCad** imports symbols (as parts, with a footprint) and footprints
from KiCad's libraries. It uses an installed KiCad's libraries, or downloads KiCad's
official symbol and footprint libraries (about 24 MB, 450 MB on disk, no 3D models) into
the configuration folder, e.g. `~/Library/Application Support/pcb-designer/kicad/10.0.8/`
on macOS. The window offers the download when no libraries are found, and "Check for
updates" afterwards; the app only goes online when you click one of these. Agents use
`download_kicad_libraries`. `--kicad DIR` adds more library folders.

KiCad's libraries are licensed CC-BY-SA 4.0, with an exception that allows using them in
your designs without restrictions (see the KiCad library license).

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
