# PCB

The board follows the schematic: components placed in the schematic get their footprints
on the board (new ones in the staging area beside it), and their pads take the nets the
schematic gives them. Board edits never change connectivity. Copper whose net no longer
fits the schematic is flagged, never deleted.

## Tools

| Tool | Key |
| --- | --- |
| Select | S |
| Find and place a part ([Parts panel](libraries.md#the-parts-panel)) | P |
| Route | T |
| Via | ⇧V |
| Board outline | O |
| Zone | Z |
| Keepout | K |
| Text | ⇧T |
| Measure, clearance | M, E |

R rotates, F flips a footprint to the other side; L switches the routing layer. All keys
can be changed in **Settings → Keymap**.

## Board parts

Board parts (mounting holes, logos, fiducials) have no symbol. **Place on board** in the
Project panel (or P and the Parts panel) places them on the PCB, where R rotates and F
flips the side; Delete removes them there. Their pads have no net, so copper of a net
touching one is a short. See [Libraries](libraries.md#board-parts).

## Routing

The Route tool draws tracks leg by leg like wires (see
[Drawing wires and tracks](editing.md#drawing-wires-and-tracks)). **Edit → Round track
corners** rounds the corners between the selected tracks with the Route tool's corner
radius.

### Net classes

Net classes (Board setup → Net classes) give nets their own clearance, track width and via
size; values left at *default* are the board's (the Default class). A class lists its nets
by name or by pattern (`GND, +*V`, `/USB_D?`); a name beats a pattern, else the first class
matching wins. The Nets panel shows each net's class, and its context menu puts a net into
one. New tracks and vias take their net's class sizes (unless the Route tool has a width of
its own), and copper keeps the larger clearance of the two classes in routing checks, zone
fills and DRC. KiCad imports keep their net classes.

### Modes: shove, walk around, highlight

The Route tool has three modes (Tool settings, or **Shift+R** to cycle):

- **Shove** (the default): tracks and vias of other nets that the new leg comes too close
  to move aside, live while the pointer moves, just far enough for the clearance of the
  two nets. What they hit is pushed in turn. Pushed tracks keep their ends on their pads
  and vias and get tidy 45° corners. Moving the pointer back lets them spring back; only
  the click commits, and one click (the leg and everything it pushed) is one undo step.
  What can't move (pads, keepouts, the board edge, arcs, locked tracks and vias) the leg
  goes round. When pushing gets stuck, the leg hugs what it was pushing and pushes the
  rest.
- **Walk around**: the leg goes round obstacles on the shorter side; nothing moves.
- **Highlight collisions**: a leg that breaks the design rules is drawn in the warning
  colour with a marker where it clashes, the hint bar says why, and the click is refused.

In Shove and Walk around modes, a leg that can't get to the pointer stops at the furthest
point that works (the hint bar says what's in the way) and a click adds it up to there. In
every mode, Shift+click adds the leg as drawn anyway (Shift+L for the via while routing), and **Allow violations** in
the tool settings always does in Highlight mode; DRC then flags it.

The rules checked while routing and placing vias: clearance to other nets' pads, tracks
and vias, keepouts and the board outline.

### Drag with shove

In Shove mode, dragging tracks and vias (only those, no footprints) with the Select tool
pushes other nets' tracks and vias out of the way the same way, and they spring back as the
drag retreats; a drag that can't push further stops at the obstacle.

### Locked tracks and vias

A track or via with **Locked** set in Properties (KiCad imports keep theirs) is never
pushed: the router treats it like a pad. You can still move it by hand.

### Vias and layer changes

**L** while routing switches the layer with a via. **Via at** in the tool settings says
where:

- **At the pointer** (the default): the leg being drawn gets a via at its end that follows
  the pointer (shoving in Shove mode); the click places both and the track goes on on the
  new layer. L again picks the next layer, or none, and Esc removes the via.
- **At the start** of the leg being drawn: placed at once; the leg goes on on the new
  layer, and L again before the track goes on takes the via back.

On boards with inner layers, vias can be blind or buried: From/To layers in the Via tool
and in a via's properties, and **Via span** (through, or just the two layers) for the vias
L adds.

## Zones and keepouts

The Zone tool (**Z**) draws a copper zone corner by corner (click the first or last corner
again to close it). Its net comes from the copper under the first corner or from the tool
settings. Zones fill around other nets with the clearance and refill on every change. In
Properties, choose how pads of the zone's net connect (thermal reliefs, reliefs for
through-hole pads only, solid, or not at all), override the clearance and thermal sizes,
and choose which unconnected islands are removed (always, never, or below an area). An
empty net makes isolated copper. Zones can have holes that only that zone leaves empty.

The Keepout tool (**K**) draws an area, on all copper layers or one, that zone fills leave
out. DRC reports tracks, vias, pads and, if enabled, footprint courtyards inside it.
Properties set what it keeps out and its layers.

The Cutout tool (tool bar, no default key) cuts a polygon out of a zone or keepout: start
inside it on the active layer and click the corners. Inside, the cutout becomes a hole
(merged with holes it overlaps); across the edge, it trims the outline. A cut that would
split the area in two is refused.

Zone, keepout and outline corners are edited on the canvas: see
[Editing outlines](editing.md#editing-outlines).

## Board setup

The Board setup panel (⌘6) holds the design rules and DRC severities, net classes, solder
mask expansion and paste margin, the stack-up (thickness, copper in µm or oz, surface
finish, mask and silkscreen colours) and the layout of the assembly files (see
[Manufacturing files](manufacturing.md)).

## Checking and output

- **Run DRC** checks the design rules: see [Design checks](checks.md).
- **File → Export Gerber files** (⇧⌘E): see [Manufacturing files](manufacturing.md).
