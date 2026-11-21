# PCB Designer – Push-and-shove routing plan

The plan for interactive push-and-shove routing on the board: how it should behave, how
it fits the code we have, and the milestones to get there. Once a milestone is built,
its design moves into PLAN.md (the "Connectivity, copper, zones, DRC" and "Views and
editing" sections) and this file keeps only what is still ahead.

## Goals

- Route a track through a crowd: tracks and vias of other nets that are in the way move
  aside, live while the pointer moves, keeping clearance. What can't move (pads,
  keepouts, the board edge, parts that are locked) stops the route instead.
- Spring back: when the pointer retreats, the tracks that were pushed return to where
  they were. Only a click commits the shove.
- One click is one undo step: the new leg and everything it pushed.
- What gets pushed stays tidy: 45° corners, no tiny segments or loops, and a pushed
  track stays connected to its pads and vias.
- Dragging a track segment or a via with the Select tool shoves too.
- The same router for agents: MCP's `add_track` can shove and reports what moved.
- Within the performance budget: a shove step within a frame (target < 4 ms) on a
  dense board, with a time limit that returns the best partial result.

Not goals for the first version: diff pairs, length tuning, autorouting, shoving arc
tracks (they are fixed obstacles), shoving on zone fills (fills refill afterwards).

## What other tools do

- **KiCad** (the PNS router, `pcbnew/router`): three modes, *Highlight collisions*,
  *Shove* and *Walk around*, cycled in the router settings. Its model: items get
  *hulls* (an octagon around a segment or via, a chamfered polygon around a pad) grown by
  clearance plus half the width of the line being moved; a *line* is the chain of
  segments of one net between joints, and lines are shoved as a whole by walking them
  around the pusher's hull. Pushed lines push others (a shove stack with an iteration
  limit). Vias are shoved by a translation and drag their lines along. When a shove
  fails, the head stops at the last point that works. Pushed lines are smoothed by an
  optimizer (merging segments, cutting corners where it is clear). Spring-back comes from
  recomputing from the original board on every move.
- **Altium**: Push / Walkaround / Hug and push / Ignore / Stop at first obstacle; the
  same ideas, plus "gloss" (smoothing) levels.
- **Others** (EasyEDA, Eagle, Fusion): mostly walkaround or none.

We follow KiCad's model: it is proven, its behaviour is what KiCad users expect, and it
maps cleanly onto our Route tool.

## Behaviour

### Modes

The Route tool gets a **mode** in Tool settings (and a key to cycle it, shown in the
hint bar):

- **Highlight**: today's behaviour. A clash turns the ghost to the flagged colour and
  refuses the click (Shift+click or "Allow violations" adds it anyway).
- **Shove** (the default from R2 on): other nets' tracks and vias move aside.
- **Walk around**: the leg goes around obstacles; nothing moves.

Shift (force) routes straight through in every mode: the leg as drawn, added anyway,
DRC flags it. "Allow violations" only matters in Highlight mode (the router never adds
a violation).

### Shove, step by step

1. The leg (the head) is laid out from the last click to the pointer as today
   (`bend::segments`, posture, corner style).
2. Tracks and vias of other nets that the head comes too close to are pushed: each is
   moved to the side away from the head, just far enough for the clearance of the two
   nets (net classes included). What they hit is pushed in turn.
3. Fixed: pads, keepouts that forbid tracks or vias, the board edge, locked footprints'
   copper, locked tracks and vias, arcs, and copper of the routed net. The start of each pushed line stays on its
   pad or via: only its inner corners move, and extra corners appear as needed.
4. If the head can't get to the pointer (a pad in the way, a pushed track squeezed
   against the board edge, the time limit), it stops at the furthest point that works.
   The ghost shows that; a click commits up to there.
5. Moving the pointer back recomputes from the board as it was at the last click, so
   pushed tracks spring back.
6. L (layer change and via) while routing: the via shoves too.
7. Click: commits the leg and all pushed copper as one undo group. Esc: drops the leg,
   nothing was pushed.

Pushed tracks are shown live as the real board (they are real edits, retracted every
step, see below), so the view, the ratsnest and the clash marker are exact.

### Walk around

The head follows the outline of the obstacles' hulls, going round each on the shorter
side (both are tried, the shorter wins; `/` could flip it later). If it can't reach the
pointer, it stops as in shove mode.

