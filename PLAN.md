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
| Files | JSON (serde), pretty-printed, stable ordering, integer coordinates, `format`/`version` header. **No migrations until production use**: format changes may break older files (the mechanism, `model::io::MIGRATIONS`, exists but stays empty). **A project is a folder** (`<name>.pcbproj/` with `project.json`, `library.json`, `schematic.json`, `board.json`); a library is a single `*.pcblib` file |

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

- The running app hosts an **MCP server (Streamable HTTP on 127.0.0.1, opt-in via the
  Tools menu or `--mcp`)** using the official Rust SDK (`rmcp`). Agents operate on the open project,
  and the user watches the changes live.
- A headless mode (`pcb-designer --mcp-stdio project.pcbproj`) serves agents without a
  GUI (M10).
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
| 10 | UX polish, theming, headless MCP mode | An agent edits, checks and saves a project over stdio without a window; dark/light themes recolour everything and persist with key bindings and recent projects |

**Later:** multi-sheet/hierarchical schematics, buses, Gerber export,
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

## Milestone 3 results (2026-11-08)

- **`mcp` crate:** rmcp 3.9 Streamable HTTP server on its own thread (tokio, 2 workers),
  bound to 127.0.0.1. `Host` validation (loopback only, rmcp default) and Origin
  validation (any browser Origin is rejected) are on; both are covered by a test.
- **Bridge:** each tool call becomes a `Call` on a std channel and wakes the GUI
  (`request_repaint`). The app answers queued calls between frames, but not during a
  drag or box selection. Replies go back through a oneshot channel, and the tool times
  out after 60 s (for example while a modal file dialog is open).
- **`Host` trait:** what the tools need from the application (document, library,
  selection per view, view rendering, window screenshot). It is implemented by the GUI
  app and by `HeadlessHost` (tests now; headless mode in M10).
- **Tools:** `get_project`, `list_library_parts`, `import_parts`, `list_objects`
  (paginated), `get_objects`, `get_component` (pin/pad world positions,
  pin↔pad map), `execute` (batch of editor commands, one undo step, all-or-nothing;
  returns created objects), `undo`, `redo`, `get_selection`, `set_selection`,
  `screenshot` (schematic/board via offscreen render, fit or the user's current view;
  or the whole window via egui), `save_project`. Server instructions explain units,
  workflow and the schematic↔board link.
- **Schemas:** all model types and `Command` derive `JsonSchema`, so `execute`
  advertises the full command schema (about 22 KB) to agents.
- **Fixes found along the way:**
  - `Object`/`ObjectId` JSON now use a `"type"` tag. With `"kind"`, labels (which have
    their own `kind` field) produced duplicate keys. A test round-trips every object
    kind.
  - `Offscreen::render` no longer consumes the scene's incremental upload state
    (`Scene::snapshot`).
- **Verification:**
  - End-to-end test with rmcp's HTTP client against a headless host: list tools, read,
    place, failing batch rolls back, select, screenshot, undo.
  - Driving the live GUI with the `mcp_call` example: batch edits appear instantly in
    both views, window capture works.
- **Next for MCP:** net/connectivity queries (M6/M7), DRC results (M9), library
  tools (M4/M5), headless `--mcp-stdio` mode (M10), and possibly an activity log panel
  listing agent edits.

## Milestone 4 results (2026-11-08)

- **Library documents:** a library file (`.pcblib`, JSON) is edited as an ordinary
  `Document` whose project holds only the library (`view::LibraryDoc`). So symbol,
  footprint and part edits are normal `insert`/`update`/`delete` commands with undo,
  grouping and validation. Deleting a symbol or footprint that a part still uses is
  refused. Save As names the library after the file. The starter library is always open
  as a built-in library with no file; Save As turns it into a file.
- **Generic editing reused:** `Interaction` (selection, box, drag with snapping,
  rotate/mirror, delete) works on symbols and footprints through `ItemTarget` (elements:
  pins, pads, graphics and texts by index). `SpatialIndex` is generic over its key.
  `ItemView` draws one item with per-element hit testing (pins and pads first, then
  outlines, then areas), hover/selection overlays and an origin cross. Hidden pin names
  stay visible, dimmed, while editing.
