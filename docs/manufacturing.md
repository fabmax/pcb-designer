# Manufacturing files

Everything is written into the project's `output/` folder. It is generated, so you may want
it in `.gitignore`.

## Gerber and drill files

**File → Export Gerber files** (⇧⌘E) writes into `<project>.pcbproj/output/gerber/`,
replacing the previous export:

- Gerber X2 files: copper, solder mask, paste, silkscreen, board outline. Copper carries
  net, pad and component attributes.
- Excellon drill files, one more per blind or buried via span (e.g. `Demo-front-in1.drl`).
- A Gerber job file (`Demo-job.gbrjob`) with the board's stack-up (Board setup →
  Stack-up: thickness, copper in µm or oz, surface finish, mask and silkscreen colours).
- `output/<name>-gerber.zip` with all of them, for the manufacturer.

Zones are refilled and the design rules checked first; violations are reported but don't
stop the export. Solder mask openings and paste come from the pads (Board setup: mask
expansion, paste margin; pads can override them in the footprint editor).

## Assembly files

The export also writes, into `output/assembly/`:

- a bill of materials (`Demo-bom.csv`), grouped by value, footprint and properties such as
  an MPN;
- component positions (`Demo-positions.csv`): board mm, rotation, side.

Board parts (mounting holes, logos) are left out, and so are components marked **Do not
populate** in Properties (their pads stay on the board; the schematic crosses them out and
both editors draw them faded).

**Board setup → Assembly files** lays both out for an assembly service:

- **Generic:** a column per property.
- **JLCPCB:** `Comment, Designator, Footprint, LCSC Part #`, the number from an `LCSC`
  property; positions `Designator, Mid X, Mid Y, Layer, Rotation`.
- **PCBWay:** manufacturer and part number from `Manufacturer` and `MPN`, SMD or THT;
  positions with the value as comment.

## Schematic PDF

**File → Export schematic PDF** writes `output/<name>-schematic.pdf`: a page per sheet
(root first, each followed by its sub-sheets) the size of its page, with frame and title
block, in the light colours, bookmarked by sheet path. Text is drawn as outlines (not
selectable).

## Outputs in the Project panel

The Project panel lists the files under Outputs › Gerber and Outputs › Assembly (marked
*outdated* when the design changed since). The job file and the CSV files open in your
system's app, and so does Outputs › Schematic PDF (double-click; it exports the PDF when
there is none yet).

Double-clicking the Gerber files opens them, read back from disk, in a read-only Gerber
tab, stacked like the board in its colours (solder mask inverted, drills as holes).
Double-clicking a single file shows only that one, the layers button toggles them, and
hovering names the file, aperture and its function, and the pad and net. The Measure tool
works there too. Open the PCB tab side by side to compare. Agents can look too:
`screenshot` with target `gerber`.

## Without a window

```sh
pcb-designer export-gerber PROJECT_DIR [--assembly generic|jlcpcb|pcbway]
pcb-designer export-schematic-pdf PROJECT_DIR
```

See [Command line](command-line.md).
