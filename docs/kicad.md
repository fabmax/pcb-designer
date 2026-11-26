# KiCad

PCB Designer follows KiCad's concepts (label scopes, hierarchical sheets, power symbols,
net classes), so KiCad's libraries and projects can be imported, and projects and libraries exported
to KiCad.

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
analyses and extra lines. XSPICE code models come along with their `.model` card; place
the analog-digital bridges as in KiCad. Potentiometers become the built-in potentiometer
model, so their slider on the schematic works; voltage and current controlled switches
come along. What has no equivalent (IBIS, transmission lines, coupled inductors) is listed
in the report and left out of the simulation.

### Without a window

```sh
pcb-designer PROJECT.kicad_pro                     # opens an import at startup
pcb-designer import-kicad PROJECT.kicad_pro OUT_DIR  # imports, saves, prints the report
```

Agents use `download_kicad_libraries`, `search_kicad`, `import_kicad`,
`import_kicad_project` and `export_kicad_project` (see [Agents](agents.md)).

## Exporting to KiCad

**File → Export KiCad project…** writes the project as a KiCad 9 project (KiCad 10 opens
it too) into a new folder you name, e.g. `amp-kicad`:

- `NAME.kicad_pro` with the net classes, design rules and ERC/DRC severities;
- `NAME.kicad_sch` and a file per sub-sheet, page sizes and title block included;
- `NAME.kicad_pcb`: footprints, tracks, vias, zones, keepouts, outline, texts, stackup;
- a project library, `NAME.kicad_sym` and `NAME.pretty`, with every part and footprint
  of the Project Library, and the `sym-lib-table` and `fp-lib-table` that name it.

The board stays linked to the schematic, so KiCad's **Update PCB from Schematic** finds
nothing to change, and exporting again into the same folder replaces the files. A folder
that holds other things is refused. A report window lists what changed on the way:

- KiCad numbers a symbol's pins by its footprint's pads, so a part with several
  footprint variants becomes a symbol per variant (`Resistor_0603`, `Resistor_0805`). A
  pin on several pads becomes stacked pins, one per pad.
- Power ports become power symbols of the project library. KiCad counts them as power
  inputs, not sources: its ERC wants a `PWR_FLAG` on nets powered only by power ports.
- Nets without a label get KiCad's names (`Net-(R1-Pad2)`, `Net-(Q1-B)`).
- Zones are written unfilled: fill them in KiCad (**B**).
- The board is moved onto KiCad's page; KiCad's drill/place origin and grid origin mark
  where our origin was (importing back moves it back).
- Texts keep their size and place, but KiCad draws its own font, so their widths differ.
- Simulation models and setups aren't exported yet, nor ERC/DRC exclusions.

A library's context menu in the Project panel has **Export as KiCad library…**: its parts
(a symbol per variant, as above) and footprints as `NAME.kicad_sym` and `NAME.pretty`.

```sh
pcb-designer export-kicad PROJECT_DIR OUT_DIR   # exports, prints the report
```
