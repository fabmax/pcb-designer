# Letting an agent use the app (MCP)

The app can run an [MCP](https://modelcontextprotocol.io) server so AI agents can inspect
and edit the open project while you watch, or work on a project without a window.

Agent edits go through the same commands as the GUI, so they can be undone with ⌘Z, and
the **Agent activity** panel lists what the agent did: each call with its arguments and
result.

## In the app

The server is off by default.

1. Turn it on with the **Access** switch in the Agent activity panel's header or **Tools →
   Agent access (MCP server)**, or start the app with `--mcp` (port 7340) or
   `--mcp-port PORT`. While it runs, the header shows the URL it listens on, and the
   panel's button has a green dot.
2. Connect your agent to `http://127.0.0.1:7340/mcp`. **Tools → Copy Claude Code command**
   copies `claude mcp add --transport http pcb-designer http://127.0.0.1:7340/mcp`;
   **Copy JSON config** gives an `mcpServers` entry for other clients.

The server only listens on 127.0.0.1 and rejects requests from web pages (browser
`Origin`) and foreign `Host` names. While it is enabled, any local program can edit the
open project, so only turn it on when you need it. **Settings → Agent access** sets the
port and whether it starts on launch.

## Headless

Agents can also use the editor without a window: `pcb-designer --mcp-stdio [PROJECT_DIR]`
serves MCP on stdin/stdout. A folder that doesn't exist yet becomes a new project, saved
there by `save_project`; `--demo` starts from the demo project. For Claude Code:

```sh
claude mcp add pcb-designer -- /path/to/pcb-designer --mcp-stdio ~/boards/my.pcbproj
```

(**Tools → Copy Claude Code command (headless)** fills in the paths.) Screenshots are
rendered offscreen; `simulate` finds ngspice like the app (the Settings paths and choice,
`PATH`, the usual places).

## Tools

### Project and editing

| Tool | What it does |
| --- | --- |
| `get_project` | Overview: sheets with their paths, libraries, unsaved changes, undo state |
| `list_objects`, `get_objects`, `get_component` | Read the design |
| `get_netlist` | Nets, buses and their members, schematic and ERC problems |
| `get_routing` | The board's routing state |
| `execute` | Any editor command as one undo step (e.g. `add_text` on a sheet or a board layer, `add_sheet`, `update`, `delete`, `move_to_sheet`, `duplicate`, `add_cutout`, `SetSimulation`) |
| `route` | A track laid by the router (shove, walk around or highlight); returns what it pushed |
| `undo`, `redo` | As in the app |
| `get_selection`, `set_selection` | The app's selection |
| `screenshot` | A schematic sheet, the board, the last Gerber export, a simulation's plot or the whole window |
| `save_project` | Saves the project |

Wherever a sheet is asked for (`sheet`, `parent`), an id, a name or a path like
`/Power/Regulator` works. `move_to_sheet` moves objects (or re-parents a sheet) and
`duplicate` copies objects in one step.

### Checks and output

| Tool | What it does |
| --- | --- |
| `run_drc` | Runs the design rule check |
| `exclude_findings` | Excludes or includes ERC/DRC findings by key, with a comment |
| `set_check_settings` | Severities and the pin conflict matrix |
| `measure_clearance` | The gap between two objects and the clearance the rules ask for |
| `export_gerber` | Gerber, drill and assembly files (`assembly` picks the BOM and position layout) |
| `export_schematic_pdf` | The schematic as PDF |

### Simulation

| Tool | What it does |
| --- | --- |
| `simulate` | Runs an analysis and answers its values: an operating point's voltages, currents and powers, or per signal of a sweep (`OUT`, `V(OUT,IN)`, `R1.1`, `P(R1)`) up to 200 points, the values at given x and the measurements of the Measure table. `values` tries part values and source settings for that run only. The run shows in the app |
| `get_spice_netlist` | The netlist an analysis would run, with node and device names and problems |

### Parts and libraries

| Tool | What it does |
| --- | --- |
| `list_library_parts` | Parts of all open libraries |
| `import_parts`, `update_parts` | Copy parts into the Project Library, update them from their library |
| `search_kicad`, `import_kicad`, `download_kicad_libraries` | KiCad's libraries ([KiCad](kicad.md)) |
| `import_kicad_project` | Opens a KiCad project as a new project |

### Library tools

Library tools let an agent make parts nobody has, e.g. from a datasheet:

| Tool | What it does |
| --- | --- |
| `list_libraries` | The project's libraries, the Project Library first (kind `project`) |
| `create_library`, `open_library`, `remove_library` | Add libraries to the project (they open in a tab) or remove them |
| `get_library_items` | Symbols, footprints and parts of a library |
| `edit_library` | Adds or changes symbols, footprints and parts (ids optional, references by name, pads mapped to pins by number) |
| `render_library_item` | A PNG of a symbol or footprint |
| `save_library` | Saves a library file |

`import_parts` puts their parts into the project. Library edits are undone in the library's
tab; edits to the Project Library go through `execute` like any project change.

## From the command line

For development, `cargo run -p mcp --example mcp_call -- URL TOOL '{"json":"args"}'` calls
a tool of a running app (`TOOL = list` lists them).
