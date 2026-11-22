# KiCad

PCB Designer follows KiCad's concepts (label scopes, hierarchical sheets, power symbols,
net classes), so KiCad's libraries and projects can be imported.

## KiCad libraries

**File → Import KiCad parts** imports symbols (as parts with a variant per checked
footprint, or a "No footprint" variant for parts that stay off the board) and footprints
from KiCad's libraries.

- A symbol's footprint list shows those matching its filters whose pads fit its pins,
  exact fits first; footprints missing pins are hidden unless you ask for them, and
  **Use** in the footprint search adds any footprint.
- The lists work with the keyboard: ↑/↓ (also from the search field) select and preview,
  Enter or a double click adds a result or toggles a footprint (Space too), Esc returns
  to the search field.
- Power symbols aren't imported: they are power ports here. Symbols with hidden power pins
  (older parts that connect VCC and GND implicitly) are refused; KiCad's current libraries
  have none.

It uses an installed KiCad's libraries, or downloads KiCad's official symbol and footprint
libraries (about 24 MB, 450 MB on disk, no 3D models) into the configuration folder, e.g.
`~/Library/Application Support/pcb-designer/kicad/10.0.8/` on macOS. The window offers the
download when no libraries are found, and **Check for updates** afterwards; the app only
goes online when you click one of these. `--kicad DIR` adds more library folders.

KiCad's libraries are licensed CC-BY-SA 4.0, with an exception that allows using them in
your designs without restrictions (see the KiCad library license).

## KiCad projects

**File → Import KiCad project…** picks a `.kicad_pro` (KiCad 6 or newer) and opens its
schematic (with all sheets) and board (footprints, tracks, vias, zones, keepouts, outline,
design rules and net classes) as a new, unsaved project. You're asked to save unsaved
changes first, as with Open. A report window lists warnings, notes on conversions, and
what wasn't imported (e.g. dimensions, title block, user-layer graphics). Parts come from
the project itself, so no libraries are needed; they land in the Project Library.

Projects from KiCad 5 or older (`.sch`) are refused: open and save them in a current KiCad
first.

### Simulation settings

KiCad's simulation fields become part models and source waveforms (models from
`Sim.Library` files are copied into the parts), and SPICE directive texts (`.tran`, `.ac`,
`.dc`, `.op`, `.param`, `.include`, …) become the [Simulation panel](simulation.md)'s
analyses and extra lines. What has no equivalent (IBIS, XSPICE digital models, switches,
transmission lines) is listed in the report and left out of the simulation.

### Without a window

```sh
pcb-designer PROJECT.kicad_pro                     # opens an import at startup
pcb-designer import-kicad PROJECT.kicad_pro OUT_DIR  # imports, saves, prints the report
```

Agents use `download_kicad_libraries`, `search_kicad`, `import_kicad` and
`import_kicad_project` (see [Agents](agents.md)).
