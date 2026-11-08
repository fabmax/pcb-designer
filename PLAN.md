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
| Files | JSON (serde), pretty-printed, stable ordering, integer coordinates, `format_version` + migrations |

## Architecture (Cargo workspace)

```
crates/
  model/        Pure data: units (i64 nm), geometry, Library/Symbol/Footprint/Part,
                Schematic, Board, JSON I/O + format versioning. No GUI deps.
  netlist/      Connectivity schematic → nets (incremental), ratsnest (per-net MST)
  editor-core/  Commands (serializable), undo/redo, selection, tools as state machines,
                query API. GUI-agnostic, testable headless.
  render/       wgpu canvas renderer: camera, layers, instanced primitives, picking,
                offscreen render-to-PNG
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