- **Tools (`view::tools::Placer`):** Pin/Pad, Line (chained, end on the last point or
  Esc), Rectangle and Circle (two clicks) and Text, with a ghost preview. R rotates.
  Names auto-increment (`A9` → `A10`, `007` → `008`). Tool settings (pin length and
  type, pad shape/size/type/drill, layer, width, fill, text size) are in the right panel.
  Shortcuts S/P/L/B/C/T are in the keymap.
- **Part form:** name, prefix, default value, description; units (one symbol each);
  footprint variants with a pin↔pad mapping grid. Mapping can be automatic: "Map by
  name" (pad "1" ↔ pin "1") or "Map in order" (pin order ↔ pads in natural order:
  1, 2, …, 10). The form also has a key/value properties editor and lists validation
  problems for the part. Removing a unit renumbers the connections; changing a unit's
  symbol drops its connections.
- **Update from library:** `Command::UpdateParts` replaces a project's copy of parts by
  the library version in one undo step. Components keep their variant (matched by id,
  then by name, else the first). Symbols and footprints no longer used are removed. The
  update is refused if placed units would disappear. `query::part_statuses` compares
  content hashes (states: up to date / changed / library not open / removed from library /
  local). The project's left panel lists every open library (Place, Edit) and the
  project parts with per-part Update and Update all.
- **App:** library tabs dock next to the schematic and can be closed (with an unsaved
  changes prompt, also on quit). Focus is the project or a library tab, and the side
  panels, shortcuts, undo and Save follow it. File menu: New/Open library, Open starter
  library. Command-line options: `--library FILE` and `--show [part:|symbol:|footprint:]NAME`
  (screenshots).
- **Starter library:** added Diode (SOD-123), NPN (SOT-23) and OpAmp (SOIC-8, named pins
  on numbered pads), 7 parts in total.
- **MCP:** `list_library_parts` covers all open libraries and reports the state of each
  project part. The new `update_parts` tool updates every changed part or the given ones.
  Agents can still create parts directly in a project by inserting
  `lib_symbol`/`lib_footprint`/`lib_part`.
- **Tests:** `view/tests/part_editor.rs`:
  - Build a part from scratch with the tools, map it and use it in a project.
  - Drag and rotate a pad, then update the project.
  - Mirror, delete and undo in a symbol.
  - Create, duplicate and delete library objects.
  - Natural-order mapping.
  - Library file round trip.

  Also `editor-core/tests/library_update.rs` (variant removal, refusal to drop units),
  op amp pinout and MCP library tools.
- **Not yet:**
  - Arcs and polygons can only be edited numerically (no drawing tools yet).
  - No pad arrays or footprint wizards (KiCad import in M5 covers most needs).
  - Library editing over MCP.
  - Library rename other than by Save As.
  - A part preview in the part form.

## Milestone 5 results (2026-11-09)

- **New crate `kicad`:** a small S-expression parser plus converters for the formats of
  KiCad 6–10:
  - `.kicad_sym` libraries (many symbols per file);
  - `.kicad_symdir` folders (KiCad 10, one file per symbol; file names may differ in case
    from the symbol name);
  - `.pretty` folders of `.kicad_mod` footprints (also the legacy `module` head).
  Both flag spellings (bare `hide` and `(hide yes)`) are accepted.
- **Symbols:**
  - Sub-symbols `NAME_U_C` become one symbol per unit (named "LM358 A", …). Pins shared
    by all units go to unit A. The De Morgan body style is skipped with a note.
  - `extends` takes the parent's graphics and pins and merges the properties.
  - Power symbols are not imported (net labels play that role here).
  - Pins keep their KiCad number (new `Pin::number`; `Pin::key()` = number or name).
    Unnamed pins ("~") show their number only.
  - Graphics: rectangles, circles, arcs (three points), polylines and Béziers, filled or
    not. Symbol texts use tenths of a degree. Visible Reference and Value fields become
    `${REF}`/`${VALUE}` texts.