### Drag with shove (Select tool)

Dragging a straight track segment (`Target::drag_axis`, it moves across itself) or a via
pushes what's in the way, with the same engine: the dragged segment or via is the
pusher, its neighbours of the same net follow as today (`track_band`). It follows the
Route tool's mode (Highlight: no shove; Walk around: no shove either, as a drag has no
path to bend), so there's one switch.

### Locked tracks and vias

`Track` and `Via` get `locked` (as footprints have): a locked one is a fixed obstacle for
the router, and Properties, MCP and KiCad import (`(locked yes)`) set it. Moving a locked
track or via by hand stays possible, as for footprints.

## Implementation

### Where things go

- `editor-core/src/router/` (new, pure and headless, unit-tested):
  - `world.rs`: the obstacle world of one layer set: every copper item as a `Geo` with
    its net, layers and whether it can move, plus keepouts and the board edge, indexed
    with an `rstar` R-tree per copper layer. Built from `Project` + `BoardConnectivity`
    (nets come from islands, as tracks store none) and cached by document revision.
  - `hull.rs`: hulls. A segment's octagon, a via's octagon, a pad's convex hull
    chamfered (polygon pads: their convex hull), each grown by the clearance between the
    two nets plus the half width of the moving line. Integer nm, 45°-friendly.
  - `line.rs`: a `Line` is the chain of straight tracks of one net on one layer between
    joints (pads, vias, tees, arcs, ends), assembled from the model like
    `query::track_chain`, with the ids of its tracks. `walkaround(line, hull, side)`
    gives the line going round a hull; the core operation of both modes.
  - `shove.rs`: the shove engine. Input: the world, the head (a line and optional via).
    A stack of pushers: the head first; every line or via it collides with is shoved
    (lines walked around the pusher's hull on the side they're on; vias translated by the
    smallest vector that clears it, dragging their lines' ends along, on all their
    layers) and becomes a pusher itself. Limits on iterations and time. Fails when a
    pushed item would have to cross a fixed one. Output: a `Shove { head, changed:
    Vec<Line>, moved_vias }`.
  - `optimize.rs`: tidies changed lines inside the area that changed: merge collinear
    segments, drop zero-length ones and loops, replace two corners by a 45° shortcut when
    it stays clear.
  - `route.rs`: the entry point both the tool and MCP use: `route(project, copper,
    request) -> Outcome`, where the request is the leg (points, width, layer, net, via)
    and the mode; the outcome is what to add, update and delete, and where the head got
    to. Truncation: if the full head fails, a binary search along the head for the
    furthest point that succeeds (a handful of shove runs).
- `editor-core/src/command.rs`: `AddTrack` (and `AddVia`) get a `mode` field (`highlight`
  default, `shove`, `walkaround`). The command runs `route::route` and applies the
  outcome in its transaction: new tracks for the head, `Update` of the pushed tracks and
  vias (keeping their ids, so undo, flags and selection behave), new tracks where a
  pushed line got more segments, deletes where it got fewer. The head's actual end is
  returned in the change set (or a small result type) so the tool knows where the leg
  stopped. Every edit stays a `Command`, so MCP and the GUI share the path.
- `view/src/pcb_tools.rs`: the Route tool keeps its leg state but, in Shove and Walk
  around modes, previews by **executing** the leg in a provisional undo group and
  retracting it on the next pointer move (`Editor::retract`), the way drags replay
  themselves. Benefits: the view redraws exactly what changed, incremental; spring-back
  comes free; the click just closes the group. Connectivity is deferred while busy, as
  for drags. Highlight mode keeps ghosts and `route_check`.
- `view/src/interaction.rs` + `document.rs`: drag with shove; a drag step is already a
  replay of one `Move`, which gets a `shove` flag (the GUI sets it from the Route mode).
- `route_check` moves onto the world's R-tree (it scans all copper per check today, a
  Later item), so Highlight mode gets faster too.
- `app`: the mode in Tool settings, a key to cycle it (keymap recipe), the hint bar text,
  and the status message when the head stops short ("blocked by a pad of GND").
- `mcp`: `add_track` and `add_via` take `mode`; the response lists the tracks and vias
  that moved. README tool list updated.

