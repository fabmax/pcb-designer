# PCB Designer – Implementation Plan

Living document. Requirements are in [pcb-designer.md](pcb-designer.md).

## Decisions

| Topic | Decision |
|---|---|
| UI elements | **egui / eframe** with a custom dark theme; **egui_tiles** for docking/split panes |
| Canvas rendering | **Custom wgpu renderer** embedded via egui paint callbacks |
| Schematic → PCB sync | **Always live.** Validated in the milestone-0 performance spike; we revisit if it is too slow |
| PCB → schematic | Never. PCB edits only change physical layout |
| Library parts | **Copied into the project** when placed. Each copy remembers its source library, part id and content hash, so "update from library" can detect and pull changes |
| Schematic scope v1 | One sheet, but the model holds `sheets: Vec<Sheet>`. Buses and multi-unit parts come later; the part model already has `units: Vec<Unit>` (only one unit is supported at first) |
| Copper zones + DRC | In scope |
| KiCad import | In scope (symbols `.kicad_sym`, footprints `.kicad_mod`) |
| MCP interface | In scope (see below) |
| Theme | Dark first |
| Files | JSON (serde), pretty-printed, stable ordering, integer coordinates, `format`/`version` header + migrations. **A project is a folder** (`<name>.pcbproj/` with `project.json`, `library.json`, `schematic.json`, `board.json`); a library is a single `*.pcblib` file |

## Architecture (Cargo workspace)

```
crates/
  model/        Pure data: units (i64 nm), geometry, Library/Symbol/Footprint/Part,
                Schematic, Board, JSON I/O + format versioning. No GUI deps.
  netlist/      Connectivity schematic → nets (incremental), ratsnest (per-net MST)
  editor-core/  Commands (serializable), undo/redo, selection, tools as state machines,
                query API. GUI-agnostic, testable headless.
  render/       wgpu canvas renderer: camera, layers, instanced primitives,
                offscreen render-to-PNG
  view/         Model → scene drawing (incremental), spatial index + hit testing,
                Document (editor + views), Interaction (selection/drag/box/keys).
                GUI-toolkit independent and unit tested.
  mcp/          MCP server exposing editor-core commands + queries
  kicad-import/ S-expression parser + converters for KiCad symbols/footprints
  app/          egui shell: docking, menus, panels, dialogs
```

The symbol editor is a small schematic editor and the footprint editor is a small PCB
editor, so there is **one generic canvas editor framework** (camera, grid, snapping,
selection, tools, undo) reused by all four editing surfaces.

## Rendering

- Internal coordinates: integer nanometers (i64). The GPU receives f32 values relative to
  a chunk/camera origin, so precision does not suffer on large boards.
- Primitives drawn as **instanced shapes with exact edges at any zoom (signed distance
  fields)**: capsule (line/track with width), arc, circle/ring, rounded rect. Polygons
  (zones, custom pads) are tessellated with lyon.
- Text: **stroke (vector) font**, which turns into capsules. It reuses the same pipeline
  and is directly usable for Gerber later.
- Per-layer GPU buffers, updated only where something changed. Panning and zooming only
  change a uniform. Small details (text, pad numbers) are hidden when zoomed far out.
- Offscreen rendering to PNG, used for visual tests, the MCP screenshot tool and
  development checks.

## Live sync

There is one in-memory `Project`. Each schematic command marks what it touched, which
triggers this chain:

1. An incremental connectivity update (union-find over pins, wires and labels).
2. A recomputed set of affected nets.
3. A board update:
   - A new component spawns its footprint in a staging area next to the board.
   - A removed component removes its footprint.
   - A changed footprint variant swaps the footprint and keeps its position.
4. A per-net ratsnest update for affected nets only.
5. A GPU buffer update for touched items only.

Tracks and vias whose net no longer matches are **flagged, not deleted**.
Cross-probing: selecting a part in one editor highlights it in the other.

Undo/redo uses one stack per project, so undoing a schematic change also reverts its
effect on the board.

## MCP interface

- The running app hosts an **MCP server (Streamable HTTP on 127.0.0.1, opt-in in the
  settings)** using the official Rust SDK (`rmcp`). Agents operate on the open project,
  and the user watches the changes live.
- A headless mode (`pcb-designer --mcp-stdio project.pcbproj`) for agents without a GUI is
  added later.
- **Every MCP tool goes through the editor-core command API**, the same path as the GUI.
  Undo, live sync and validation therefore work the same way for agents and humans.
- Threading: the server runs on a tokio runtime. Requests travel over a channel to the app
  thread, are applied between frames, and the reply is sent back. The server then calls
  `request_repaint()`.