- **Footprints:**
  - y is flipped (KiCad footprints are y-down). Angles keep their value: both are
    counterclockwise on screen.
  - Only front layers (F.Cu/SilkS/Fab/CrtYd/Mask/Paste) are imported. Back-side and
    inner-layer graphics and keepout zones are skipped with notes.
  - Pad shapes: roundrect, chamfered (as a polygon), trapezoid (as a polygon), oval,
    circle, rect.
  - Custom pads: a single polygon that contains the anchor becomes a polygon pad.
    Otherwise the anchor is the pad and the primitives become copper graphics.
  - Holes: oval drills become round (minimum size). Drill offsets are ignored.
  - Aperture-only pads become filled paste/mask graphics.
  - Back-side copies of exposed pads (thermal-via footprints) are dropped; back-only pads
    move to the front.
- **Deterministic ids:** an FNV hash of library, name and element. Re-importing updates
  library objects in place, so "update from library" works for projects that use them.
- **Catalog:**
  - Indexes names only, about 0.1 s for all KiCad libraries. Parses lazily and caches
    whole-library files.
  - Searches by words in "Lib:Name".
  - Finds KiCad: `KICADn_SYMBOL_DIR`/`KICADn_FOOTPRINT_DIR`, the macOS app bundle,
    /usr/share/kicad and Program Files.
  - Footprint suggestion uses the symbol's default footprint if it exists. Otherwise it
    picks among `ki_fp_filters` matches: first all pin numbers present and the same pad
    count, then filter order, fewest extra pads, and common packages (0603, SOT-23,
    SOIC…).
- **Parts:**
  - One variant per chosen footprint. Pads connect to pins with the same number; several
    pads may share a pin (split thermal pads).
  - Properties: KiCad symbol, Datasheet, Keywords.
- **Renderer:** filled polygons. There is a new TRIANGLE primitive (three vertices in the
  existing 32-byte instance) plus ear-clipping triangulation (`render::tess`). Outline
  edges are anti-aliased; inner edges are hard, so no seams. Used for filled symbol
  bodies, filled silkscreen and polygon pads (and later zones).
- **Library upsert:** `LibraryDoc::upsert` adds and replaces objects in one undo step.
  New objects that clash with existing names get "NAME 2".
- **App:** File › Import from KiCad… (also in the Libraries panel) opens a window with:
  - symbol/footprint search;
  - a list of picks, each with a footprint picker (suggestion plus filter matches; a
    footprint search result can be assigned with Use);
  - the target library (new or open);
  - a report with notes.
  The import opens the first part in the library editor. Command line: `--kicad DIR`,
  `--kicad-pick [footprint:]LIB:NAME`, `--kicad-import`, `--kicad-search TEXT`.
- **MCP:** `search_kicad` and `import_kicad` (symbols with optional footprints, lone
  footprints, target library by name). The import returns part ids for `import_parts`.
  Verified live: search NE555, import, import_parts, place_part.
- **Verification against the official libraries** (`kicad/tests/corpus.rs`, enabled by
  `KICAD_SYMBOL_DIR`/`KICAD_FOOTPRINT_DIR`):
  - KiCad 10.0 (22,911 symbols in `.kicad_symdir`) and 9.0 (22,583 symbols in
    `.kicad_sym`) with 15,468 footprints: everything parses and converts, with JSON
    round trips.
  - All symbols with a suggested footprint become valid parts (21,583), and the library
    validates.
  - Spot checks: Device:R → R_0603 with both pads; LM2904 → 3 units on SOIC-8, 8/8
    pads.
  - 3.6% of parts have a pin without a pad. They come from KiCad's own data: filters
    that name the wrong package (74LS20: `DIP?12*`) or default footprints numbered
    differently. The part form shows these as "Pins without a pad".
  - Unit tests have handwritten KiCad 6- and 10-style fixtures. `library_import.rs` covers
    both symbol formats on disk, re-import, undo and name clashes.
- **Not yet:**
  - Back-side footprint content.
  - Text boxes, De Morgan bodies, alternate pin functions.
  - Oval drills (approximated).
  - 3D models.
  - Symbol and footprint preview inside the import window (the editor shows the result).

## Milestone 6 results (2026-11-09)

- **Connectivity** (`editor_core::connectivity`, rebuilt from the project):
  - Union-find over pins, wire ends, labels and junctions. Points that coincide
    connect. A wire end, pin, label or junction lying inside a wire connects to it
    (T connection). Crossing wires connect only with a junction. Labels connect by name.
  - Net names: power label, then net label, else `Net-(R1-1)` from the lowest
    reference and pad. Two different label names on one net are reported as a problem.
  - Junction dots are drawn automatically where three or more connections meet, and
    unconnected pins get a ring. Both are a separate scene item, updated after each change.
  - A full rebuild takes about 13 ms at 10k components (release). The document skips
    recomputing during a drag when a build takes over 4 ms, and catches up afterwards.
    An incremental version is deferred until it's needed.
