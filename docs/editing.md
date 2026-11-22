# Editing

What works the same way in the schematic, on the board and, where it applies, in the
part editors.

## Drawing wires and tracks

The Wire, Bus and Route tools draw one leg per click. Each leg bends once: the first part
follows the way the pointer leaves the start point (horizontal or vertical, or straight or
diagonal at 45°); move back to the start to choose again, or press **/** to flip it. The
tool settings fix it if you prefer.

Corner styles (tool settings, or Shift+Space to cycle): 90°, 45° and free angle for wires
and buses; 45°, 90°, arc 45° and arc 90° for tracks (the arc styles round the corner as
widely as the legs allow).

Routing has more to it (design rules, vias, push and shove): see
[PCB](pcb.md#routing).

Schematic wires and buses tidy themselves after every edit:

- A wire ending on another wire's middle, or a junction placed on a wire, splits that
  wire there, so each piece ends where something connects.
- Wires in a straight line that meet end to end, with nothing else at the joint (no other
  wire, pin, label or junction), become one wire.
- A wire drawn on top of another one merges with it into a single wire.

## Selecting and dragging

Dragging keeps things connected, in the schematic and on the board:

- Wires attached to a moved symbol follow it, and horizontal and vertical wires stay so.
  A wire end that sat on something that stays put (a pin, a label, a branch) stays there,
  with a short wire joining it.
- A single horizontal or vertical wire segment only moves across itself, and its
  neighbours stretch.
- On the board, tracks follow a moved footprint or via, and a dragged track segment moves
  across itself; track ends slide along the next track to keep 45° and 90° corners.
  Tracks in the way are pushed aside ([Drag with shove](pcb.md#drag-with-shove)).
- Wires a drag leaves end to end in a straight line become one, and a wire dropped onto
  another one's middle splits it.
- Esc cancels a drag.

Double-click a wire to select its whole run up to branches and pins, or a track up to
pads, vias and branches; with ⇧ or ⌘ it's added to the selection.

Right-click the canvas for a menu with the common actions.

## Copy and paste

⌘C / ⌘X / ⌘V (Ctrl on Windows and Linux) copy, cut and paste the selection; ⌘D duplicates
it without touching the clipboard. A paste follows the pointer (R rotates, X mirrors)
until you click; Esc cancels it.

- **Schematic:** symbols, wires, buses, labels, texts, sheet symbols; across sheets, and
  into other projects and app instances, since clips carry their parts. A copied sheet
  symbol copies its whole sheet with its sub-sheets, as an independent copy named amp2,
  amp3, …; their components get new references.
- **Board:** tracks, vias, zones, keepouts, graphics, texts, and board parts such as
  mounting holes; they keep their layers. Footprints of schematic parts aren't copied on
  the board: copy their symbols in the schematic.
- **Symbol and footprint editors:** pins, pads, graphics, texts; pins and pads get the
  next free numbers.

Pasted components get the next free reference unless theirs is free (after a cut). Clips
are JSON text on the system clipboard (clips of older versions are refused).

## Texts

The Text tool (schematic **T**, board **⇧T**) places a free text: click where it goes and
type. Its size, alignment and rotation (R) are in the tool settings; on the board also its
layer (Top Silkscreen by default; F switches the side) and weight (Bold by default, as thin
silkscreen doesn't fabricate well). Texts on copper have no net.

Double-click a text to edit it in place: free texts, labels, sheet names and reference and
value texts. Arrows, Home/End, ⌥ (Ctrl) for words, ⌘ for lines, ⇧ to select, ⌘A, the
clipboard, and clicks, drags and double-clicks on the text work as in a text field.
Shift+Enter starts a new line in a free text, Enter finishes as one undo step, Esc
cancels, and clicking elsewhere or switching tools finishes too. An emptied free text is
deleted; labels and references can't be empty, and a reference already used is refused.
Properties edits free texts in a multi-line field. Footprint texts stay upright, in the
Gerber files too.

Texts are set in [JetBrains Mono](https://www.jetbrains.com/lp/mono/) (Regular and Bold).
The text size is the height of capitals; KiCad imports keep their sizes but not their
exact widths (KiCad's own font differs). In the Gerber files texts are filled regions.

### Reference and value texts

Reference and value texts belong to each symbol in the schematic and each footprint on the
board: click one to select it, drag it away, R turns it; it moves (and on the board flips)
with its owner, which is highlighted with a line to the text while it is selected. They
stay upright whatever the owner's rotation. Properties sets the size, offset and rotation
(and on the board the layer), hides it, and **Reset to library position** puts it back
(the owner's Properties have a visibility checkbox per text). Library changes don't move
placed texts.

## Editing outlines

Select one zone, keepout or board drawing (or a drawing in a part editor) to edit its
corners on the canvas:

- Drag a corner, drag an edge's midpoint to add a corner, and Delete (or double-click) a
  corner under the pointer to remove it.
- Corners snap to the grid and to pads, track ends and other corners; Alt drags freely,
  Shift keeps the edges at 0°, 45° or 90°.
- A rectangle (like the board outline) resizes by its corners and edges; right-click →
  **Convert to polygon** makes its corners editable one by one.
- A circle has a radius handle; an arc has its two ends (they slide round the circle) and
  a midpoint that changes the radius keeping the ends in place.

## Measuring

The layout editors (PCB, the part editor's footprint, the Gerber tab) have a Measure tool
(ruler button, **M**): click a start point, the ruler follows the pointer with its length,
ΔX, ΔY and angle in the current unit, a second click pins it, the next click starts a new
one. Ends snap to pad and via centres, track ends and corners (in the Gerber tab: flash
centres and line ends), from anywhere inside a pad, via or flash to its centre, else to
the grid; hold Alt for a free point. ⌘C (Ctrl+C) copies the reading as text. Esc clears
the measurement, a second Esc returns to Select.

On the board, **E** (or Tool settings) switches the Measure tool to clearances: click two
objects (copper on the active layer, a drill hole or the board edge) and it shows the gap
between their edges, with the clearance the design rules ask for; a gap below it is drawn
in the warning colour.

## Undo

Every edit is one undo step (⌘Z, ⇧⌘Z), whether you made it or an
[agent](agents.md) did. Edits of the Project Library share the project's undo history;
edits of other libraries are undone in the library's tab.