- Tool groups:
  - **Query:** project summary, components, nets, unrouted connections, DRC results.
  - **Schematic edits:** place component, wire/connect pins, labels, set properties.
  - **Board edits:** move/rotate footprints, tracks, vias, zones.
  - **Libraries:** search parts, create parts, import KiCad files, update from library.
  - **View:** render a region of the schematic or PCB to PNG.
  - **File:** save.
- The MCP foundation (server, screenshot, query, generic command passthrough) comes early,
  because it also helps develop and test the editors. Each editor milestone then adds its
  own tools.

## Milestones

| # | Milestone | Done when… |
|---|---|---|
| 0 | **Spikes.** Workspace, eframe + wgpu window, dark theme, docking, canvas with pan/zoom/grid. (a) Rendering performance test. (b) Live-sync performance test on a synthetic large project. Offscreen PNG rendering | ≥60 fps pan/zoom with ~10k footprints / 100k+ primitives. Schematic edit → nets → ratsnest → GPU buffers within one frame (<16 ms) at that size |
| 1 | Core model, units, geometry, JSON save/load + versioning, command/undo framework, query API | Round-trip and undo tests pass |
| 2 | Generic canvas editor framework: selection, move/rotate/mirror, snapping, property panel | Used by everything below |
| 3 | MCP foundation: server in app, threading, screenshot, query, command passthrough | An agent can inspect the app and take screenshots |
| 4 | **Part editor:** symbol editor, footprint editor, pin↔pad mapping UI, library bundles, update-from-library flow, small starter library | A part can be built from scratch and updated in a project |
| 5 | **KiCad import** of symbols and footprints | Standard KiCad libraries import correctly |
| 6 | **Schematic editor:** place parts, wires, junctions, labels, power symbols, connectivity | Correct netlist for sample circuits |
| 7 | **PCB editor:** outline, layer stack (N layers, 2 by default), live footprint sync, ratsnest, manual routing (45°/90°), vias, net highlighting | A small board can be fully routed |
| 8 | Sync edge cases (flagged tracks, swaps), cross-probing | Schematic edits update the PCB correctly in all cases |
| 9 | **Copper zones** (fill with clearance, thermal reliefs) + **DRC** (clearance, unrouted, overlaps) | Zones fill correctly; DRC finds seeded errors |
| 10 | UX polish, theming, headless MCP mode | |

**Later:** multi-sheet/hierarchical schematics, buses, multi-unit parts, Gerber export,
autoplacement, autorouting, simulation.

## Milestone 0 results (2026-11-08, Apple M4, release build)

Synthetic project: 10,000 components, 88k pins, 67k wires, 47k labels, 31.8k nets
(largest: GND with 7,110 pins), 25k tracks, 5k vias → 2.5 M schematic and 0.5 M PCB
render primitives.

**Live sync** (`cargo run --release -p spike --bin sync_bench`), schematic edit → nets →
footprints → ratsnest → scene diff:

| | p50 | p95 | max |
|---|---|---|---|
| All edit kinds (600 random edits) | 2.5 ms | 2.9 ms | 3.6 ms |
| Edits not touching GND | 0.01–0.2 ms | | |
| GPU diff per edit | 0.005 ms | 0.11 ms | 0.46 ms (≤ 2.3 MB) |

- Connectivity is incremental. Additions merge nets (smaller into larger). Removals run an
  interleaved split search, so cost depends on the smaller side. Edits touching GND
  dropped from 8 ms (re-flood) to about 0.2 ms.
- The worst case is now the ratsnest of the 7k-pad GND net (Delaunay + Kruskal, about
  2.3 ms). Options for later: incremental MST, or computing very large nets on a worker
  thread.
- **Conclusion: always-live sync works with a comfortable margin.**

**Rendering** (`pcb-designer --components 10000 --no-vsync --animate`): both canvases
animated at once (about 3 M primitives), 90–170 fps uncapped. There is no culling yet;
fully zoomed-out views are the slowest. Possible improvements: per-chunk viewport culling
and level of detail for tiny primitives.

**Open items from the spike**
- The spike's copper islands are known by construction. The real board needs geometric
  copper connectivity (M7).
- The placeholder stroke font needs replacing (moved to M10; it is readable enough until then).
- Same-layer transparency (render each layer to a texture, then composite) is still open.

## Milestone 1 design notes (2026-11-08)