- **Dragging keeps wires attached** (`keep_connected` on `Move`/`Rotate`/`Mirror`; the
  schematic view sets it, commands default to false):
  - Horizontal and vertical wires attached at one end shift sideways and stay
    orthogonal. At the far end, the single aligned neighbour stretches, or a dogleg wire
    is inserted where something else is attached.
  - All other wires, and every wire on rotate/mirror, stretch. Zero-length wires are
    deleted.
  - Stable over many small drag steps; the whole drag stays one undo step.
- **Schematic tools** (`view::sch_tools`, toolbar above the schematic, settings in the
  right panel):
  - **Wire (W):** orthogonal L segments with a ghost. A chain ends on a pin, wire, label
    or junction (that target is highlighted), on a repeated click, or with Esc. `/`
    switches horizontal-first and vertical-first corners.
  - **Label (L):** a trailing number counts up (NET1, NET2, …).
  - **Power (P):** GND, +3V3, +5V and so on; any name works.
  - **Junction (J).**
  - **Place:** started from the library panel's Place button. The ghost follows the
    cursor; R rotates and X mirrors. Clicks place repeatedly; multi-unit parts place
    unit A, B, … of one component (new `PlaceUnit` command), then start a new component.
  - New `AddLabel.rotation`.
- **Context-dependent shortcuts:** each action is global or belongs to the library, the
  schematic or the board. Contexts may share keys (L = line in the symbol editor, label
  in the schematic). Conflicts are reported only within a context.
- **Properties:**
  - A selected wire, label or junction shows its net and pins.
  - Symbols and footprints list pin, pad and net.
  - The project summary counts nets and unconnected pins.
- **MCP:** a `get_netlist` tool returns nets with pins (reference, pin, pads, position),
  unconnected pins and connectivity problems.
- **Tests:**
  - `editor-core/tests/netlist.rs`: sample circuits (LED + resistor, inverting op amp,
    dual op amp across units), T connections, crossings, labels, touching pins, and drag
    behaviour.
  - `view/tests/sch_tools.rs`: a circuit drawn with the tools, wire-chain rules,
    unit-by-unit placement.
  - MCP e2e: `get_netlist`.
- **Development aid:** `--script "key W; click 7.62,15.24; move 17.78,22.86"` feeds input
  events (schematic mm, or `ui-click x,y` in screen points) before `--screenshot`.
- **Not yet:**
  - Bus wires, sheet pins and hierarchical labels.
  - ERC (pin type checks).
  - Wire posture that flips automatically with the cursor.
  - Drag of wire segments themselves (moving a wire moves it and drags its neighbours).

## Milestone 7 results (2026-11-09)

