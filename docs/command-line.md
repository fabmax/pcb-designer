# Command line

The binary is `pcb-designer` (inside the macOS app:
`PCB Designer.app/Contents/MacOS/pcb-designer`).

## Opening the app

```sh
pcb-designer                       # an empty project
pcb-designer my-board              # open a project folder (or my-board/project.pcbproj)
pcb-designer PROJECT.kicad_pro     # import a KiCad project at startup
pcb-designer --demo                # the audio amplifier example
pcb-designer --example amp         # an example project: led, amp
```

| Option | |
| --- | --- |
| `--library FILE` | Adds a library to the project (as an unsaved change); repeatable |
| `--kicad DIR` | Another folder of KiCad libraries; repeatable |
| `--keymap FILE.json` | Key bindings for this session |
| `--mcp`, `--mcp-port PORT` | Starts the [MCP server](agents.md) (port 7340 by default) |
| `--mcp-stdio [PROJECT_DIR]` | [Headless](agents.md#headless) MCP on stdin/stdout, no window |

`PCB_DESIGNER_CONFIG=DIR` uses another configuration folder (settings, recovery copies,
downloaded KiCad libraries).

## Without a window

```sh
pcb-designer export-gerber PROJECT_DIR [--assembly generic|jlcpcb|pcbway]
pcb-designer export-schematic-pdf PROJECT_DIR
pcb-designer simulate PROJECT_DIR [--analysis NAME] [--csv FILE] [--netlist]
pcb-designer import-kicad PROJECT.kicad_pro OUT_DIR
pcb-designer export-kicad PROJECT_DIR OUT_DIR
```

- `export-gerber` writes the [manufacturing files](manufacturing.md); `--assembly` picks
  the layout of the BOM and position files (else the project's setting).
- `export-schematic-pdf` writes the [schematic PDF](manufacturing.md#schematic-pdf) and
  prints its path.
- `simulate` runs an analysis of the project's [simulation setup](simulation.md) (the
  first, or an operating point when there is none) and prints the values, or writes every
  point as CSV; `--netlist` prints the SPICE netlist.
- `import-kicad` imports a [KiCad project](kicad.md#kicad-projects), saves the project
  folder and prints the report.
- `export-kicad` writes a project as a [KiCad project](kicad.md#exporting-to-kicad) into
  `OUT_DIR` and prints the report.

Options for development (screenshots, scripted input, stress boards) are in
[AGENTS.md](https://github.com/fabmax/pcb-designer/blob/main/AGENTS.md#seeing-the-app).
