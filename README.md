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
`get_netlist`, `get_routing`, `run_drc`, `execute` (any editor command, as one undo
step), `undo`, `redo`, `get_selection`, `set_selection`, `screenshot` (schematic, board
or the whole window) and `save_project`. Agent edits go through the same commands as the
GUI, so they can be undone with ⌘Z, and the status bar shows what the agent just did.

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

## Settings

Theme (dark or light), length units (mm, mil or inch), key bindings and recent projects
are saved in `settings.json` in the configuration folder (macOS:
`~/Library/Application Support/pcb-designer`). Edit them in **Edit → Settings…** (⌘,): click `+` next to an
action and press the new shortcut. `PCB_DESIGNER_CONFIG=DIR` uses another folder;
`--keymap FILE.json` loads key bindings for one session.

For development, `cargo run -p mcp --example mcp_call -- URL TOOL '{"json":"args"}'`
calls a tool from the command line (`TOOL = list` lists them).