- **Board connectivity** (`editor_core::copper`, rebuilt after board or schematic changes):
  - Copper items: pads (on their side's copper; through-hole on all layers), tracks and
    vias. Shapes are capsules (tracks, vias, round and oval pads) or polygons (other
    pads). Items on a common layer that touch form islands (grid broad phase, union-find).
  - Nets come only from the schematic. A pad gets its pin's net; tracks and vias get the
    net of the pads in their island. Tracks have no stored net. An island with pads of
    two nets is reported as a short.
  - Ratsnest: a minimum spanning tree per net over pad centers, via centers and track
    ends, with points of one island already connected (Delaunay edges for large nets).
    "Fully routed" means no ratsnest lines and no shorts.
  - About 6 ms for 20k pads (10k components, release), on top of the ~13 ms schematic
    connectivity. Same rule as the schematic: no rebuild during a drag if it is slow.
- **Layer stack and design rules are undoable:** `SetCopperLayers { count }` (even, 2–32;
  refuses to drop inner layers that still have copper) and `SetDesignRules`. A change set
  carries the board setup before and after; the board view rebuilds its layers.
- **Board tools** (`view::pcb_tools`, toolbar above the PCB with the active layer and
  routing status, settings in the right panel):
  - **Route (T):** 45° or 90° corners, straight-first or bend-first (`/`). Clicks snap
    to pad and via centers and track ends within 8 px. A route ends on copper. L
    switches to the next copper layer and adds a via at the current end when routing.
    The net being routed is highlighted.
  - **Via (Shift+V)** and **Outline (O)**: two corners make a rectangle on the Board
    Outline layer.
- **Net highlighting:** the routed net, or the nets of selected tracks and vias.
  Track and via properties show their net.
- **Board setup panel** (PCB with nothing selected): copper layer count and design rules,
  edited live with one undo step per edit.
- **MCP:** a `get_routing` tool returns unrouted connections (net, from, to), shorts,
  layers and rules. Agents route with `add_track`/`add_via`.
- **Demo:** D1 is rotated so its anode faces R1. The new short detection found that the
  old R1→D1 track crossed D1's cathode pad.
- **Tests:**
  - `editor-core/tests/board.rs`: routing removes ratsnest lines, vias, shorts,
    through-hole pads, a fully routed LED board, undoable layer stack and rules.
  - `view/tests/pcb_tools.rs`: routing two nets with a layer change through the tool,
    outline and vias.
  - MCP e2e: route one ratsnest line.
- **Not yet:**
  - Clearance checking while routing (DRC is milestone 9).
  - Push-and-shove routing; dragging tracks while keeping them connected.
  - Blind and buried vias in the UI.
  - Copper graphics inside footprints (from KiCad custom pads) don't count as connections.

## Milestone 8 results (2026-11-09)

- **Board follows the schematic** (tests in `editor-core/tests/board.rs` on a routed
  board):
  - Placing a part adds its footprint; deleting it removes the footprint, and undo
    restores it in place.
  - Variant changes swap the footprint in place; copper reconnects to the new pads.
  - Renamed labels and swapped reference designators carry the routed copper along:
    tracks have no stored net, and placements belong to components, not names.
  - Flipping a footprint moves its SMD pads to the other side.
- **Copper is flagged, never deleted.** After a schematic change, a track or via is
  flagged when its island connects pads of different nets (`Short`) or touches no pad
  (`Unconnected`):
  - The board view draws an orange halo around flagged copper.
  - The toolbar counts flagged items.
  - Track and via properties name the problem.
  - A "Board problems" section (PCB, nothing selected) lists shorts and selects all
    flagged copper with one click.
  - MCP `get_routing` lists flagged copper.
- **Cross-probing** (`view::probe::cross_probe`, a pure function):
  - The active view's selection highlights matching objects in the other view (yellow
    overlay): symbols ↔ footprints.
  - Schematic wires, labels and junctions highlight their nets' copper on the board.
  - Tracks and vias highlight their nets' wires, labels and junctions in the schematic.
  - The net being routed always wins.
- `view/tests/probe.rs`: probing both ways. The board view stays identical to a rebuild
  after variant changes, deletes, layer stack changes and undo/redo.
- **Not yet:** pin/gate swaps driven from the board (back-annotation is out of scope:
  the PCB never changes the schematic).

## Milestone 9 results (2026-11-09)

- **Zone fill** (`editor-core::zones`, i_overlay boolean operations on nm coordinates):
  - Each zone fills its outline, clipped to the board outline minus the edge clearance.
  - Other-net copper keeps the clearance (per-zone override or the design rule).
  - Pads of the zone's net connect through thermal reliefs. The gap and spoke width are
    new design rules (`thermal_gap`, `thermal_spoke`, default 0.5 mm; older files load
    with the defaults).
  - Tracks and vias of the zone's net, and copper without a net (stitching vias), are
    flooded.
  - Zones fill in priority order. Lower-priority zones keep the clearance to fills of
    other nets.
  - Islands touching no copper of the zone's net are removed.
  - Fills are derived data, like connectivity: they are never saved.
- **Two-pass connectivity:** board connectivity is built without zones, the zones are
  filled using those nets, then connectivity is rebuilt with the fill pieces as copper
  (`CopperItem::Zone`, `Geo::Region`). Zones satisfy ratsnest lines, and stitching
  vias take the zone's net.
  - `RegionIndex` grids a fill's edges for point and shape queries. Without it, a
    full-board fill with 80k vertices took 0.7 s just for the touch tests.
  - Fixed along the way: region–region touch tests only looked at one region's holes.