### Why the replay pattern for the preview

The alternative is ghosts: draw the pushed tracks as previews and hide their originals.
That needs a "hidden objects" layer in `View`, duplicate drawing code for every changed
item and a separate ratsnest. Replaying a command each pointer move reuses the drag path
(`execute_grouped` + `retract`), which already does about 0.2 ms per step for a
footprint with tracks. Gotcha to keep in mind: every step bumps `doc.revision`, so caches
keyed by it (the clash key, the world) must be keyed by the revision *at the leg start*,
not the current one.

### Geometry details

- Coordinates stay integer nm; walkaround builds paths from hull vertices, so results
  land on whole nm and 45° directions stay exact (an octagon's chamfers are cut at
  integer offsets).
- Clearance between two items: `Clearances::between` (net classes), the board edge
  clearance for the edge, as DRC uses. The shove leaves a few nm of slack so DRC and the
  router agree after rounding.
- Arcs: obstacles via their `Geo::Path`'s hull (chords grown by `ARC_ERROR`). Routing
  with an arc corner style shoves as if the corner were 45°, then fillets on commit (as
  `AddTrack.mids` does today); if the fillet would clash, the corner stays sharp.
- Zones: fills are not obstacles (they make room on refill); zone outlines aren't
  either. Keepouts with `no_tracks` / `no_vias` are fixed obstacles.
- Multi-layer: through-hole pads and vias are obstacles on every layer they span; a
  shoved via checks every layer it spans and can push lines on other layers.

### Tests

- `editor-core/tests/all/router.rs`: hulls (octagon of a segment, clearances by class),
  line assembly (joints at pads, vias, tees), walkaround on both sides, a single shove,
  a cascade of three parallel tracks, a shove blocked by a pad (the head stops short),
  a via shove dragging its tracks, spring-back (route then shorter: the same board),
  shoved lines keep their ids and pass DRC (no new violations), time limit returns a
  partial result.
- `view/tests/all/pcb_tools.rs`: the Route tool in shove mode: preview, click commits
  one undo step, Esc leaves the board unchanged, undo restores everything.
- `mcp/tests/all`: `add_track` with `mode: shove`.
- `edit_bench`: a shove step on the `--stress` board, measured in release.

## Milestones

### R1: Router world and walkaround (done)

Built as described in PLAN.md ("Router"): an R-tree in `BoardConnectivity` (one for all
layers, filtered by mask) instead of a separate world, hulls, `walkaround`, the mode
setting (Shift+R) and stopping short. Lines moved to R2, where shoving needs them.

### R2: Shove tracks (done)

Built as described in PLAN.md ("Shove", "Route tool and the router"). Differences from
the plan above: the router runs in `Document` (it needs the board connectivity) and its
result is ordinary commands (`AddTrack` + `SetTrackPaths`), not a `mode` on `AddTrack`;
MCP has a `route` tool instead of `add_track.mode`; the head walks round what can't be
pushed before shoving (KiCad's "walk around solids" in shove mode). The preview shows
the head as a ghost and only the pushes as edits.

### R3: Shove vias (done)

Built as described in PLAN.md ("Shove"). A pushed via moves straight away from the
pusher (no search for the smallest move), and the head never moves vias it ends on
(their island is the head's).

### R4: Drag with shove

The Select tool's track segment and via drags shove (`Move.shove`).

### R5: Polish

The optimizer's corner shortcuts (pushed lines keep the hull's shape: vertical walls and
chamfers round the head, never smoothed back), arc corner styles while shoving, the
status text naming what blocked the route, performance on deep cascades (each line is
re-pushed ~5× today: push a pusher's hits in one walk, or order the stack by distance),
an index of pushed lines.

## Later

- Diff pairs (routing and shoving them as pairs), length tuning (meanders).
- Shoving arc tracks.
- "Hug and push" / walking around in shove mode when a shove fails.
- Shoving footprints' copper (moving parts out of the way), KiCad's "smart pads"
  (choosing a pad's exit direction).
- Autorouting, which could build on the same world and walkaround.

## Decisions taken

- Shove is the default mode (from R2; KiCad's default is Highlight collisions).
- Tracks and vias get a `locked` flag; locked ones are never pushed.
- Drag with shove follows the Route tool's mode; there is no separate switch.