- **`model` crate**
  - Integer-nanometer `Point`/`Size`, which serialize as `[x, y]`.
  - `Angle` in degrees, normalized; multiples of 90° rotate exactly.
  - `Transform` applies mirror, then rotation, then translation.
  - `Shape` and stroke-font `Text`.
  - Every object has a UUID `Id`.
  - `Store<T>` is an insertion-ordered map, saved as a JSON array. Removing an object and
    re-inserting it at its old index restores the order exactly.
- **Library**
  - `Symbol` pins have names. `Footprint` pads have numbers.
  - `Part` = units (one symbol per unit) + variants (footprint + pad → `(unit, pin)`
    mapping). A pin can have several pads.
  - Embedded parts keep `source` provenance (library id, part id, content hash) for
    "update from library".
- **Schematic**
  - Design-wide `Component`s (refdes, value, part, variant).
  - Each unit is placed as a `SymbolInstance`. Every graphical object carries its sheet
    id, so multi-sheet support only needs more sheets.
- **Board**
  - `FootprintInstance` stores only the placement of a component. The footprint is
    derived from the component's variant, so swapping the variant needs no board edit.
  - Layer ids: copper 0 (top) … 63 (bottom), technical layers from 100.
- **`editor-core` crate**
  - Serializable `Command`s (JSON with a `"command"` tag) are the only way to modify a
    project. The same commands will back the GUI tools and MCP.
  - Each command runs in a `Transaction` that records object-level before/after
    snapshots. Commands are atomic: on error the transaction rolls back.
  - Schematic → board sync runs inside the same transaction, so one undo step reverts
    both. Undo/redo replays the change sets; tests check byte-identical files after
    undo all / redo all.
  - The returned `ChangeSet` tells consumers what changed (input for incremental
    connectivity and render updates).
- **Built-in starter library** (fixed ids): resistor (0603/0805), capacitor, LED
  (pins A/K → pads 2/1), 1x4 header.
- **Deferred:**
  - Multi-unit placement (only unit 0 is placed).
  - Flagging tracks whose net changed: needs board connectivity (M7).
  - Performance of linear scans (refdes numbering, sync lookups): fine for now, revisit
    with 10k-part projects.

## Milestone 2 results (2026-11-08)

- **`view` crate**
  - `View`: draws the schematic or board into a render `Scene` and updates it per
    `ChangeSet`. Component edits redraw their symbols/footprint; library edits rebuild.
    Selection/hover overlays are separate scene items. An R*-tree over object bounds
    does hit testing (thin objects beat bodies, smaller bodies beat larger ones) and box
    selection.
  - `Document`: editor + folder path + both views, kept in sync on execute/undo/redo.
  - `Interaction`: click / shift / cmd selection, box select (left→right inside,
    right→left crossing), hover, drag with grid snapping of the grabbed object's
    reference point (one undo step per drag; R rotates while dragging; Esc cancels it),
    rotate / mirror / flip / delete / select all. Locked footprints don't move;
    deleting footprints is explained (they follow the schematic).
  - Demo project and `stress_project(n)` generators.
- **App** now edits real projects: File New / Open / Save / Save As (project folders),
  unsaved-changes prompt (also on window close), undo/redo with labels, library panel
  with Place, properties panel (refdes, value, footprint variant, position, rotation,
  mirror, side, locked, track width/layer, via sizes, labels, zones, texts; grouped
  undo for drags), per-view layer toggles and grid presets, status bar.
  `pcb-designer [DIR | --demo | --new | --stress N]`.
- **Performance** (`cargo run -p view --release --example edit_bench`, 10k components,
  0.58 M primitives per view):
  - Generate + build both views: 0.42 s.
  - Dragging a symbol: 0.035 ms per frame (command + both views). Hit test: 0.2 µs.
  - Rotate all 20k schematic objects: 73 ms (editor 7.5 ms after making in-place
    updates O(1); it was 580 ms).
  - Delete all (30k objects incl. footprints): 0.42 s. `Store` removals shift the index
    map (O(n) each); revisit if bulk deletes matter.
- **Panels** keep their width (scroll areas fill the panel instead of shrinking to their
  content), so the canvases no longer jump when the selection changes.
- **Key bindings** go through a `Keymap` (action → shortcuts, exact modifier matching,
  JSON like `{"rotate": ["Space"]}` on top of the defaults, conflict detection). Menus
  show the current bindings; Help → Keyboard shortcuts lists them.
  `pcb-designer --keymap FILE.json` loads custom ones. A settings UI to edit and persist
  them comes with M10.
- **Deferred:** filled polygons in the renderer (zones/polygons draw as outlines),
  zone fill display (M9), text editing on canvas, the spike-based benchmark app mode
  (the `spike` crate is kept for `sync_bench`).