- **Rendering:** fills are drawn translucent in their layer colour, triangulated with
  i_triangle (holes, robust, O(n log n)). Edges used by only one triangle are the
  outline, and only those are anti-aliased. Earcut needed 1.9 s for one fill with
  thousands of holes.
- **Zone tool** (Z on the board, also in the tool bar):
  - Click corners; clicking the first or last corner again closes the zone.
  - Its net comes from the tool settings, or else from the copper under the first
    corner.
  - The net being drawn is highlighted.
- **DRC** (`editor-core::drc::run`, a pure function, sorted most severe first):
  - Shorts.
  - Clearance between copper of different nets on a shared layer (grid broad phase,
    exact distances). Pad pairs within one footprint and zone fills are skipped; fills
    keep the clearance by construction.
  - Copper too close to or outside the board edge.
  - Overlapping courtyards on the same side.
  - Track width and drill (vias, through-hole pads) below the minimums.
  - Copper touching no pad.
  - Unrouted connections.
  - Each violation has a message, a position and the objects involved.
- **DRC in the GUI:**
  - "Run DRC" in the board properties lists violations by kind.
  - Clicking a violation selects its objects and centres the view on it.
  - The board shows ⊗ markers, kept across view rebuilds.
  - The panel says when the board has changed since the last run.
- **DRC over MCP:** `run_drc` returns counts by kind plus the first `limit` violations,
  and marks them in the GUI.
- **Tests:**
  - `editor-core/tests/board.rs` (zones):
    - The fill connects its net.
    - Clearance to other-net copper and to the edge.
    - Thermal gap and spokes.
    - Dead islands.
    - Priority.
    - Stitching vias.
  - `editor-core/tests/drc.rs`: a clean routed board, plus seeded errors (one of each
    kind, found exactly once).
  - `view/tests/pcb_tools.rs`: drawing a zone with the tool.
  - `render`: regions with holes.
  - MCP end-to-end: `run_drc`.
- **Performance** (`edit_bench`, release, Apple M4). The zone is one top-layer zone of
  the largest net over the whole board:

  | | 2k components | 10k components |
  |---|---|---|
  | Fill + connectivity + render | 0.10 s (80k vertices) | 0.54 s (400k vertices) |
  | DRC | 8.5 ms | 13 ms |

  Board changes refill all zones. While dragging, slow rebuilds are deferred until the
  drag ends, as before.
- **Not yet:**
  - Incremental or background refill (only zones near the change).
  - Arc outlines in fills (circles are polygons with 16–64 segments, circumscribed so
    clearances are never undercut).
  - Exact courtyard polygons (DRC uses bounding boxes).
  - Per-net clearance classes.
- Dev aid: `cargo run -p mcp --example render_board -- out.png` renders the demo board
  offscreen after a DRC. It needs no window, which helps when the display is asleep.

## Milestone 10 results (2026-11-09)

- **Headless MCP mode:** `pcb-designer --mcp-stdio [PROJECT_DIR | --demo]`.
  - `mcp::stdio::serve` runs the protocol on a background thread (current-thread tokio
    runtime). The calling thread owns the `HeadlessHost` and answers calls until the
    client disconnects.
  - It serves the same tools as the GUI server; its instructions say it is headless.
  - A folder that doesn't exist yet becomes a new project, saved there by
    `save_project`.
  - Screenshots render offscreen, and logs go to stderr.
  - `mcp/tests/stdio.rs` drives a whole session over an in-memory pipe: read, route,
    DRC, screenshot, save, reload from disk.
  - The release binary was smoke-tested with raw JSON-RPC over real stdin/stdout.
  - Tools → "Copy Claude Code command (headless)" gives the `claude mcp add … --
    pcb-designer --mcp-stdio PROJECT` line.
- **Themes (dark and light):**
  - Canvas colours moved from constants into `view::style::Palette`, with `DARK` and
    `LIGHT` palettes.
  - The active palette is a global (`set_theme`, `pal()`). It is baked into scene
    instances when objects are drawn.
  - `Document::restyle` rebuilds both views with their overlays: connectivity, copper
    and fills, net highlights, cross-probe, DRC markers. `ItemView::restyle` does the
    same for library editors.
  - Canvas backgrounds come from the palette too.
  - App chrome colours are functions of the same theme, so chrome and canvases always
    match.
  - Switch with View → Dark/Light theme, or in Settings.
  - `view/tests/theme.rs`: after a restyle, colours per layer equal a fresh build in
    the new theme, and switching back restores the old colours.
