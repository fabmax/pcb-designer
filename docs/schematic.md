# Schematic

The schematic editor draws the circuit. Every change shows up on the board right away:
sync runs one way, schematic → board (see [PCB](pcb.md)).

## Tools

| Tool | Key |
| --- | --- |
| Select | S |
| Find and place a part ([Parts panel](libraries.md#the-parts-panel)) | P |
| Wire | W |
| Bus, bus entry | B, E |
| Label | L |
| Power port, power flag | ⌘P, ⇧P |
| Junction | J |
| No-connect flag | Q |
| Text | T |
| Probe ([Simulation](simulation.md)) | O |

R rotates and X mirrors what's selected or being placed; Esc cancels. All keys can be
changed in **Settings → Keymap**.

## Sheets

Sheets work as in KiCad: the Schematic node in the [Project panel](libraries.md#the-project-panel)
is the root sheet, with its sub-sheets nested below it.

- **New sheet…** (+ button, or **New sub-sheet…** on a sheet) asks for a name; then the
  new sheet's symbol follows the cursor on its parent sheet until you click.
- Double-click a sheet symbol (or the sheet in the tree) to open the sheet in its own tab.
- Rename or delete sheets with right-click; deleting takes everything on the sheet and
  its sub-sheets along (after asking). In the tree you can drag a sub-sheet onto another
  sheet.
- Reference designators are unique across all sheets.
- Hierarchical labels on a sheet become pins of its symbol on the parent sheet (inputs on
  the left, the others on the right, in name order; the symbol grows to fit them); wire
  them there like component pins. Messages lists sheet pins left unwired.

### Pages and title block

Each sheet is drawn on a page with a frame: a zone grid and a title block. With nothing
selected, Properties shows the sheet's page (A5–A0, US and ANSI sizes, custom, or None for
no frame; landscape or portrait) and the project's title block (title, date, revision,
company and a comment), which every sheet's frame shows with its page number. The comment
field wraps in the panel; only its line breaks start new lines in the frame.

## Connections

- **Wires and buses:** the Wire and Bus tools draw one leg per click; see
  [Drawing wires and tracks](editing.md#drawing-wires-and-tracks).
- **Snapping:** wires only connect where their ends meet exactly, so the tools that
  connect (wire, bus, bus entry, label, power port and flag, junction, no-connect) snap
  to what's near the pointer, within a few screen pixels at any zoom: pin ends, wire
  ends, labels, junctions and sheet pins, else onto a wire under the pointer (buses for
  the bus tools), else to the grid. That makes pins off the grid easy to wire. Alt snaps
  to the grid only.
- **Labels:** local labels connect on their sheet only (nets are named `/Sheet/NAME`),
  global and power labels everywhere.
- **Power ports** (Power tool, ⌘P) are drawn like KiCad's power symbols: pick the glyph
  (bar, arrow, circle, ground, signal ground, chassis, protective earth) in the tool
  settings or Properties, or leave it on Auto, which goes by the name (GND, AGND, … get
  ground).
- **Pins** connect only through wires; there are no hidden pins. Messages lists pins left
  unconnected; mark the ones open on purpose with a **no-connect flag** (tool bar, Q): an
  X on the pin end, which moves and is deleted with its symbol.
- **Power flags** (⇧P) say a net is powered; see [Design checks](checks.md).

## Moving objects to another sheet

**Move to sheet…** (Edit menu or the right-click menu; no default key, bind one in the
settings) moves the selected objects to another sheet you pick from a list. They keep
their identity (references, footprints and routing stay), take their reference and value
texts and no-connect flags along, and follow the pointer on the new sheet until you click
(Esc undoes the move). Wires you didn't select stay, so the status bar tells how many nets
were renamed by the move. A moved sheet symbol takes its sheet along as a sub-sheet of the
target.

## Components

- Place parts from the [Parts panel](libraries.md#the-parts-panel) (**P**).
- **Footprint variants** can be switched in a symbol's Properties (and a footprint's on the
  board).
- **Parts without a footprint** (power flags, logos): components whose variant has the
  footprint "None (not on board)" stay in the schematic and its netlist but have no
  footprint on the board.
- **Do not populate** (Properties) keeps a component off the BOM and position files; the
  schematic crosses it out and both editors draw it faded.
- **Reference and value texts:** see [Editing](editing.md#reference-and-value-texts).

## Output

**File → Export schematic PDF**: see [Manufacturing files](manufacturing.md#schematic-pdf).
