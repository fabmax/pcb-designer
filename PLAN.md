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

- The running app hosts an **MCP server (Streamable HTTP on 127.0.0.1, opt-in via the
  Tools menu or `--mcp`)** using the official Rust SDK (`rmcp`). Agents operate on the open project,
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