- **Settings** (`app::settings`, JSON in the platform configuration folder;
  `PCB_DESIGNER_CONFIG` overrides it):
  - Saves the theme, recent projects (newest first, 10) and the key bindings that
    differ from the defaults. Broken files fall back to the defaults.
  - **Edit → Settings…** (⌘,) has the theme and an editable shortcut list: `×` removes
    a shortcut, `+` records the next key press with its modifiers (Esc cancels). It
    warns about conflicts and can reset to the defaults. Help → Keyboard shortcuts
    opens the same window.
  - `--keymap FILE` still applies for one session.
  - **File → Open recent** lists the projects; opening and saving add to the list.
- **Small things:**
  - Tools → Run design rule check.
  - The README covers headless mode and settings.
  - `render_board` example: `--light` and `--schematic`.
- **Verification:** light-theme renders of board and schematic, made offscreen with
  `cargo run -p mcp --example render_board -- out.png --light [--schematic]`. Window
  screenshots weren't possible in this session: the display was asleep and eframe drew
  no frames. The settings window and theme switch in the GUI still need a manual look.
- **Not yet:**
  - Saved window layout and pane sizes. The eframe persistence feature is off.
  - Per-layer colour customisation.
  - UI scale.

## GUI refinements (2026-11-09)

- **Default layout: tabs.** Schematic and PCB are full-size tabs of one area, with the
  schematic in front. The usual workflow is schematic first, then layout, so a
  permanent split wasted half the screen.
  - View → Tabs / Side by side switches layouts; library tabs stay with the schematic.
    Dragging tabs still works for any other arrangement.
  - Keyboard focus follows the visible tab, whether it changed by a click, drag and
    drop or closing a tab.
  - Cross-probing works across tabs: with nothing selected in the active view, the
    other view's selection is highlighted. For example, select R1 in the schematic,
    open the PCB tab, and R1's footprint is highlighted.
  - The script step `board` (dev aid) brings the PCB tab to the front.
- **Panel styling (IntelliJ-like).** The window background shows through 6 pt gaps
  between the side panels, the tab groups, the menu bar and the status bar. Panels and
  tab groups have rounded corners (10 pt) and no outline (tried; flat looks better).
  Hairlines inside separate the tab bar, tool bar and canvas. Side by side, the
  schematic and PCB groups are now clearly separate.
  - GPU canvases can't be clipped by egui, so their corners are masked afterwards with
    the window background (`theme::round_corners`, drawn by egui_tiles'
    `paint_on_top_of_tile`). A 1 pt stroke in the background colour anti-aliases the
    rounded edge.
  - Tabs are rounded pills: the active one is filled and the others highlight on hover.
    The tool bars share the tab bar's colour, with a hairline to the canvas.
  - `--side-by-side` starts in the split layout.
- **Floating tool bars (Blender-like).** The canvas fills the whole tab; the tools float
  over it (`toolbar.rs`).
  - Icon buttons in a vertical strip at the top left, in rounded groups:
    - Schematic: Select | Wire, Label, Power, Junction | Place (while placing).
    - PCB: Select | Route, Via | Outline, Zone.
    - Library: Select | Pin or Pad | Line, Rect, Circle, Text.
  - The icons are vectors drawn with egui's painter (`icons.rs`), so they stay crisp
    and follow the theme. The active tool is filled with the accent colour.
  - Tooltips open to the right of the button. They show the name, the shortcut from
    the keymap (so rebinding shows up) and a sentence on what the tool does.
  - A header next to the strip holds the tool options. PCB: active layer and routing
    status. Library: grid and Fit. The usage hint sits at the bottom left as a
    non-interactive pill.
  - Blocks have a soft shadow and a faint outline, so they read against any canvas.
  - The bars are drawn after the canvas in the same layer, so they win egui's hit test.
    They sense click and drag, so a press on them never reaches the canvas and starts
    no box selection or placement. The header's size is only known after its
    contents, so its background blocks clicks on the previous frame's rect.
  - Script step `ui-move x,y` (dev aid): hover a screen position, e.g. to screenshot a
    tooltip.
