# Libraries and parts

A **part** is what you place: a symbol (one or more units) and its footprint variants,
plus fields such as an MPN and an optional [simulation model](simulation.md#models-of-your-parts).
Parts live in libraries; the design uses copies of them in its Project Library.

## The Project panel

The Project panel (⌘1) shows a tree, IntelliJ style:

- **The project:** the Schematic with its [sheets](schematic.md#sheets) and the PCB
  (double-click to show them; their tabs can be closed like any other, and the tree opens
  them again), the [simulations](simulation.md#simulations-in-the-project-panel), the
  **Project Library**, and the [outputs](manufacturing.md#outputs-in-the-project-panel).
- **Built-in Library:** generic parts that ship with the app (below).
- **External Libraries:** the project's library files (`.pcblib`).

Libraries expand to Parts, Symbols and Footprints. A click (or the arrow keys) shows an
item in the preview tab (in italics), which the next item replaces; double-click (or
Enter) opens it in a tab of its own, as does editing it or double-clicking the preview's
tab. Past the most library tabs (Settings → Libraries, 10 by default) the tabs used
longest ago close; nothing is lost, edits stay in the library until it's saved.
Right-click for more: place a part, copy it to the Project Library (where it can be
changed for this design only), create, duplicate, rename or delete items, and save, reveal
or remove libraries.

- **Keys:** ↑/↓ move, ←/→ collapse and expand, P places the selected part, Delete deletes
  an item, a sheet or a simulation.
- **Search:** type, or click the search button, to filter the tree. ↑/↓ step through the
  matches and Esc ends the search.
- **+ button:** New sheet…, Add simulation, New library… (asks for its file right away), Add library…,
  Import KiCad parts… (the library entries are also on right-click of External Libraries
  and in the File menu).

## The Project Library

The Project Library holds the project's own copies of the parts the design uses. It is
editable like any library, in its own tab: edits are project changes, so they share the
project's undo history and update the schematic and board right away. Each part shows its
state:

- **library changed**: Update takes the new version.
- **modified**: edited here.
- **both changed**: Update asks first, because it drops the local edit.
- **library not loaded**, **removed from library**, or **local** (made here).

Parts nothing uses are dimmed, and **Remove unused parts** (right-click the Project
Library) deletes them with the symbols and footprints only they needed. A part, unit or
variant in use can't be removed, and neither can a symbol or footprint a used part needs.

## The Built-in Library

A standard library of generic parts: resistors, capacitors (also polarised), inductors,
ferrite beads, fuses, diodes (Schottky, Zener, TVS), LEDs, NPN/PNP transistors, N/P
MOSFETs, crystals, test points, a push button, slide switches (on/off and changeover), a
trimmer potentiometer, connectors `Conn_01x01`…`Conn_01x40` and
`Conn_02x01`…`Conn_02x20` (pin headers and sockets; single rows also JST PH/XH/SH and
screw terminals), a few ICs (op amps, regulators, 74HC logic: the 74HC00, 02, 04, 08, 14,
32 and 86 gates, the 74HC74 flip-flop and the 74HC595 shift register), board parts (mounting holes M2
to M4, a fiducial) and simulation sources. Each part offers its common packages, SMD and
through-hole, as footprint variants; set the value per component. Every part has a
simulation model.

It is read-only; **Save a copy as…** makes an editable library of your own from it.

## External libraries

The project lists its library files in `project.pcbproj`, relative to the project folder, and
they load with the project. A file that can't be read shows as missing, with Locate… and
Remove. Adding a copy of a library that is already there is refused, because a copied file
keeps the library's id. KiCad's libraries can be imported too: see
[KiCad](kicad.md#kicad-libraries).

## Categories

Parts, symbols and footprints can have a category: plain text such as "Passives" or "SMD",
set in the part form or in a symbol's or footprint's Properties (with nothing selected).
The ▼ menu next to the field lists the categories already in use. Each library's Parts,
Symbols and Footprints show a folder per category, followed by the items without one.
Right-click a folder → **Rename category…** to rename it on all of its items in one step
(an empty name removes the category). The tree search also finds the items of a category
by its name. The Built-in Library's parts are sorted into categories, and its footprints
into "SMD", "Through hole" and "Mechanical". KiCad imports have no category.

## The Parts panel

The Parts panel (⌘0) lists the parts of the Project Library, the loaded libraries and the
Built-in Library, grouped by library and category: schematic parts while the schematic is
the active editor, board parts while the board is. Categories start closed; click a
library or category row to open or close it.

- **Search** matches every word in the name, category, description and properties (an
  MPN, say), and shows every category with matches open.
- Click a part to preview its symbol or footprint; double-click it (or Enter) to place it,
  copying it into the Project Library first.
- **P** on the schematic or the board opens the panel with the search field focused:
  type, ↓, Enter, then click on the canvas. Esc clears the search, a second Esc goes back
  to the canvas.
- Its context menu also opens a part in its library or copies it to the Project Library.
- A link under the search field counts the matches in the KiCad libraries ("79 matches in
  the KiCad libraries", or "Import KiCad libraries" without a search); clicking it opens
  the [KiCad import](kicad.md#kicad-libraries) with the same search.

## Kinds of parts

- **Footprint variants:** a part offers one or more footprints; switch them in Properties
  for a symbol in the schematic or a footprint on the board (the board lists only
  variants with a footprint).
- **Parts without a footprint** (power flags, logos): in the part form, a variant's
  footprint can be "None (not on board)". Components using such a variant stay in the
  schematic and its netlist but have no footprint on the board. A part can offer it next
  to real footprints; switching a component to a variant with a footprint places it in
  the staging area beside the board.
- **Board parts** are described next.

### Board parts

Board parts (mounting holes, logos, fiducials) have no symbol: a part without units. They
show a mounting-hole icon in the tree, and **Place on board** (or P) places them on the
PCB. Their pads have no net. Copy, paste and ⌘D work on them on the board, and Properties
edits their reference and value (as for every footprint). The Built-in Library has
mounting holes (M2 to M4) and a fiducial. Remove a part's last unit in the part form to
make one.

## Part editors

Symbols and footprints open in their own editor tabs; parts in a form (units, pins to pads
per footprint variant, fields, category, simulation model).

The symbol and footprint editors draw lines (**L**), rectangles (**B**), circles (**C**),
arcs (**A**: click the centre, the start, then the end follows the pointer; **/** goes
round the other way) and polygons (**G**: click the corners, close on the first or last
one), plus pins or pads (**P**) and texts (**T**). Drawing snaps to the grid and to pins,
pads and corners; Alt snaps to the grid only. A selected drawing has the same handles as
board drawings ([Editing outlines](editing.md#editing-outlines)).

**Create array…** (⇧⌘D, right-click or Edit menu) copies the one selected pad or pin: in a
line, two rows (SOIC, DIP; numbered counterclockwise), a grid (BGA; rows A, B, … without I,
O, Q, S, X, Z) or four sides (QFP, QFN; the end pads turned by 90°). The selected pad is
the first and numbers go on from its own. The settings are in the Tool settings panel and
the copies show on the canvas while you set them up; Enter (or Create) makes them, Esc
cancels. Names that are taken already are listed but allowed. Pins make lines only.

The footprint editor shows each pad's solder mask opening and paste on the Solder mask and
Paste layers. A pad's Properties have Solder mask and Solder paste rows: the checkbox turns
the opening off, and the margin overrides the board's rule for this pad (it shows the rule
until you type, ↺ goes back to it). In the Project Library the project's rules apply,
elsewhere the defaults; board rules apply when the part is placed.

Agents can make parts too, e.g. from a datasheet: see [Agents](agents.md#library-tools).