- **Library navigation removed.** The Back / Forward bar above library items, its
  history and the Back / Forward actions (View menu, ⌥← / ⌥→) are gone; the library
  tree is the navigation, and the item canvas now fills the tab like the others.
- **View buttons and units.** Grid and layer settings moved from the side panel to
  floating buttons at the canvas' bottom left: Layers, Grid and Fit view. Each opens a
  popup above itself. The usage hint now sits right of the buttons. The library item
  editor uses the same buttons, so its grid/Fit header is gone.
  - Layer popup: a checkbox per layer with a colour swatch in the colour the editor
    draws it, plus All / None. The order and grouping come from
    `View::layer_menu` / `ItemView::layer_menu`: copper top to bottom, vias and holes,
    top and bottom technical layers, outline, then overlays. Hidden layers show a dimmed
    swatch.
  - Grid popup: units (mm / mil / in), grid size (presets plus a field for any value)
    and Show grid. Schematic and symbol presets stay 100/50/25 mil in every unit.
    Board and footprint presets are round numbers in the unit. Switching units moves
    the grid to the nearest preset, so 0.5 mm becomes 25 mil instead of 19.69 mil.
  - The unit is global (`app/src/units.rs`, like the theme) and saved in settings.json.
    It applies everywhere lengths are shown or typed: property fields, tool settings,
    design rules, the board problems list and the status bar. The model stays in nm.
- **Snap to grid toggle** in the grid menu (per editor, `Interaction::snap`). Off:
  pointer positions are used as they are, to the nm. The grid keeps its size and stays
  visible, so turning snapping back on restores the spacing. The status bar shows
  "no snap" while it is off. Copper snapping in the router (`snap_to_copper`) is
  separate and stays on.
- **Tab icons.** Schematic (resistor zigzag), PCB (board with a track) and library
  (books) tabs have an icon. Each is coloured from the palette of what the tab shows:
  schematic wire, top copper, label amber. The icon is dimmed on inactive tabs.

## KiCad library download (2026-11-09)

- **Download instead of a KiCad installation.** `kicad::download` fetches the official
  `kicad-symbols` and `kicad-footprints` GitLab release archives. These are 12 MB each
  and about 450 MB unpacked; no 3D models. They are unpacked into
  `<config>/kicad/<version>/{symbols,footprints}`, which follows `PCB_DESIGNER_CONFIG`.
  - Only stable tags of `SUPPORTED_MAJOR` (10) that both repositories have are used.
    Release candidates are skipped, and KiCad 11 waits until the importer is tested
    against it and the constant is raised.
  - The download goes into `.download-<version>`, is renamed into place when complete,
    and older versions are deleted afterwards. A cancel, error or killed process leaves
    nothing half-installed (a leftover temp folder is replaced on the next try).
  - Gotcha: cancelling uses its own error and flag. `io::ErrorKind::Interrupted` would
    be retried forever by `std`'s and `tar`'s read loops.
  - `download::ensure` is shared by the import window, the app's MCP host and headless
    mode. With an existing download and no update request it makes no network access.
  - Libraries searched: the folders added with `--kicad` or "Add folder…", then the
    download if there is one, else an installed KiCad (`download::library_paths`).
- **Import window:** offers the download when nothing is found, then shows progress
  (bytes against an approximate total; GitLab sends no length) with Cancel. Afterwards
  it shows the downloaded version and "Check for updates" → "Update to x.y.z". Errors
  get a Retry. The work runs on threads polled each frame (`KicadImport::poll`).
- **Agents:** `download_kicad_libraries {update}` waits for the download and returns the
  version, folder and library counts. In the app it joins a download already running
  from the window. Headless mode now supports `search_kicad`, `import_kicad` and the
  download. Its imports go into a "KiCad import" library, and their parts straight into
  the project (`Command::ImportParts`); the `library` parameter is ignored there.
- **Tests:** `kicad/tests/download.rs` serves generated archives and tag lists from a
  local HTTP server. It covers version choice, install, keeping a download without an
  update, update and replacing the old version, cancel, and missing releases. The test
  against the real gitlab.com is `#[ignore]`d: `cargo test -p kicad --test download --
  --ignored`. `mcp/tests/kicad_headless.rs` covers headless search and import.
