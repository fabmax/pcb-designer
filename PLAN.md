# PCB Designer – Plan and design notes

Requirements are in [pcb-designer.md](pcb-designer.md), user documentation in
[docs/](docs/README.md). This file holds the decisions, the current design of each area
with the gotchas a change needs to know, the planned **next milestones**, and the list of
things to do **later**.

## Decisions

| Topic | Decision |
|---|---|
| UI | egui / eframe, egui_tiles for tabs and splits. Dark and light themes |
| Canvas | Custom wgpu renderer (SDF instanced primitives) inside egui paint callbacks |
| Schematic → PCB | Always live, inside the same command transaction. PCB → schematic: never |
| Copper | Tracks have no stored net; nets come from the pads they touch. Copper whose net no longer fits is flagged, never deleted |
| Library parts | Copied into the project (Project Library). Each copy keeps `source` (library id, part id, content hash) for "update from library" |
| Schematic scope | KiCad's concepts, so KiCad projects can be imported and exported: buses, label scopes (local / global / hierarchical / power), net names and hierarchical sheets with sheet pins as KiCad does. Multi-unit parts work |
| Files | JSON, pretty, stable order, integer nm. A project is a folder of any name, recognised by its `project.pcbproj` (JSON; with `library.json`, `schematic.json`, `board.json`; `model::io::is_project_dir`). Opening accepts the folder or its `project.pcbproj` (`project_dir`). 0.4.0 named the project file `project.json`: still read (only when its `format` is ours), deleted when the new one is saved. Saving into a folder that isn't a project but has a file of ours by name is refused; a library is one `*.pcblib`. Files saved by releases keep loading: a renamed, removed or reinterpreted field bumps the kind's version (`FileKind::current_version`) with a migration in `model::io::MIGRATIONS` (on the JSON, before deserializing); a new field with a default needs neither. `view/tests/saved/VERSION/` holds each release's examples (`save_examples -- --release VERSION`); `saved_files.rs` loads them, saves again and fails if any old value (after migrating) is gone or changed |
| MCP | In the app (Streamable HTTP on 127.0.0.1, opt-in) and headless over stdio. Every tool goes through editor-core commands |
| Simulation | ngspice 47, built from source and bundled: its shared library in the app (live traces), worker processes of our own for sweeps and isolation. Design in "Simulation (`sim`)" |
| Routing | Push and shove after KiCad's PNS router (hulls, lines walked round a pusher's hull, a shove stack, spring-back by recomputing from the board at the last click). Design in "Connectivity, copper, zones, DRC" (Router, Shove) |
| Platforms | macOS first. macOS-only chrome (native menu, title strip) is `cfg`'d; Windows/Linux keep the egui menu bar and system title bar |
| CI | GitHub Actions. `ci.yml`: fmt and clippy (`-D warnings`) on Linux, on the latest stable Rust (update locally with `rustup update` when its new lints fire), `cargo test --workspace` on macOS, Linux and Windows (`--no-fail-fast`) for pushes to main and pull requests (on Linux with Mesa's software Vulkan for offscreen rendering). `.github/actions/ngspice` builds the bundled ngspice with `scripts/build-ngspice.sh` on every OS (Windows in MSYS2 UCRT64, its `lib/` put on `PATH` for the MinGW runtime DLLs), cached by the script, version and patches, so the simulation tests run everywhere (`REQUIRE_NGSPICE` turns their skip without ngspice, `sim::skip_without_ngspice`, into a failure); it comes after `rust-cache`, whose post step cleans `target/` after this cache is saved. `release.yml` (manual, or a `v*` tag, which also publishes a GitHub Release): the ad-hoc signed `.app` for Apple Silicon (`bundle-macos.sh`: ngspice in `Contents/Frameworks` and `PlugIns/ngspice`, each signed before the app), and a `pcb-designer/` folder for Linux (`lib/libngspice.so.0`, `lib/ngspice/*.cm`; the binary's debug info stripped: the release profile's line tables would make it 4× the size; symbols stay) and Windows x86_64 (`ngspice.dll` and MinGW's runtime DLLs beside the exe, `ngspice\*.cm`), each with ngspice's COPYING. Linux system packages are in `.github/actions/linux-deps`. Dependabot (`.github/dependabot.yml`) opens weekly update PRs: egui's family with wgpu, winit and muda in one, other minor/patch crates in one, majors alone, actions in one. Cargo counts 0.x → 0.y as minor, so a breaking 0.x bump can hold up the grouped PR: split it off then. Tests that need a GPU adapter skip without one |

## Model and files (`model`)

- Lengths are `i64` nm (`Coord`); `Point`/`Size` serialize as `[x, y]`. `Angle` in
  degrees; multiples of 90° rotate exactly. `Transform`: mirror, then rotate, then
  translate.
- Every object has a UUID. `Store<T>` is insertion-ordered and saved as an array;
  re-inserting at the old index restores the order exactly (undo).
- Board parts: a `Part` without units (`Part::is_board_part`). Their components have no
  symbols and live on the board only: `PlaceBoardPart` creates component + footprint
  (footprint first, so sync doesn't stage another), deleting the footprint deletes the
  component, `PlacePart` refuses them (and `PlaceBoardPart` normal parts). Every variant
  needs a footprint. A placed part can't change kind (`check_kind_kept`, in `Update` and
  `UpdateParts`).
- Library: `Symbol` (named pins, `Pin::number`, `key()` = number or name), `Footprint`
  (pads by number), `Part` = units (one symbol each) + at least one variant (footprint +
  pad → `(unit, pin)` map; several pads may share a pin). A variant without footprint
  (`PartVariant::no_footprint`, "No footprint") has no connections; components using it
  are off the board (power flags, logos, KiCad's "exclude from board"). Off-board is a
  choice the part offers, never a free per-component switch.
- Categories: `Part`, `Symbol` and `Footprint` each have a plain-text `category` (empty:
  none; not written then). There is no list of categories: `Library::categories` derives
  them (parts and symbols share one vocabulary, footprints have their own).
  `Transaction::normalized` trims them, so they match exactly. A part's category is part
  of its content, so "Update from library" takes it. KiCad imports leave all three
  empty. A part made from a symbol (`LibraryDoc::new_part`) takes the symbol's category; after
  that they're independent (several parts may share a symbol across categories).
- Schematic: design-wide `Component`s (refdes, value, part, variant), units placed as
  `SymbolInstance`s; every graphic carries its sheet id. A unit of a part with several
  units shows its reference as `U1-A` (`Part::unit_reference`: the unit's name, its
  letter when unnamed); drawing only, `refdes` and netlists stay `U1`. Buses: `BusWire`, `BusEntry
  {pos, dir}` (wire end = `pos + dir.offset()`, a fixed 2.54 mm diagonal).
- Sheets (KiCad's model): one root sheet (no `parent`, named "Schematic") and sub-sheets
  with a `parent`, each drawn once on its parent as a sheet symbol (`pos` = bottom left
  corner, `size`). All in one `schematic.json`. Names are unique among siblings and
  have no `/`; `sheet_path` is `/Power/Regulator` (empty for the root, as in net names),
  `display_path` shows the root as `/`, `find_sheet` takes an id, a path or a unique
  name. `sheet_of(id)` is where an object is drawn (a sheet's symbol: its parent).
- Pages: every sheet has a `Page` (`PageSize`: ISO A5–A0, US Letter/Legal/Tabloid, ANSI
  C–E, `Custom {w, h}`, or `None` for no frame; plus `portrait`), its lower left corner
  at the origin, so content on the page has positive coordinates. The project's
  `TitleBlock` (title, date, revision, company and a multi-line comment, free text; the
  comment's line breaks are its lines in the frame, KiCad's comments 1–9 become lines) lives on
  the root sheet so that `update` and undo cover it; `Schematic::title_block()` reads
  it, `SetTitleBlock` writes it. `page_order` numbers pages: root first, then depth
  first with sub-sheets in `natural_cmp` name order (the Project panel's order).
- Board: `FootprintInstance` stores only the placement; the footprint comes from the
  component's variant, so variant swaps need no board edit. Copper layers 0 (top)…63,
  technical layers from 100.
- `Project::libraries`: external library paths, absolute in memory, stored relative to
  the project folder with `/` (`io::stored_path`, `io::normalize` is lexical).
- Object JSON uses a `"type"` tag (`"kind"` clashed with labels' own `kind` field).
- Shared helpers: `natural_cmp` (numbers by value, then case-insensitive, then bytes;
  allocation-free, used for every name sort including netlists) and `Fnv1a` (stored
  hashes: `content_hash`, `design_hash`; `of_json` hashes while serializing).
  `kicad::kid` is a different, two-lane hash; changing it changes imported ids.

## Commands, undo and sync (`editor-core`)

- Serializable `Command`s (`"command"` tag) are the only way to change a project. Each
  runs in a `Transaction` with object-level before/after snapshots; on error it rolls
  back. Schematic → board sync runs in the same transaction, so one undo reverts both.
  Tests check byte-identical files after undo all / redo all.
- The `ChangeSet` drives incremental updates. The `Setup` (layers, rules, ERC and DRC
  settings) and the library list ride along as before/after (`setup`, `libraries`;
  `board_setup_changed`, `erc_changed`, `drc_changed` say which part); views ignore them. Gotcha:
  `ChangeSet::coalesce` (also behind `then`, which merges grouped commands into one undo
  step) folds only updates into an earlier change of the same object. Removals and
  re-insertions keep their event order, because undo re-inserts removed objects at
  their stored index: an object updated, then removed, becomes one removal where the
  removal happened; one removed and inserted again stays two changes (it is appended).
  `then` takes `self` (history grouping takes the top entry's set, no copy per drag
  frame; `coalesce` still walks the whole list). If undo or redo finds a change's
  object missing (history and project diverged, a bug), it logs and skips that change
  instead of panicking.
  `Change::affects_connectivity`: schematic changes always; library edits only when
  pins, pads, units or variants change.
- `keep_connected` on `Move` / `Rotate` / `Mirror` / `FlipSide`: a plan is made from the
  geometry *before* the move, then applied after it. `rubber_band` (wires, buses
  separately): unmoved wires at moving points follow (orthogonal ones shift sideways,
  far end stretched or doglegged); an end of a moved wire on something staying put (pin,
  label, junction, entry, sheet pin, branch, a collinear or diagonal neighbour, a wire's
  inside) is *held*: an orthogonal wire keeps its direction and a dogleg joins it back,
  others stretch; a single perpendicular neighbour follows instead. Unmoved wire ends,
  labels and junctions on a moved wire's inside slide across with it. Merging what the
  move left collinear is `segment_tidy`'s (below). `track_band`: track ends inside moved pad/via copper
  (`copper::footprint_copper`, grid-indexed ends) follow; a straight follower moves
  along and its far end slides along a single straight, non-parallel, unmoved neighbour
  (`slide_point`, refused if either would turn around), else it stretches; ends of
  moved tracks on staying copper or neighbours slide the same way or get a joining
  track. Sheet symbols drag their pins' wires. Gotcha: everything scanned per command
  is limited to the points involved (`attach_counts` with a `wanted` set) — a full pin
  scan per step made symbol drags 10× slower.
- **Segment tidy** (`segment_tidy`, run by `Editor::execute_grouped` after every
  command; wires and buses per sheet, tracks per copper layer): around the points the
  command changed, overlapping collinear segments become one, a segment is split where
  another's end or a junction (tracks: a via) lies inside it, and collinear segments
  meeting with nothing else attached are joined (wires: `attach_counts`; tracks: no pad
  or via copper covering the point; no tee). So a T always ends wires, and four ends
  meeting stay four (joined they'd only cross, unconnected). Tracks: only straight ones,
  and they become one only with the same width and lock (`Seg::class`; a wider track on
  top stays, its ends split the one below); arc ends still count. `Command::tidy_keep`
  says which ids survive: a move's moved segments are never split or dropped (a drag
  replays its Rotate and Move with the ids it started with); moves without
  `keep_connected` (a carried paste's steps) and `SetTrackPaths` (shoves, several per
  step) skip the tidy. A shove drag step runs its Move and pushes as one transaction
  (`Editor::execute_then`), tidied after the pushes: tidied after the Move alone, a track
  dragged onto another merged with it before the shove could push it. The kept id:
  moved, else untouched, else changed, else new. Gotcha: only segments reaching into the
  box around the changed points are indexed, and `attach_counts` / `pads_near` skip
  symbols and footprints farther than the library's longest pin or pad reach from it; a
  full scan per command made the 10k stress sheet 8× slower to build and drags 3× slower.
- `Document::read_only` (Built-in Library) makes every command fail with a reason.
- Sync (`sync.rs`): a component is on the board exactly when its variant has a footprint.
  It checks changed components *and* the components of changed parts (a part edit can
  take a variant's footprint away), then adds placements in the staging area or removes
  them; copper at removed ones stays. It keys on `variant.footprint.is_some()`, not on
  the footprint lookup, so a dangling reference never deletes placements.
- `ImportParts` / `UpdateParts` carry `PartBundle`s (part plus exactly the symbols and
  footprints it references, checked by `check_bundle`), built by `Library::bundles`,
  which also sets the part's `source`. Use `Command::import_parts(&lib, ids)` rather
  than cloning a library into the command.
- `UpdateParts` replaces project parts by the library version (variant kept by id, then
  name). It refuses when placed units would disappear; `check_part_still_fits` guards
  `Update` of a used `LibPart` the same way.
- `Update` validates references and keeps ownership fixed: a footprint's or symbol's
  `component` can't change, and a symbol can't take a unit its component already has
  placed (checked against the project before the update, so two symbols can't swap
  units in one `Update`).
- Part states (`query::part_statuses`): the stored `source.content_hash` is compared
  with the library's hash and with the copy's own → up to date, library changed,
  modified locally, both changed, library missing, removed from library, local.
- Clipboard (`clip`): a `Clip` is versioned JSON (`pcb_designer_clip`, a `domain` tag,
  the `anchor` it is held by) of whole objects in their original coordinates.
  Schematic clips carry the parts their symbols use (components come first). `Paste`
  gives new ids, keeps a refdes if it's free (so cut + paste keeps it) else the next
  one, imports missing parts (present ones win), then moves everything by
  `at - anchor`. Library item clips (pins, pads, graphics, texts) are pasted by
  `view::item::paste_elems` as an `Update`. Version 2: an older clip is refused.
  - A sheet symbol is copied with its sheet tree (`Sheet` objects parents first, then the
    sub-sheets' contents). Paste maps old sheet ids to new ones, names a top-level copy
    like `unique_sheet_name` (amp → amp2 → amp3) and drops copied title blocks. Only the
    top-level objects move by the delta (`placed`); sub-sheet contents keep their places.
  - Board clips carry board-part footprints with their component, fields and parts (a
    `library`, imported on paste). Footprints of schematic parts are left out and counted
    as skipped: they follow the schematic.
  - `Duplicate {ids, delta}` is copy + paste in one command (MCP's duplicate).
- `MoveToSheet {ids, sheet}` keeps ids (so components keep refdes, footprints, routing).
  A symbol takes its fields and the no-connects on its pins (`query::pin_positions`);
  unlisted wires stay. A sheet re-parents with its subtree, refused below itself,
  renamed like a copy on a clash. No net repair: the app counts renamed nets in the
  status bar. Gotcha: a moved sheet keeps its symbol position, which may lie off the new
  parent's page.
- `query::unused_library_objects` lists unused parts, then the symbols/footprints only
  they need; deleting in that order passes the in-use checks.

## Connectivity, copper, zones, DRC (`editor-core`)

- **Schematic** (`connectivity`): union-find over pins, wire ends, labels, junctions,
  entry wire ends (`Node::Entry`) and sheet pins. Coinciding points connect; an end
  inside a wire is a T connection; crossings need a junction. Same-named labels join:
  local and hierarchical ones on their sheet, global and power ones on all sheets
  (`LabelKind::is_global`). Net name as in KiCad: power, then global, then local, then
  hierarchical as `{sheet path}/{name}` (`/A`, `/Power/A`; `Schematic::sheet_path`),
  fewest `/` first, else `Net-(R1-1)`. Power ports are labels, not symbols: their
  `style` (`PowerStyle`, KiCad's power glyphs) is drawing only; `None` means the one the
  name suggests (`PowerStyle::for_name`: *GND* or EARTH → ground, CHASSIS, PE, else a
  bar). There are no hidden pins: pins connect only by wire.
- **No-connect flags** (`NoConnect`, an X at a pin end) are documentation too. A pin
  under one, or of kind `NotConnected`, is left out of `unconnected` (in `flagged`
  instead); every other open pin is a problem ("Unconnected pin R1-2"), as are flags off
  a pin or on a connected point. Flags on a symbol's pins move with it (all of `Move`,
  `Rotate`, `Mirror` via `transform_connected`) and are deleted with it. Gotcha: tests
  that expect no problems must wire or flag every pin, or filter "Unconnected pin". "Several names" is reported per sheet only (a net
  may be named differently on each side of a sheet pin). Gotcha: local nets are
  `/NAME`; tests that only need a named net use global labels. Problems are `Problem
  {rule, severity, message, objects, pos, key, excluded}`.
- **Sheet pins** are derived, never stored: one per hierarchical label name of the
  sub-sheet (`Schematic::hier_ports`, first label's shape), laid out by `Sheet::layout`
  (inputs left, the rest right, by name, 2.54 mm from the top; the drawn rectangle grows
  downwards past the stored `size` to fit). A sheet symbol's size is whole pin pitches
  (`Sheet::snap_size`, applied by `Transaction::insert` / `replace`, so every edit path
  keeps it): with the corner on the grid, every pin is on it. Properties' Height keeps
  the top edge (and so the pins) in place. On the parent they connect like component
  pins (`Net::sheet_pins`) and join the sub-sheet's labels of their name. ERC: unwired
  sheet pins, hierarchical labels on the root sheet, bus-named pins. The view redraws a
  sheet's symbol when a hierarchical label on it changes (`apply_objects`); pins are
  drawn as the label glyph with `LabelShape::reversed`.
  Junction dots and unconnected-pin rings are drawn automatically.
- **Buses:** a bus only groups signals; bus wires and entries never join nets, and a wire
  or pin touching a bus is a problem. Bus names follow KiCad (`D[0..7]`, `{SDA SCL}`,
  `I2C{SDA SCL}` → `I2C.SDA`), vectors capped at 1024. A label whose name parses is a
  bus label and skipped for nets. Gotcha: members are *label* names (`D0`), not net
  names (`/D0`); find a member's net with `Connectivity::member_net`.
- **Pads without a pin** (pads no pin maps to, and all pads of board parts) get a net of
  their own per pad name, `unconnected-(H1-Pad1)` as in KiCad, so copper of another net
  touching them is a short and pads with the same name (a mounting hole's via ring) don't
  short each other. Zones keep clearance from them like from any other net. Gotcha:
  overlapping footprints now short through such pads too.
- **ERC** (`erc.rs`, run at the end of `Connectivity::build`, its findings join
  `problems`): per wired net (lone pins are left to "Unconnected pin"), every pair of
  pin kinds present is looked up in the pin matrix (`ErcSettings::pin_pair`, a symmetric
  9×9 over `PIN_KINDS` with KiCad's defaults in `DEFAULT_MATRIX`), one finding per
  conflicting pair per net. Inputs need a driver (any other kind but power input and not
  connected, a power port or a power flag); power inputs need a power output pin, a power
  port or a power flag. Power ports and flags are not in the matrix. Plus components
  sharing a reference (`R?` and empty ones aside). Messages name pins as `U1-14 (VCC)`.
- **Power flags** (`PowerFlag`, KiCad's PWR_FLAG): a pennant at a point that powers its
  net and otherwise behaves like a no-connect flag (moves and deletes with the symbol
  whose pin it sits on, `flags_on_pins`). One touching no wire or pin is a warning.
- **Findings review** (`model::checks`): every problem has an `ErcRule` and every DRC
  violation a `ViolationKind`, each with a default `Severity` (Error, Warning, Ignore).
  Their identity is a key `finding_key(rule code, parts)`: an FNV hash of the sorted
  object ids, pins as `pin:{symbol}/{number}` (`pin_part`), nets for shorts
  (`short_key`), so it survives moves and edits. Settings live in the project:
  `Schematic::erc` (severities, matrix, exclusions) and `Board::drc` (severities,
  exclusions), both storing only changes from the defaults, and change through
  `SetErcSettings` / `SetDrcSettings` (part of `Setup` in the change set, with layers and
  rules; an ERC change makes connectivity stale, a DRC change re-runs an existing report).
  `apply_settings` (both sides) sets severities, flags excluded findings (kept, listed
  last), drops ignored ones and sorts errors first; the unconnected-pin rings, the canvas
  markers (layers of their own, the schematic's "ERC markers" and the board's "DRC
  markers": hidden in the layer menu, they're gone from the hover tip too
  (`View::markers_visible`); `View::drawing` leaves them out of prints) and the live board shorts in Messages
  follow it too. An exclusion keeps the message it was made for, so one that matches
  nothing any more is shown as unused and can be removed.
- **Hover tip** (`view::findings::at`, shown by the app's `findings_tip` after the
  tooltip delay, Select tool idle only): the messages of what is drawn under the
  pointer: markers (only while their layer is shown) and unconnected-pin rings by distance, plus
  findings listing the hovered object, and the hovered track's or via's flag (shorts
  following the short rule's severity and exclusions, like Messages). The short's nets
  come from a scan of the island's pads (hovered item only, so no index).
- **Arc tracks** are a `Track` with `mid` (a point on the arc, KiCad's start/mid/end);
  `a`/`b` stay the ends, so anchors, ratsnest and routing don't know about arcs.
  `Track::arc()` gives the exact `Shape::Arc` (drawing, Gerber G02/G03 with exact ends),
  `Track::path(err)` chords. Copper uses `Geo::Path` (capsules along chords at most
  `ARC_ERROR` = 1 µm off, widened by that), split into capsules by `Geo::capsules` for
  all tests. Moving an end goes through `Track::set_ends`, which keeps the bulge (mid in
  chord coordinates, scaled).
- **Board** (`copper`): pads (through-hole on all layers), tracks, vias and zone fill
  pieces form islands (grid broad phase). Nets come from pads; an island with two nets
  is a short; copper touching no pad is unconnected. Ratsnest: MST per net over pads,
  vias and track ends (Delaunay for large nets).
- **Board area** (`zones::board_area`, used by fills and DRC): rectangles, polygons and
  circles on the outline layer, plus lines and arcs chained end to end (1 µm tolerance)
  into loops, as KiCad draws outlines. Loops are oriented by nesting depth, so inner
  ones are cutouts. Strokes that close no loop give only their bounding box, and only
  when there is nothing else.
- **Zones** (`zones`, i_overlay on nm): outline minus `holes`, clipped to the board
  outline minus edge clearance and minus `no_pour` keepouts; keep clearance to other
  nets, own pads per `ZoneConnection` (thermal, thermal for through-hole only, solid,
  none; thermal gap/spoke overridable per zone), flood own tracks/vias and netless vias
  (stitching), keep clearance to netless tracks (flooding them gave them the zone's
  net), priority order, drop islands per `IslandRemoval`. A zone with net `""` is isolated
  copper: clearance to everything, all islands kept, no `ZoneNet`. Fills are derived,
  never saved.
  Two-pass connectivity: copper without zones → fill → rebuild with fills.
  `RegionIndex` grids fill edges (without it, touch tests on 80k vertices took 0.7 s).
- **Net classes** (`DesignRules::net_classes`, edited with `SetDesignRules` like the
  other rules): a class has optional clearance, track width, via diameter and drill
  (`None`: the board's, the "Default" class) and `nets`, exact names or `*` `?`
  patterns (`glob_match`). `class_of`: a class listing the exact name wins, else the
  first matching pattern. `assign_net` moves a net by exact name (the Nets panel's
  menu). By name, not by net id: nets are named in the schematic and a renamed net
  falls out of its class unless a pattern catches it. `BoardConnectivity::clearances`
  resolves the clearance per net index once per check (`Clearances::between` is the
  larger of two nets'); DRC, `route_check`, the Measure tool's `clearance::rule` and
  zone fills use it (a zone's own clearance, else its net's class's, against the
  other copper's class). Board edge clearance stays the board's. Commands don't know
  nets, so `Document::execute_grouped` fills in `AddTrack.width` / `AddVia` sizes from
  the class of the copper at the track's ends / the via (only when classes exist);
  the Route tool sizes its tracks and vias by the routed net, the Via tool by the net
  under the cursor. Track widths aren't checked against the class (only the minimum).
- **DRC** (`drc::run`, pure, most severe first): shorts, clearance (pads within one
  footprint and fills skipped), keepouts, board edge, courtyard overlaps (bounding
  boxes), track width and drill minimums, via spans (`ViaSpan`: a layer off the board,
  `from == to`, or `from` below `to`), zone net, unconnected copper, unrouted.
- **Checks while routing** (`route_check::check`, through `Document::route_clash`):
  `Probe`s (a track leg, a via) against other nets' pads, tracks and vias (zone fills
  make room themselves), keepouts forbidding tracks / vias, and the board outline
  (clearance to the edge, inside). Copper at the leg's start is never a clash, nor at its
  end when the net isn't known yet (a route from free space takes the net it ends on).
  `Clash.at` is the point of the probe nearest the problem (the marker). A clash refuses
  the click unless `PcbPlacer::allows_violations`: the "Allow violations" setting, or
  `force` (Shift held, set by the app every frame like `free`; Shift+L is a second
  binding of next layer so the via of a forced L goes through too). Copper near a
  probe comes from `BoardConnectivity::query`, an R-tree of item bounds built lazily on
  the first query after a rebuild (~2 ms for 20k items; `at` and `nearest_anchor` use
  it too); the route tool only rechecks when its inputs or the document revision change
  (`ClashKey`).
- **Router decisions:** Shove is the default mode (KiCad's default is Highlight
  collisions). Tracks and vias have a `locked` flag; locked ones are never pushed. Drag
  with shove follows the Route tool's mode (no separate switch). The preview executes
  the pushes as real edits in a provisional undo group, retracted every pointer move
  (the drag replay path), not ghosts: the view, ratsnest and markers stay exact and
  spring-back is free. Gotcha: every step bumps `doc.revision`, so caches keyed by it
  must use the revision at the leg start.
- **Router** (`router`): `RouteMode`
  (Highlight, Shove (default), Walkaround; a `PcbSettings` field, Shift+R cycles it),
  `route` dispatches. `hull::Hull`: a convex counterclockwise polygon, the obstacle's
  points grown by an octagon of clearance + half width + `SLACK` (10 nm), so edges run at
  45° multiples round round and axis-aligned copper (rotated pads give other angles); a
  path chord per hull for arcs; keepouts are their outline's convex hull.
  `walk::walk` replaces the path's first crossing of a hull by its outline from the
  entry to the *last* exit, always turning one way, up to 64 times; ending inside a hull
  stops where it goes in, starting in one gives just the start. Gotcha: overlapping
  hulls put path corners exactly on or inside another hull's outline: the entry is
  searched back to the last corner not inside (`inside`: more than 0.5 nm), and an exit
  or entry at a corner on the outline takes the nearest edge (`nearest_edge`).
  `walkaround` gathers hulls from `query` (5 mm round the leg, more if a walk leaves
  that), walks both ways, smooths each walk (checked exactly with `route_check`),
  checks each exactly with `route_check` and cuts it to the
  longest clean start (binary search to 1 µm: the board edge and approximations), and
  takes the one that reaches, then the shorter. ~0.2 ms per walk on the 10k stress
  board. Obstacles and their clearances are exactly `route_check`'s (same islands at
  the ends, net classes; `check_with` leaves out copper a filter says).
- **Smoothing** (`router::smooth`): walked paths carry the hull outlines' corners
  (steps round every obstacle, walls along a hull's sides). `smooth(path, clear)`
  replaces runs of up to 8 corners by the shortest octilinear connection (one segment;
  straight + diagonal either way round) when it saves 1 µm or a corner and `clear`
  passes for the new segments, never turning sharper than 90° where it joins; then cuts
  90° corners at 45° (whole shorter side, ½, ¼); up to 4 passes. Ends stay. A plain
  dog-leg stays as it is (the other way round is no shorter), so the posture holds.
  Used on the head's walks (both modes) and on pushed lines. Gotcha: a walk that left
  the leg as it was isn't smoothed, so the Route tool's corner style holds (90° corners
  would be chamfered) unless the leg had to go round something. Cost: ~+1.5 ms on a
  40-line cascade.
- **Shove** (`router::shove`): the head first walks round what can't move
  (`lay(…, fixed_only)`: pads, locked vias and tracks, arcs, keepouts; checks skip
  movable copper), then `shove::shove` pushes. `line::Line`: the chain of straight tracks of one
  width on one layer between joints (pads or vias containing the end, ends with ≠ 1
  other track end exactly there, arcs, a width change), with its tracks in order; fixed
  if a track is locked or it's an arc. Engine: a stack of pushers (the head, pushed
  lines); a pusher's *nearest* hit (exact `Geo::distance` below clearance −
  `TOLERANCE` 5 nm, as walked lines sit at the hull) is walked round the pusher's
  segment hulls both ways, the shorter one clear of fixed copper, the head, keepouts and
  the edge wins, and it's pushed with the pusher on top of the stack (its own pushes
  settle first, then the pusher is checked again). Vias (unlocked) are pushed by
  translation away from the pusher's nearest segment (up to 8 steps, then checked like
  a line), and drag the tracks ending within their radius on every layer: those lines
  get the new end and are walked round the pusher, then pushed in turn. Pushing a
  fixed line, a pad, a locked via or one of the pusher's own pushers (`pushes_back`: no
  ping-pong) fails. The result is `Shoved { lines, vias }`, `commands()` turning it into
  `SetTrackPaths` + via `Update`s. `shove_via`: a new via (L while routing in Shove
  mode) is a head of just a via; it's first checked against fixed copper, keepouts and
  the edge, then pushes; the tool adds it and the pushes in one undo group.
  `shove_shapes` is the engine with any head copper (shapes on layer masks, a net, the
  islands it belongs to).
- **Hug and push** (`router::shove`): a failed shove returns `shove::Stuck { clash, hug
  }`, `hug` being the copper of what the head pushed directly at the root of the stuck
  cascade (`Engine::root` walks `pushed_by` up from the failing pusher; the head's own
  hit if the head was pushing). The head is then laid again round the fixed copper
  *and* everything hugged so far (`lay(…, hug)`), smoothed, and shoved; up to `HUGS`
  (4) rounds while each adds new copper to hug, within the same 40 ms. Only a hug walk
  that reaches is taken; otherwise the leg stops short on the first walk as before
  (binary search). Gotcha: a line's ends never move, so a dangling track whose end lies
  in the head's hull can't be pushed and gets hugged.
- **Drag with shove**: `Document::drag_shove` (the app sets it each frame from the Route
  tool's mode: Shove only) makes a board drag of tracks and vias only
  (`ProjectTarget::edit`, `Move` in an undo group) run `Document::move_shoving`: the
  `Move`, then `router::shove_moved` with the tracks and vias the move changed (its
  ChangeSet, so the band's neighbours push too) as the head, against the connectivity
  from before the drag (`drag_base`, cloned once per drag group: each step is a replay
  from the start, so it stays valid and pushes spring back), the pushes in the same
  group. A failed shove retracts the step and errs; `Interaction::drag_to` then replays
  the last offset that worked and keeps the message (the drag stops at the obstacle). Limits: 1000
  pops and `SHOVE_TIME` (40 ms) for the whole call; on failure a binary search (20 µm)
  finds the longest head that pushes clear. Lines keep their end points. Pure: every
  step starts from the board as it is, so pushed tracks spring back. Once settled, each
  pushed line is smoothed (`smooth::smooth`, checked by `Engine::clear`: `hits` with no
  hits, the board limits). Costs (release): a push
  ~0.1 ms; a cascade through 40 lines at minimum pitch ~7.5 ms (~4 pops per line;
  `deep_cascade_timing`, an ignored test). `Command::SetTrackPaths`
  lays a chain along a new path (ids kept in order, extra segments copy the first
  track, left-overs deleted). `locked` on `Track` / `Via` (Properties, KiCad import;
  not serialized when false).
- **L while routing** (`next_layer`), two ways (`via_at_cursor`, "Via at"):
  - Pointer (default): L arms a via at the leg's end (`via_to`: the layer it goes to;
    L again cycles it, back to the leg's layer disarms; Esc disarms before ending the
    track). The leg stays on its layer; the via is part of the check: Shove gives the
    router's head a via (`router::shove_leg`, it pushes and stops short with the end),
    and `router::check_via` checks what can't be pushed (everything in the other modes;
    Highlight adds a via probe to the leg's). The click commits leg + `AddVia` with the
    preview's pushes in one undo step (`Document::commit_route` takes commands) and
    switches the layer; ending on copper drops the via. Ghost: `PcbGhost::Via` at the
    leg's end.
  - Leg start: adds a via at the track's end at once (shoving in Shove mode, one undo
    step with its pushes) and remembers it (`via_added`: position, layer it came from,
    `route_revision` after it). The next L, if the track hasn't gone on (same start,
    same revision: a route preview doesn't count), undoes it first; back on the layer
    it came from, no via, else one from that layer to the new one (3+ layers). Gotcha:
    the taken-back via is on the redo stack until the next edit. With Pointer, L with
    the pointer on the leg's start does this too.
- **Route tool and zones**: a leg ending, or a track starting, inside a zone isn't on
  copper: the tool asks `BoardConnectivity::item_at` (`at` without zones) whether the
  track ends there, which net it takes and whether to show the target. Zone tool:
  starting on a zone does take its net (`at`).
- **Route tool snapping** (`snap_to_copper`): pad and via centres and track ends within
  the tolerance, else the centre of a pad or via under the pointer, else the middle line
  of a straight track under (or within the tolerance of) the pointer, at the grid point
  projected onto it (geometry from the router's connectivity, so a route preview's
  pushes don't move it), else the grid point. Gotcha: tracks between header pins are
  off the grid, so the grid point alone never lands on them.
- **Route tool width wheel**: Cmd+wheel (Ctrl elsewhere) steps the width
  (`PcbPlacer::step_width`, `width_step`: 0.05 / 0.1 / 0.25 mm below 0.5 / 2 / 10 mm, from
  the rules' minimum) from the width it lays now (the net class's if unset), setting
  `PcbSettings::width`. A mouse wheel (`MouseWheelSource::Wheel`, only set on macOS)
  steps once per event: macOS sends one per notch but accelerates its delta (a slow
  notch is a fraction of a line); elsewhere line deltas are notches already, and a
  trackpad's points add up (50 a step). The canvas collects the steps only with `Canvas::command_wheel`
  on, and then zooms only by pinches (`Event::Zoom`, multi-touch), not by egui's
  `zoom_delta`: that counts Cmd+wheel as zoom, smoothed over the following frames, so
  skipping it just in the wheel's frame still zoomed. The ghost follows because the clash key holds the width.
  `Canvas::flash` shows the width by the pointer for 1.2 s.
- **Route tool and the router**: in Shove and Walk around (`PcbPlacer::routed`, Shift
  forces Highlight) the router's path is the ghost and what a click adds (straight
  segments; the leg's own, arcs included, if unchanged); stopping short shows a marker
  and a click adds up to there and goes on. Shove's pushes are real edits, a *route
  preview* in `Document` (`set_route_preview` / `clear_route_preview` /
  `commit_route` / `drop_route_preview`): an undo group retracted before each step,
  with the board connectivity from before the edits kept (`route_copper`,
  `route_base`, `route_revision` for cache keys), so snapping and routing never see the
  pushes; the click runs the leg in the same group (one undo step). Gotchas: any other
  command, undo, redo or save drops (retracts) the preview first; anything else
  changing the document (revision) makes its state stale; the app drops it every frame
  the tool doesn't want one (`wants_route_preview`) and counts routing as busy
  (connectivity deferred on large boards). MCP `route {layer, points, width, mode}`
  does the same in one step and returns the path, `reached`, `blocked` and the pushed
  chains.
- **Vias** have a span `from`..`to` (copper layers, top one first; `AddVia { span }`,
  `None` through). `Via::is_through` / `kind` (Through, Blind, Buried); `via_layers` is
  every copper layer between. Arcs: `fillet::fillet` rounds a corner with a tangent
  arc (largest radius the shorter leg allows, or a given one); `RoundCorners` rounds
  corners between selected straight tracks where exactly two meet and no pad or via is.
- **Keepouts** (`Keepout`, KiCad rule areas) are separate from zone holes on purpose: a
  hole is part of one zone's shape (other zones may fill it, it moves with the zone), a
  keepout is a rule for all zones on its layers (`layers` empty = all copper layers).
  `no_pour` is enforced by the fill; tracks, vias, pads and footprints (front courtyard
  bounding box, on the footprint's side) by DRC; tracks and vias also while routing.
- Gotcha: a `Zone` binds to its net by *name* (`Zone.net`, sheet-qualified for local
  labels: `/MID`), unlike everything else, whose nets come from pads. Renaming a label
  or removing the net's last pad leaves the zone on no net (as in KiCad); DRC reports it
  as `ZoneNet`.
- **Mask and paste** (`fab`) are derived from pads, never stored: every pad opens the
  mask on its side (holes on both sides) grown by `DesignRules::mask_expansion`; SMD
  pads get paste grown by `paste_margin` (negative shrinks, as in KiCad). Per pad,
  `mask` / `paste` turn that off (test points), and `back` puts an SMD pad on the
  footprint's other side (card edge fingers); footprint graphics and texts have `back`
  too (`board_layer(side)`). Always go through `fab::pad_*_layers` / `Pad::side`,
  never `for_footprint(…, inst.side)` for pads. Back courtyards are ignored by DRC.
- **Drills** are a `Size`: round when square, else an oval slot along the longer side,
  turning with the pad. `fab::pad_hole` is the one placed hole (`Hole { a, b,
  diameter }`, `a == b` for round) for the view, Excellon (slots as `G85` routes, read
  back as strokes) and DRC (minimum drill = the narrow side). Drill offsets are ignored
  on import. Vias stay
  tented. Explicit footprint mask/paste graphics add to that. `fab::pad_outline` is the
  one placed pad shape (rotation includes mirroring; grown rectangles keep sharp
  corners, polygons offset with round joins) for the board view and the Gerber export.
- **Reference and value texts** (`FootprintField`, `board.fields`; `SymbolField`,
  `schematic.fields`): a placed footprint or symbol gets its own copy of each library
  text that is exactly `${REF}` / `${VALUE}` (`is_field_text`), in owner coordinates,
  so it moves, turns and flips with its owner but can be dragged and turned on its own
  (`transform_objects` moves a field alone in its owner's frame; mirroring one alone is
  refused). Created and removed with the owner (`insert_fields` / `remove_fields`,
  `insert_symbol` / `remove_symbol`: placing, units, sync, board parts, paste); deleting
  one is refused, hide it instead. Copy takes a symbol's fields along; pasted symbols
  without fields get fresh ones. The drawing skips the templates for placed owners (the
  place ghosts draw them). Fields and footprint texts are drawn `Text::upright` (in
  Gerber too). Hit testing is the text's bounding box, which beats the owner's body.
  Saves from before `SymbolField` load without schematic fields (no fix-up). Gotcha: a
  library edit of the templates doesn't reach owners already placed.
- `SetComponent` refuses an empty or already used reference (except ones ending in `?`,
  which annotation numbers later).
- **Slow rebuilds wait:** when connectivity (or redrawing library users) took over 4 ms
  (`Document::slow`), it is deferred during a drag and caught up afterwards.

## Gerber export (`gerber`)

- `gerber::generate` writes one Gerber X2 file per layer (copper, mask, paste, silk,
  outline; mm, format 4.6, so coordinates are nm) and Excellon drills (plated: TH pads
  and through vias; non-plated; one per blind/buried span, `-front-in1.drl`, X2
  `Plated,1,2,Blind`, `FileKind::SpanDrill`), named like KiCad's (`Demo-F_Cu.gbr`,
  `-PTH.drl`), and a job file last (`-job.gbrjob`, `FileKind::Job`). `export`
  replaces `<project>/output/gerber/` and `output/assembly/`, writes
  `output/<name>-gerber.zip` (own minimal zip writer, fixed dates; the job file is in
  it) and `export.json` (`Manifest`: files and assembly files with kinds and labels,
  and `design_hash` of board + schematic + library, for "outdated"). `Document::export_gerber`
  refills, runs DRC (violations are warnings) and needs a saved project and an outline.
  Used by the menu (⇧⌘E), MCP `export_gerber` and `pcb-designer export-gerber DIR`.
- Pads are flashes; rectangles and ovals at quarter turns use R/O with swapped sides,
  everything else the `RoundRect` macro with the corner centres pre-rotated (readers
  such as PyGerber ignore the rotation of circle primitives). Polygon pads and zone
  fills are regions; holes are bridged into the outline with cut-in lines
  (`bridge_holes`), never drawn clear, as clear polarity would also erase other copper
  inside the hole. Mask files are written with negative polarity (drawn = opening).
- Copper carries X2 object attributes: `.N` (net, from `BoardConnectivity`) on fills,
  tracks, vias and pads, plus `.P` (refdes, pad name) and `.C` on pads of components.
  `GerberWriter::object` writes `%TO` only when the set changes, `%TD*%` before the
  next set and at the end; regions clear only their aperture function
  (`%TD.AperFunction*%`), as a bare `%TD*%` would also drop the object attributes.
  Values are escaped (`,` `*` `%` `\` and non-ASCII as `\uXXXX`).
- **Job file** (`job.rs`): JSON with the creation time (UTC, from `SystemTime`; no date
  crate), the outline size at the stroke centre line, the copper count, the stack-up
  (`Board::stackup`, Board setup → Stack-up, `Command::SetStackup`, part of `Setup`):
  `BoardThickness`, `Finish` (unless "not specified") and `MaterialStackup` (silk, mask,
  copper and FR-4 between, the copper's leftover thickness split evenly; dielectrics
  aren't modelled; copper is stored in nm and labelled in µm and oz), the board rules and `FilesAttributes` scanned
  from each file's own `TF.FileFunction` / `TF.FilePolarity` lines (Gerber `%TF…*%` and
  Excellon `; #@! TF…`), so the two can't disagree.
- **Assembly** (`assembly.rs`): `-bom.csv` (lines grouped by value, footprint and all
  properties; designators naturally sorted and joined; a column per property key used)
  and `-positions.csv` (footprint origin in board mm as in the Gerbers, rotation as
  placed, Top/Bottom), laid out by the board's `AssemblyPreset` (Board setup →
  Assembly files, `Command::SetAssemblyPreset`, part of `Setup`; MCP `export_gerber
  {assembly}` sets it undoably, the CLI's `--assembly` only for that export): Generic
  (a column per property key), JLCPCB (LCSC number from `LCSC` / `LCSC Part #` /
  `JLCPCB Part #`, any case) and PCBWay (`Manufacturer`, `MPN`, SMD/THT from plated
  holes). Board parts and "do not populate" components (`Component::dnp`,
  imported from KiCad's `(dnp yes)`) are left out; DNP only touches assembly, the
  board keeps their pads. Views draw them faded (`Painter::fade` on the item's
  instances, its fields too) with a `sch_dnp` cross over the symbol body. CRLF and quoted fields, as the
  assembly services' importers expect.
- Text is written as regions, one per glyph outline (`font::text_shapes`, holes
  bridged like zone fills), mirrored around its anchor on bottom layers (the editor
  never mirrors text). Glyphs are placed relative to the anchor in f32 and offset in nm:
  board coordinates are too large for f32 at nm precision.
- Checked against PyGerber renders (in a throwaway venv); no files are committed.
- **Schematic PDF** (`gerber::pdf`, `view::print`): our own PDF writer (like the zip
  writer; flate2 for the content streams) draws render `Instance`s as paths: capsules
  as round-capped strokes, rings as strokes between their radii, rects with Bézier
  corners under a `cm` rotation, arcs in quarter-turn Béziers. Runs of triangles of one
  colour (text, polygon fills) become one path with every triangle turned
  counterclockwise: shared edges cancel, so no anti-aliasing seams. Alpha goes through
  an ExtGState per value. `view::print` builds a fresh `View` per sheet (depth first)
  under `style::with_palette(&LIGHT)`, a thread-local override so the app's theme and
  parallel tests aren't touched, with only junction dots of the connectivity
  (`View::set_junctions`: no unconnected rings, findings or overlays) and the user layers
  (`View::drawing`). A sheet without page gets its drawing's bounds and a margin.
  `Document::export_schematic_pdf`, File menu, MCP `export_schematic_pdf`, CLI
  `export-schematic-pdf`. Checked with PDFKit renders (no PDF tools in the repo).
- **Reader** (`gerber::reader`): Gerber X2 / RS-274X and Excellon into primitives in nm
  (circles, round lines, rotated round rects, polygons; arcs become segments). Macros
  are evaluated (primitives 1, 4, 5, 20, 21; expressions, `$n=`), LR/LS apply, trailing
  zero omission and inches work. Unsupported things are warnings, not errors (G74, G91,
  LM, thermals, exposure-off primitives). Round-trip tests write a board with rotated,
  mirrored, oval and polygon pads and probe points just inside and outside every pad,
  and grid-sample zone fills against the bridged regions.
- **Viewer** (`view::GerberView`): one scene layer per file, stacked like the board,
  labels and kinds from `export.json` (else the X2 `.FileFunction`). Negative files are
  inverted with i_overlay: the outline file's bounding box minus the openings. Clear
  polarity is drawn in the background colour (our files never use it). `describe` (an
  R-tree over objects, exact `Prim::contains`) names the file, D code, aperture,
  `.AperFunction`, pad (`.P`) and net (`.N`) under the pointer. The reader keeps each
  distinct `%TO` set once (`Image::object_attributes`, objects hold an index).
  The viewer skips files whose kind isn't an image (the job file). The app's Gerber tab (`Tab::Gerber`, not a
  `ViewId`: it has no document) uses its own `Canvas`; MCP `screenshot` target
  `gerber` (optional `layers`) renders the same view offscreen.
- **Outputs in the tree** (`app/src/outputs.rs`): Outputs › Gerber › files and
  Outputs › Assembly › files. Image files open in the Gerber tab; the job file and the
  CSVs are `NodeId::DataFile` and open in the system's app. Outputs › Schematic PDF
  (`NodeId::SchematicPdf`: the newest `output/*-schematic.pdf`) opens in the system's
  app, or exports when missing; it isn't marked outdated (no hash saved with it). Scanned
  when the project folder changes, after an export and when the window regains focus
  (no file watching). "Outdated" compares the manifest's hash with `design_hash`,
  recomputed once per document revision and never while an interaction is running.

## Simulation (`sim`)

User-facing behaviour is in [docs/simulation.md](docs/simulation.md); what's ahead in
**Later** (Simulation).

- **Decisions:** the schematic simulates as drawn (no separate simulation schematic);
  sources are ordinary components (one part per kind, `VSource` / `ISource`, the
  waveform picked in Properties), kept off the board by having no footprint, and a test
  bench sub-sheet keeps them out of a design's main sheets. The setup lives in
  `schematic.json` (`Schematic::simulation`, like the ERC settings), so it rides in the
  change set's `Setup`. The plot is a tab beside the schematic, not a modal window, and
  a widget of our own: `egui_plot` lacks log axes, a second y axis and per-column
  decimation, and doesn't follow the palettes. ngspice comes with the app (built from
  source, see "Bundled ngspice"): the shared library in the app's process by default,
  worker processes of our own for sweeps, as the fallback and as the safe setting. Not goals for now: mixed-signal, board parasitics, RF.

- **Model** (`model::simulation`; named so, as `model::sim` would clash with the `sim`
  crate under `use model::*`): `Part::sim: Option<SimModel>` is part content (in the
  content hash, so "Update from library" brings it). `SimModel { device, pins, params }`:
  `pins` are `(unit, pin)` in SPICE node order, empty = every unit's pins in symbol
  order; `params` override a primitive's `.model` card or go to a subcircuit instance,
  `${VALUE}` takes the component's value. `SimDevice::Ignore` leaves a part out
  silently (connectors), `sim: None` with one warning listing them. `Component::sim:
  ComponentSim` holds overrides, `exclude` and a source's `Source { waveform, ac,
  ac_phase }`. Waveform and analysis numbers are SI `f64`; values and parameters stay
  strings.
- A source's value is its waveform text ("SIN 1 V 1 kHz AC 1 V"), set by
  `Transaction::normalized` on every insert/replace of a component with `sim.source`
  (it takes the project, to say V or A from the part's device), so `${VALUE}` fields
  show it and no edit path can forget it.
- **Setup:** `Schematic::simulation` (in `schematic.json`, written only when not
  default), changed whole by `SetSimulation` through `Setup` (`simulation_changed`).
  `check_simulation` refuses duplicate ids or names, empty names, bad ranges and DC
  sweeps of non-components. `Probe::Voltage { net, at }` binds to an object first, the
  net name second (`plot::Signal::of_probe(p, conn)`: a renamed net still follows its
  wire); `Probe::Current` to a pin.
- **SI values** (`model::si`): `parse` takes `10k`, `4k7`, `2R2`, `5V1`, `µ`/`u`, `meg`,
  units and trailing words; `M` is mega (SPICE says milli). It refuses `1N4148` (a unit
  word must follow the number). `format` is engineering notation; `spice` writes 12
  significant digits, plain from 0.001 to 10⁶ else an exponent, never a suffix
  (`100e-9` is otherwise `1.0000000000000001e-7`).
- **Netlist** (`sim::netlist::build(project, conn, analysis)` → `Netlist { text, nodes,
  devices, sense, problems }`): nets with pins become nodes; ground is the nets of
  ground-styled power ports (or named `GND` / `0`, or the setup's `ground`), several
  join as `0` with a note, none is an error. Node names are `node_name(net)`: lower
  case (ngspice lowercases vector names), unique case-insensitively, `+5V` → `p5v`.
  Devices go per component in refdes order, named letter + refdes unless the refdes
  starts with the letter (a MOSFET `Q2` is `MQ2`). An unconnected pin is still a net of
  its own (`Net-(R1-1)`); pins of unplaced units get `nc_…` nodes; `.options rshunt=1e12`
  keeps floating nodes solvable (it shows in the ninth digit). Model cards are parsed and
  rewritten (`ModelCard`), shared by name; component overrides make `NAME_DEVICE`;
  different texts under one name are renamed `_2`. VDMOS cards get three nodes, other
  MOSFETs a bulk tied to the source. A SPICE line (`SimDevice::Spice`) names its device
  itself: a letter right before `${REF}` is dropped when the refdes starts with it
  (`V${REF}` for `V1` is `V1`, as KiCad names devices), and `devices` holds that name. `value_number` finds the number of a value or a
  voltage inside a part number (`BZT52C3V3` → 3.3) for `${VALUE}` parameters. Current
  probes insert a 0 V `VSENSE_…` in series with the pin; an operating point does so on
  every pin of every device (cheap there, and a sweep keeps its few probes), so its
  netlist differs from a sweep's. `Netlist::pins` lists every simulated pin
  (`DevicePin`: component, refdes, symbol, pin, key, outer node) for labels (`I(R1.1)`)
  and the device table. Supplies are `VSUPPLY_…` to
  ground. Problems carry the component's symbols for zooming.
- **ngspice output gotchas:** plots come last analysis first (reversed on read); an
  operating point has no scale vector; vector names are `v(node)` and `i(vsource)`
  (older versions `v1#branch`); `No. Points` can exceed the data of a stopped run (the
  reader takes whole points); B-source lines must use subcircuit parameters by bare
  name or through a `.param` in the subcircuit — `{…}` expressions of parameters inside
  a B-source evaluate wrongly in ngspice 47 without an error. `set ngbehavior=ltpsa`
  (LTspice/PSpice compatibility, as KiCad runs it: vendor `params:` subcircuits fail
  without it) must be set before the circuit loads.
- **Worker** (`sim::Worker`, `worker.rs`): the library in a process of our own, `PROGRAM
  ARGS… LIBRARY` with the netlist on stdin: the app is its own worker (`pcb-designer
  --sim-worker`, handled first in `main`), tests use the `sim-worker` binary
  (`CARGO_BIN_EXE_sim-worker`; other crates' tests have none). It runs `Libngspice`
  and answers in tab-separated lines starting with `MARK` (ngspice may print to stdout
  itself): the live plot as it grows (polled every 10 ms), then the plots, log,
  warnings and `done` or `error`. Floats as Rust's shortest round-trip text, so worker
  and in-process results are identical. Stop kills it and keeps the live points; dying
  without an answer is "ngspice crashed". `version` runs one with `--version` (cached).
- **libngspice** (`sim::Libngspice`, `shared.rs`): loaded with `libloading` (never
  unloaded; one `Api` per path), found from the `ngspice_library` setting,
  `NGSPICE_LIBRARY` (both errors when they don't load), then the bundled one
  (`bundled_library`: beside the executable, or in development
  `target/ngspice/<triple>/lib` upwards from it, the triple from `sim/build.rs`), then
  the usual places (Homebrew, Linux `libngspice.so.0`, Spice64's `ngspice.dll`,
  KiCad's bundled one).
- **Bundled ngspice:** `scripts/build-ngspice.sh` builds the release pinned in
  `scripts/ngspice.env` (checksum as Homebrew's) with our patches
  (`scripts/ngspice-patches`) into `target/ngspice/<triple>/` (about 85 s, 5.6 MB):
  shared, XSPICE and KLU; no OSDI, OpenMP, FFTW, readline, X11. Not shipped:
  `table.cm` (GPL, loads into our process), the Icarus bridge. Licences: BSD, XSPICE
  public domain, `numparam` and KLU LGPLv2 (an unmodified shared library is fine),
  COPYING ships with it. Gotcha (patched): ngspice 47's `ft_numparse` tests
  `(double)(int)val == val`, undefined above `INT_MAX`; current clang folds it, so
  `.options rshunt=1e12` became 2.1 GΩ and every result drifted by ~1e-6. One instance per process: a global
  `RUN` mutex serialises runs, the callbacks write into a global `SINK` (never hold it
  across an ngspice call: `SendChar` also comes on the calling thread). A run:
  `ngSpice_Circ` (fails only on "circuit not parsed": "Error on line" is a line ngspice
  drops and goes on), `bg_run`, wait for `BGThreadRunning`
  (`bg_halt` on cancel), then `write '<tmp>/out.raw'` (single quotes: double ones stay
  in the name) read by the raw reader (`write` sorts vectors: compare by name), `destroy all`, `remcirc`. Streaming: `SendInitData`
  names the vectors (`out` → `v(out)`, `v1#branch` → `i(v1)`, the scale moved first),
  `SendData` appends each point to the `Live` plot, `SendStat` ("tran: 42.3%") sets its
  progress. Init: `set ngbehavior=ltpsa`; without its spinit (a library looks in a
  compiled-in folder, Homebrew's isn't found) it has no code models and POLY sources of
  vendor op amps fail, so `codemodel` loads them from `lib/ngspice` beside it.
  `ControlledExit` (a `quit`, or it gave up: e.g. it can't allocate a run's vectors on
  macOS; Linux overcommits and runs on) sets a flag and the run fails with "ngspice
  exited". The instance is spent for the rest of the process: `ngSpice_Init` again
  (with or without `ngSpice_Reset`) leaves it corrupt and later runs crash (macOS
  malloc traps it), so `Libngspice::exited` makes library runs fail and
  `Engine::backend` / `describe` pick a worker.
- **Versions:** the app bundles 47; a user's own library may be older (Ubuntu ships
  45). Before 46 current
  sources have no `r=` (PWL repeat), so a repeating PWL current source is netlisted as
  the I element with its DC and AC parts, plus a hidden repeating V of the PWL less its
  start driving a G of gain 1 beside it.
- **Engine** (`sim::Engine`, from `EngineSettings`: the library path, `process_only`
  and the `WorkerCommand`): `Libngspice::locate` once, then the library loaded in this
  process (not with `process_only`) and a `Worker` on the same path. `backend` is the
  library, but a worker for sweeps (parallel), once ngspice exited here, and with
  `process_only`; without a worker command (`HeadlessHost` default, other crates'
  tests) sweeps run their steps one at a time (`Backend::parallel` false). The app,
  the headless host (MCP) and `simulate` (CLI) use `Settings::engine`; `describe` is
  "libngspice-47, shared library (bundled)" or "libngspice-47, worker process".
- `SimResult` reads by net (`voltage`, ground as zeros) and pin (`current`), `Vector`
  has `db`, `phase`, `magnitude` and `at` (interpolated). `sim::report` gives the text
  summary and CSV of the `simulate` command; its rows are the nets, then per device its
  pin currents and (operating points) `P(R1)` = Σ V·I into the pins (positive: the
  device takes power, a source's is negative). `report::devices` is that table with
  each pin's voltage and current.
- **Built-in models** (`builtin::generate::sims`): a table by part name sets the
  generated parts' models (1N4148, B5819, zener with `BV=${VALUE}`, 2N3904/2N3906,
  2N7002/BSS84 as VDMOS, a Butterworth–Van Dyke crystal with `f=${VALUE}`, ferrite and
  fuse as 0.1 Ω) and adds `VSource` / `ISource` and `Resistor_Sim` / `Capacitor_Sim` /
  `Inductor_Sim` (Simulation category, a "No footprint" variant only, so off the board by
  the existing rule). The `_Sim` passives share the board passives' symbols (a
  symbol's category needn't match its parts': it stays in Passives). The hand-made parts' texts
  (behavioural op amp, LM358, regulators, quad NAND) are `pub` there and stored in the
  file; changing one means setting it again on the part (a one-off program or the app),
  then `gen_builtin`. `every_part_simulates` checks every part has a model with real
  pins.
- **Session** (`sim::Session`, the app's `SimPanel` holds one): one run at a time. The
  netlist is built on the UI thread (it needs the project), only `Backend::run` goes to
  the worker (`SharedBackend = Arc<dyn Backend + Send + Sync>`); `poll` each frame
  collects it (`JoinHandle::is_finished`). `start` abandons a running run first (Run
  while running restarts; its worker ends on its own, the result dropped); `stop`
  cancels and keeps the run until `poll` collects what it has (`Finished::stopped`,
  `range` the whole analysis so the axis doesn't shrink; a stopped sweep is
  cancelled). Dropping the session abandons its run (the library would run on). A
  single run streams into a `Live` (`Arc<Mutex<…>>`); `poll` brings `live(analysis)`, a
  `Finished` with a new `run` number, up to date by appending the new points
  (`Live::update`), every 100 ms, less often past 200k points (the plot rebuilds its
  data from all of them). The plot and the panel's table show it while it runs
  (`range` fixes x; not dimmed), with `progress` as "Running… 42 %" and a bar along
  the top of the plot. Results
  are kept per analysis id with the summary rows (`report::rows`, computed once), and
  dropped by `retain` when the analysis is deleted. **Stale** = the netlist the analysis
  would get now differs in text from the one it ran (so edits that don't reach the
  netlist, like moving a symbol, don't count), rebuilt once per document revision. The
  last `Attempt` (started or refused) keeps the netlist problems plus ngspice's errors
  and warnings (without objects) and the log; Messages' Simulation tab shows it.
- **Simulations in the project tree** (`project_panel::simulation_node`): a
  `NodeId::Simulations` node below the PCB, a `NodeId::Analysis(id)` per analysis in
  setup order (no reordering). Add simulation is an `Entry::Sub` (on the node and the
  header's +); rename (name dialog, `Naming::Analysis`), duplicate (right after,
  `sim_ui::duplicate`) and delete are one `SetSimulation` each. Rows show
  `SimPanel::tree_detail`: progress, "queued", the failed attempt's error count
  (`SimPanel::failed`, per analysis: the session keeps only the last attempt),
  "outdated". Staleness for all analyses is `check_stale`: at most once a second
  (`STALE_EVERY`), only after an edit or a run, not while busy or with stale
  connectivity (`is_stale` caches per revision, so stale connectivity would stick), and
  only while the node is expanded or the panel or a simulation tab is open.
- **Current simulation** (`SimPanel::current`, saved as the workspace's `sim_current`):
  set by a click in the tree (`Act::PreviewAnalysis`), a press in its tab
  (`PlotAct::Focus`), a tab button click, adding, running. No fallback to the first: the
  panel then says to select one, and ⌘R refuses (it adds an operating point only when
  there are none). Probes still fall back to the first sweep, MCP to the first analysis.
- **Output tabs**: `Tab::Simulation(id)` / `Focus::Simulation(id)`, one per analysis,
  `TabRef::Simulation { analysis, preview }` in the workspace. `SimPanel::preview` is the
  analysis in the preview tab; there is one preview tab in the app (`take_preview_tile`
  hands its tile over between library items and simulations). A Run brings its tab to
  the front or shows it in the preview (`open_analysis(id, true, false)`; a preview stays
  one); Run all opens none; agents' runs open none. New tabs go next to other simulation
  tabs, else right of the schematic when it's alone in its group. Tabs of deleted
  analyses close on the revision change (`close_deleted_analyses`). The focused tab's
  analysis is the tree's selection (`focus_node`), as with item tabs.
- **Run all** (`SimPanel::run_all`, ⇧⌘R): a queue that `next_in_queue` (each frame,
  after `poll`) starts one at a time; a run it didn't start is replaced. Its outcomes
  count into `Batch` (only runs it started) and end in one status line. Stop and a
  single run (also the tuner's and an agent's) drop the queue.
- **Panel** (`app/src/sim_ui.rs`): the current simulation's name, Run/Run all/Show/Stop
  with the last attempt's error count linking to Messages, its form, the result table
  (named and probed nets, "Show all nets", a filter; a click selects the net), and
  "Circuit" collapsed (ground, supplies, temperature, extra lines, netlist and log
  window); with the Simulation node selected, Circuit only. Every edit is one
  `SetSimulation`; value fields keep their own text buffer while focused and keep text
  that doesn't parse (red, not committed). ngspice is
  looked for once (when the panel shows or on Run; `SimPanel::engine`) and again when
  a setting changes; the Settings page caches its own lookup per `EngineSettings` and
  shows what's in use (and the executable sweeps use).
- **Plot** (`sim::plot`, headless; drawn by `app/src/plot_ui.rs` in `Tab::Simulation(id)`,
  under `push_id(id)` so tabs side by side keep their widget state apart):
  `signals` = the analysis' probes, or without any the named nets (at most 8, not
  saved). `Signal` is `Voltage`, `Diff`, `Current` or `Power`; `plot::vector` and
  `signal_label` read any of them from a result. `data` makes the traces: AC is dB left
  and unwrapped phase right (dashed, "V(X) phase"); else one axis per unit in the order
  V, A, W: the first present left, the next right, a third on `Axis::Right2` further
  out. `Scale`
  (linear or log) does fractions, zoom around a point, pan and a minimum width (flat
  traces still get an axis). `linear_ticks` divides rather than multiplies for negative
  exponents (3 · 0.2 ≠ 0.6) and keeps ticks at the range ends (0.7 / 0.1 = 6.999…).
  `decimate` keeps each pixel column's first, min, max and last sample in order, plus
  one sample beyond each edge; the widget caches it per x range, width and run (a
  million points draw as a few thousand). The view (`PlotState`, per analysis, in
  memory) fits y to the visible part while the user hasn't zoomed y; a rerun keeps the
  zoom when the x range is the same. The table under the plot is sized to its rows
  (constant row heights in `plot_ui.rs`, so nothing in it may come and go with the
  cursors: the cursor hint sits in the tool bar; at most 40 %) until the handle between
  them is dragged: then `SimPanel::table_share`
  (a share of the height, for every analysis, in memory; double-click resets it).
  Right-click opens a menu (Clear cursors, with the Cancel key from the keymap; a
  right drag still pans, egui opens menus on clicks only), and the hover readout hides
  while it's open (it covered the menu). Cursors snap to samples, the hover readout
  interpolates. Box zoom starts at `press_origin` (egui reports drags some pixels
  late). The tab follows the panel's analysis; the first finished sweep opens it (right
  of the schematic when that tab is alone in its group, else in its group), unless the
  user closed it (`plot_closed`). Ticking a row in the panel's result table adds or
  removes a `Probe` (one `SetSimulation`). Trace colours: `style::trace_color`, an
  8-colour categorical palette per theme checked for colour-vision deficiency; past 8
  colours repeat dashed; a probe's colour is its index. Missing signals carry a reason
  (`plot::Missing`, `missing_reason`: net gone, not simulated, pin not in the model,
  run again); right-click on a readout row removes its probe, hovering one highlights
  its net or pin on the schematic. The tab has the canvas background (like the schematic), the
  chart `plot_bg` (the window background in dark, white in light: the light window grey
  hid the grid and left yellow at 1.5:1), `plot_grid` and `plot_text` from the canvas
  palette, so the offscreen plot (MCP) matches.
- **On the schematic** (`view::sim_overlay`): `SchTool::Probe` (O) is handled by the
  app (`probe_ui`), which knows the target analysis (`SimPanel::probe_analysis`: the
  panel's if a sweep, else the first sweep). `probe_target` takes a wire, label or
  junction's net, else the nearest pin whose body (not its end) is within the
  tolerance; with Alt the symbol under the pointer (`ProbeTarget::Part`, power). The
  probe toggles on release (one `SetSimulation`): a press on a net remembered in
  `SimPanel::probe_drag` and released on another net makes `ProbeTarget::Diff` (a
  rubber band shows meanwhile). `Probe::Voltage { reference: Option<NetRef> }` is the
  differential probe, resolved like the net; `Probe::Power { component }` makes the
  netlist sense every pin of that device in sweeps, and `SimResult::power` sums V·I
  (none for AC). The overlay draws a differential probe's two nets in its colour and a
  powered part's units framed (symbol bounds from the view's index). The tool group
  (Probe, Run) shows while the Simulation panel or tab is open. `SimOverlay` (probed
  nets and pins with colours, voltage tags, two-pin currents as (symbol, pin the
  current enters, text), `dim`, hovered net/pin) is built by `SimPanel::overlay`
  (cached on revision, run, probe analysis, hover and theme) and handed to
  `Document::set_sim_overlay`, which redraws each sheet's `SIM_ITEM` on change, with
  the connectivity and on restyle. Probes and values go on the schematic's
  "Simulation" layer (in the layer menu, so its visibility is saved with the
  workspace; `View::drawing` leaves it out of prints), hover marks on the hover layer. Everything shows only while the panel or tab is
  open. Values come from the panel's operating point, else the last one run;
  voltage tags sit at a net's first label on the sheet, else the middle of its longest
  wire there; ground gets none; current tags reach away from the part body. Tags use
  `sim_value` / `sim_value_bg`, three significant digits, and are world-sized. Parts
  with more than two pins get a hover tooltip (pin V, I, power). `SimPanel::hover` is
  cleared each frame before the tree draws and set by the Probe tool or a hovered
  table row.
- **Measurements** (`sim::measure`): over a window with interpolated ends; average and
  RMS integrate linear pieces exactly (uneven transient steps weigh right); frequency
  from mean crossings with 10 % hysteresis, upward ones when there are two, else every
  crossing as half a period; rise and fall 10–90 % of the window's range, the first
  edge that starts below 10 %; AC peak, −3 dB points either side (interpolated in log
  f) and the phase margin at the first downward 0 dB crossing. `measure::table` is the
  plot's Measure table (cached per range and data in `PlotState`); its range is the
  cursors when both are set, else the visible x. Agents get the same numbers.
- **Distortion and spectrum** (`sim::spectrum`): our own, not ngspice's `.four` (printed
  text, fixed to the netlist, blind to cursors) or `.disto` (small-signal, unreliable).
  `distortion` takes the fundamental from `measure`'s crossings and the last whole
  periods of the window (the most settled; ngspice's first sample lands a little after
  the start, so a 10-period run measures 9), then integrates each harmonic exactly over
  the linear pieces (by parts: the value terms cancel between pieces, the slope terms
  use sin(x)/x so short steps don't cancel out), so it needs no resampling or window
  function. Harmonics up to the 10th, below half the mean sample rate. THD+N is
  `rms_ac² − fundamental²` of the same interpolant (Parseval), so it shows the floor
  linear pieces leave: (ωΔt)²-ish, about 0.015 % at 100 steps a period. The spectrum
  view resamples evenly to a power of two (2^10…2^18), Hann window, our own radix-2 FFT
  (no dependency), dB of RMS. In the tab, Spectrum is per analysis (`SimPanel::spectrum`)
  with its own `PlotState` (`spectra`) so the time view keeps its zoom and cursors; its
  window is the time view's cursors, else its visible x, cached in the time state;
  Measure there shows `spectrum::table` of the time traces over that window
  (`Measuring`). MCP: `distortion` per transient signal, `screenshot` `spectrum`.
- **Panel extras:** `suggested_supplies` lists power port nets that no supply and no
  voltage source pin powers, with `supply_voltage` from the name (`None` for `VCC`, the
  user types it).
- **Tuner** (`sim::tune`, `app/src/tune_ui.rs`): rows are `Simulation::tune`
  (`TuneParam` = a part's value or a source's field by key, from `Source::fields_mut`;
  the setup check refuses a parameter twice, min ≥ max, and min ≤ 0 on log rows). The
  slider values live in `SimPanel::tuner` (`Tuner`, not saved); `Tuner::changed` is
  what differs from the document, and `Overrides` (component copies with the values
  put in) is what `netlist::build_with`, `Session::start_with` and `is_stale` take, so
  a tuned run isn't stale and an untuned one is. A part's value is stored as its value
  text reads back (`si::value_text`), so Apply (one `Update` of the override
  components) leaves nothing tuned. Re-runs: on release or a typed value, while
  dragging when the tab's last run took under `tune::LIVE`; `tune_run` skips a run whose
  netlist matches the last and queues one (`tune_pending`, started from the app's
  frame) while a run goes. The tuner's height doesn't depend on what is tuned (the
  "Tuned:" summary sits in its header) and rows `push_id` their parameter: a layout
  change under a dragged slider ends the drag. "Tune R1" in the schematic's context
  menu takes a symbol or its field; a source's level (value, amplitude, high, v2)
  first. Agent runs use their `values` only, not the tuner's.
- **Parameter sweeps** (`Analysis::step`: a `TuneParam` and `StepValues` list, linear
  or log, at most `MAX_STEPS`): `Session::start_with` builds a netlist per value
  (`Overrides::try_with` on top of the tuner's, refused when the parameter is gone) and
  runs them on `min(steps, cores)` worker threads pulling from a shared index; a failed
  step cancels the rest and names its step. `Finished` keeps the first result as
  `result` (everything that ignores steps keeps working), the others in `rest`, the
  values in `step`, and `base` (the netlist without the step) for the stale check;
  the tuner's overrides leave the swept parameter out (`SimPanel::overrides(project,
  analysis)`), and its row is disabled. `plot::data_steps` merges the steps onto the
  union of their x samples (transients step differently; the first step's x past
  400k), traces by signal then step, labelled "V(OUT) · R1 = 4.7 kΩ"; `PlotData::color`
  gives a colour per step for one signal, else per signal with steps faded towards the
  chart. The readout puts a header row before each signal's steps (hides them all).
  An operating point gets a column per step; sorting by one shows its values on the
  schematic (`PlotState::sort`, in the overlay key). Phase margins pair magnitude and
  phase traces of the same step.
- **Model editing** (`app/src/sim_form.rs`): the parsing lives in `model::simulation`
  (`logical_lines`, `ModelCard`, `SubcktHeader`, `SimModel::problems`, `SimDevice::ports`)
  so the netlister, the library's issues and the forms agree. `Library::sim_pins` is the
  explicit mapping or every unit's pins; `sim_issues` adds pins mapped twice or not on
  the part, skips board parts (no units); `issues()` lists them as "simulation model:".
  `auto_map_pins` matches port names and aliases, then fills with unused pins; a
  subcircuit whose ports change is mapped again. Switching the device keeps the text (a
  primitive without text gets `generic_card`). `ComponentForm` (Properties) edits a
  clone of the component, committed as one `Update`; overrides list the model's
  parameters with the part's values as hints, empty = no override; PWL is `t v, t v`
  text. `SiFields` keeps the text being typed and the invalid ones (shared with the
  panel).
- **Agents** (`mcp::simulation`): `simulate` is prepared in `handle` (analysis by name
  or id, else the host's shown one; signals parsed against a fresh netlist: `R1.1`,
  `I(R1.1)`, `OUT`, `V(OUT)`) and run by the host through its `Session`
  (`start_with` + extra current probes, which `Finished::extra` keeps so the result
  isn't stale for them) and `values` (`parse_values`: `R1`, a DC source's `V1`,
  `V1.amplitude`, checked against `tune::tunable`) as `Overrides`. `check` after each poll answers: done, failed, stopped (by the
  user, another analysis started), or 120 s passed (the server waits 130 s for it,
  other calls 60 s). The app answers from `SimPanel::poll`; the headless host blocks
  in a poll loop with its own session (`sim_backend` lets tests fake ngspice, else its
  `engine` settings).
  Replies round to six significant digits; sweeps' points are `decimate`d over 49
  columns (≤ 200 points). The screenshot draws the plot as a `render` scene in pixels
  (y up, a camera at zoom 1), text with the canvas font, dashes cut by hand.
- **Demo:** a "Testbench" sheet with `V1` (5 V + 0.5 V 1 kHz sine, AC 1 V) on +5V
  replaces the supply; analyses Operating point, Transient (3 ms) and AC, the last two
  probing +5V and the LED's anode.
- **Tests:** netlists and the raw reader (ngspice 47 fixtures in `tests/fixtures`) run
  everywhere; the circuits in `tests/all/ngspice.rs` (RC in AC and transient, diode, op
  amps, regulators, transistors, NAND, zener, crystal, DC sweep, errors, cancel, the
  demo) run by every backend found, which must agree (same vectors and values), and
  skip with a note when none is; library-only ones stream and stop, refuse to run after
  ngspice exits (a `.control` `quit`; that test reruns the test binary as a child
  process, which would otherwise spend the other tests' library), and serialise runs from threads (they queue:
  tests waiting for the library allow for the others' runs). The KiCad demo comparison also
  runs our netlists through the library. `tests/all/bench.rs` builds circuits
  by labelling pin ends. `tests/all/plot.rs` covers scales, ticks, decimation and AC
  traces on a made-up result. `tests/all/session.rs` drives the session with a fake backend
  that waits for a gate (start, refuse, fail, stop, restart, stale).`mcp/tests/all/simulation.rs` covers the tools with a fake
  transient backend, and with ngspice (skipped without it).

## Rendering (`render`)

- Instanced primitives in one 32-byte instance: capsule, circle/ring, rounded rect, arc,
  triangle. Triangles mark outer edges (anti-aliased); inner edges are hard so polygons
  show no seams (but a fan's hairline gaps can show: see the app icon).
- Polygons: ear clipping (`render::tess`) for small ones, i_triangle for zone fills
  (holes, O(n log n); earcut took 1.9 s on a fill with thousands of holes).
- Text (`font`): JetBrains Mono Regular and Bold, embedded (`render/fonts`, SIL OFL;
  the app stays MIT/Apache). `ttf-parser` reads outlines; each glyph is flattened (2
  font units tolerance), cleaned with i_overlay (non-zero) and triangulated with
  i_triangle once, cached per weight and char (`glyph`), and placed per text
  (`Placement`: anchor, cap height, rotation, alignment, weight, mirror) as triangles
  (`text_triangles`, the view) or outlines (`text_shapes`, Gerber). `Text.size` is the
  cap height; lines are `line_spacing()` apart (the font's ascender − descender + gap);
  Bottom = last baseline, Top = first cap top. `text_box` (advances, descender to cap
  top) is the hit box and the editor's frame. No shaping or kerning; missing chars get
  `.notdef`. Both weights share the layout functions (monospace: same advances;
  a test checks). `Text.bold` replaced the stroke width: board and footprint texts
  default to Bold, KiCad board imports are Bold, schematic ones follow `(bold yes)`.
  Cost: a glyph is ~20–60 triangles, so texts are most of the stress scenes' instances
  (see Performance).
- Small details hide when zoomed out (`LayerDesc::min_zoom`), and a layer can hide
  zoomed in (`max_zoom`). Per-layer buffers; only dirty items upload;
  pan/zoom is a uniform.
- `Scene::set_item` gives up a shrunk item's tail as garbage (else `instance_count`
  drifts from a rebuild). `Scene::snapshot` doesn't consume the incremental upload.
- `Offscreen` renders to PNG (tests, MCP screenshots, `render_board`, the app icon). Its
  background is opaque. Each one is its own wgpu instance and device, without
  `InstanceFlags::DEBUG`: with it (debug builds) the Vulkan loader's
  `vkSetDebugUtilsObjectNameEXT` segfaults when threads create devices at once (parallel
  tests on Linux CI, Mesa lavapipe); `renderers_are_made_in_parallel` catches it there.

## Views and editing (`view`)

- `View` draws one schematic sheet or the board into a `Scene` incrementally; overlays
  (selection, hover, highlights, DRC markers) are separate items. A view is identified
  by `ViewId` (`Sheet(id)` or `Board`); `draw_object` skips objects of other sheets, so
  every path (apply, highlights, probes, previews) filters by sheet for free. Highlights
  leave out the fill layer and cover bodies with a translucent rectangle instead. An R*-tree does hit testing (thin
  beats body, small body beats large) and box selection. `ItemView` does the same for a
  single symbol or footprint.
- The drawing frame (`frame.rs`): paper edge, border with zone grid (zones divide the
  border evenly, about 50 mm each) and the title block (110 × 32 mm, close to KiCad's
  default so text KiCad users put in its comment area lands there here too). One scene
  item on the "Frame" layer, not in the R-tree (not selectable); redrawn on any sheet
  change, since it shows the title block, sheet names and the page count. Fit to view
  includes it (scene bounds).
- Pad numbers on the board: 45% of the pad's smaller side, at most 70% of the
  footprint's `${REF}` text (1 mm without one: `pad_number_limit`), so exposed pads
  don't get numbers larger than the part's label. The footprint editor isn't capped
  (pads redraw one by one there; a cap from the reference text would go stale).
- Net labels (`net_labels.rs`, `View::update_net_labels`, called by the app every frame
  with the viewport): one overlay item drawn only for the area around the viewport (half
  a viewport more each side), from the view's R-tree, so a large board stays cheap. It's
  redrawn when the viewport leaves that area, the zoom crosses a 15% step, the board or
  its copper changes (`apply_objects`, `redraw_users`, `set_copper` and `rebuild` drop
  it) or a copper layer's or "Pad numbers"' visibility changes. From 25 px/mm, straight
  tracks get their net along them (upright, 0.6 of the width, at least 8 px; at the
  middle and at fixed places either side, so labels don't swim as the view pans; not
  where the text would touch a pad on the layer). Each copper layer has its own label
  layer right above it, so higher copper covers a lower track's name. From 50 px/mm the
  labels draw the pad numbers too (the static "Pad numbers" layer has `max_zoom`), with
  the net below the number on pads that have one, along the longer side, both shrunk to
  fit the pad. The label layers aren't user layers (not in menus, prints or saved
  visibility). Highlights leave the pad numbers out (a copy would sit beside the
  labels').
- Library edits redraw only their users (`View::users_of`): symbols depend on symbol,
  part and the component's footprint (pad names); footprints on footprint and part.
  Edits that don't change drawing (name, prefix…) redraw nothing.
- `Document` = editor + folder + a view per sheet + the board view. `changed` adds or
  drops sheet views when sheets come and go (commands, undo); `view(id)` of a missing
  sheet returns an empty view. `LibraryDoc` wraps a document whose
  project holds only a library; the Project Library's `LibraryDoc` wraps the project
  document itself (one undo history).
- `Interaction`: click/shift/cmd select, box (inside, or touching with Alt; modifiers
  count at the release; what it would select is highlighted live, searched again only
  when the box changes; `View::select_rect` leaves out reference and value fields and
  adds the drawn ones of each symbol or footprint it found; areas, `view::Area`: zones,
  keepouts and filled board drawings are hit on their outline like lines, inside (the
  polygon, minus holes) after everything else, and `Target::grips` says a drag from
  inside doesn't move them: the press then selects only on release, and a drag boxes
  unless the area is selected with other objects; unfilled board drawings (the board
  outline) are hit on their line only, `view::stroke_dist`, shared with the part editors;
  a touching box takes areas and board drawings only where it meets their line,
  `outline_segments`, or when it encloses them), drag with snap of the grabbed reference point (one undo step; R rotates while
  dragging), `snap` toggle. Locked footprints don't move. A press becomes a drag when
  the *hand* moves the pointer 4 points on screen (`PointerInput::screen`), not when the
  view shifts under it (a panel changing width); the drag then follows the hand's motion.
- A drag *replays* itself: each step retracts the previous one (`Target::retract` →
  `Editor::retract`: pops the open group without a redo entry) and runs the whole
  rotation (about the start anchor) and offset from the start. So what stays attached
  is decided by the geometry at drag start (passing over wires never picks them up),
  doglegs never pile up, and a drag equals one MCP `Move`. Carried pastes still move by
  steps (their group holds the paste). Esc retracts too. Gotcha: a step costs an undo
  plus a full move (~2 ms for a symbol with wires on a 10k sheet).
- `Target::drag_axis`: a single straight segment (horizontal/vertical wire, any straight
  track; 45° as a unit diagonal for whole grid steps) moves only across itself.
  `Target::chain` + `Interaction::double_click`: wire and track chains
  (`query::wire_chain` / `track_chain`). A double click can arrive while the second
  press is still pending (script input): it uses the press up.
- The GUI sets `keep_connected` for every move, rotate, mirror and flip except carries;
  the commands default to false (MCP gets exact moves unless it asks).
- Tools: `tools::Placer` (library: pin/pad, line, rect, circle, arc, polygon, text; names
  auto-increment; arc: centre, start, then the end follows the pointer with `sweep`
  accumulated from angle deltas so it can pass 180°, `/` (`flip_arc`) takes the other way
  round; polygon: closes on a click on its first or last corner, ≥ 3 corners; the drawing
  tools snap to `Content::snap_points`, Alt grid only), `sch_tools` (wire, bus, bus entry, label, power, junction, no connect, place;
  `bend` for corners; `SchPlacer::sheet` is the sheet they work on, `set_sheet` ends a
  wire begun elsewhere; the Sheet tool places a new sheet's symbol with one
  `AddSheet`; the connecting tools snap like routing does, `SchPlacer::snap` →
  `snap_to_connection`: the nearest pin end, wire end, label, junction, bus entry end
  or sheet pin within 8 screen points, else the grid point projected onto a wire under
  the pointer, else the grid; the bus tools take buses instead of wires; candidates
  from the sheet view's R-tree; Alt grid only, decided by the app), `pcb_tools` (route with copper snapping and the checks above: a clash
  turns the ghost to the flagged colour with a marker and refuses the click unless
  `allow_violations`; L changes layer + via (through, or the two layers with
  `via_pair`); via (span `via_span`), outline, zone, text (layer, F → `LayerId::
  other_side`), measure). The Text tools hide their ghost while an edit is open. Bus naming prompts: `SchPlacer::take_prompt`
  returns a `SchPrompt` for unnamed buses and entry signals.
- `bend`: corner styles (`Corners`: wires 90°/45°/free, tracks 45°/90°/arc 45°/arc
  90°), the two-leg `path`, `segments` (arc styles round the corner with
  `fillet(…, None)`: `AddTrack.mids`), and `Posture`: the first leg follows the way
  the raw cursor leaves where it was at the leg's click, decided once it's `DECIDE`
  (40) screen points away (45°: diagonal when more than 22.5° off the axis), held until
  it's back within those 40 points (which clears it). Points are logical, so the
  distance is the same on HiDPI screens. The app feeds the raw cursor and the zoom to
  the placers' `steer` every frame, also before a leg (the click's position is the
  origin); the snapped cursor said little (one grid step zoomed in, pixel jitter zoomed
  out). `/` flips it until the next leg (or swaps a fixed `Bend`). Placers call
  `Posture::start` on every leg click, so a flip before the first click is lost.
- `measure`: the measure tool's state (`Measure`: click, live end, click pins, next
  click starts anew, Esc clears) and snapping (`snap`: nearest object point within the
  pixel tolerance, else the grid point; Alt free). Object points: `board_points` (pad and
  via centres, track ends, board graphic vertices), `footprint_points`,
  `GerberView::snap_points` (flash centres, stroke ends, via the R-tree). One
  measurement per editor, kept in `PcbPlacer` / `Placer` / `GerberPane`, cleared by
  `set_tool`; not saved, no commands. ⌘C copies `Reading::text` when the focused
  editor's Measure tool shows one (`App::measure_reading`), before object copy. The app draws it (`canvas::draw_measure`).
  Snapping inside a pad, via or Gerber flash goes to its centre (explicit points within
  the tolerance win). On the board, `BoardSnap` keeps the points and pad/via outlines in
  R-trees, rebuilt when the document revision changes.
  Clearance mode (`settings.clearance`, E): `measure::Clearance` picks two
  `editor_core::clearance::Feature`s (copper on the active layer, pad and via holes, the
  board edge) and `clearance::gap` gives the distance, nearest points (`Geo::nearest`)
  and the rule (copper of different nets on a common layer, or copper to the edge).
- `handles`: corner handles of one selected zone, keepout or board drawing, and of one
  selected graphic in the part editors (`shape_*` functions on a `Shape`; the object
  functions delegate board drawings to them). Polygons
  (holes too): corners move, midpoints insert, Delete / double click removes (≥ 3 stay).
  Rectangles keep their shape (corner: opposite corner fixed, edge: moves); "Convert to
  polygon" in the context menu. Lines: both ends. Circles: a radius handle. Arcs: both
  ends (centre and radius fixed, the sweep changes) and a midpoint (radius changes, the
  ends stay put). The app (`corner_handles`) runs a drag
  as `Update`s in one undo group from the original object, snapping like Measure (a
  `BoardSnap` built once per drag), Alt free, Shift 0°/45°/90° to the neighbours. The
  library editor does the same in `ItemEditor::graphic_handles` with `update_item`.
- `array`: "Create array…" (⌘⇧D, context and Edit menu) on one selected pad or pin.
  `slots` lays out Linear, Dual row (the second row 90° counterclockwise of the first's
  direction, running back: numbers go round counterclockwise), Grid (selected pad top
  left, columns →, rows ↓ lettered without I, O, Q, S, X, Z; continues from a name like
  "C3", else starts at A1 and renames the selected pad) and Quad (counts and spans for
  sides and ends; end pads turned 90°; an exposed pad isn't part of it). Names go on with
  `increment_name`; pins: Linear only. `apply` is one `update_item` (one undo step); the
  selected element stays first and keeps its id. Clashing names are allowed, the dialog
  lists them (`clashes`). In the app it's a mode, not a tool: `ItemTab::array`
  (item, element) shows the form (`array_form.rs`, one for pads and one for pins,
  remembered like the tool settings) in the Tool settings panel, which the app opens and afterwards gives back to
  the panel it replaced (`App::array_dock`). The copies are the canvas preview; Enter
  (unless a field had focus this or the last frame) creates, Esc, another tool, another
  item or a selection change cancel.
- `clipboard`: copy (anchor = snapped reference point of the first copyable object) and
  paste into a view, checking the clip's domain. A paste is carried:
  `Interaction::carry` is a drag without a button in the paste's undo group (moves,
  R, X join it; a click drops; Esc undoes the paste). Carried edits are wrapped in
  `Edit::Carry` so schematic wires the paste passes over aren't rubber-banded.
  `clipboard::paste` returns only what landed in the view (not copied sub-sheet
  contents, nor footprints sync stages for pasted components), so only that is carried.
- **In-place text editing** (`text_edit`): `TextEdit` (text, caret and anchor as char
  indices, `TextTarget` says what a commit changes) is GUI-independent; `geometry`
  maps the font layout (`font::caret_positions`, `line_origins`, y-up local coordinates
  rotated about the anchor) to world quads for the frame, selection and caret, and
  `index_at` back. `Document` holds the one edit (`EditSite`: a view, or an `Item` of a
  library editor, whose `ItemView` pulls `Document::item_text_edit` each frame) and
  draws it over the text: a frame filled with the background hides the original.
  The edit's shape comes from what the painter recorded (`Painter::texts`, via
  `View::object_text`), so every kind of text edits where it is drawn. `commit`
  returns one command (free text: update, or delete if emptied; label, sheet: rename;
  reference, value: `SetComponent`; new texts: insert unless empty; library texts:
  update, removed if emptied); unchanged commits nothing. Double-click starts an edit;
  text tools start one with a `New*` target. Gotcha: everything that can change the
  document (actions, tool switches, library item switches) commits first (`App::
  commit_text_edit`), and the app feeds keys to the edit before shortcuts, consuming
  plain keys and ⌘Z/Y/C/X/V (no undo inside an edit).
- Owner highlight: a selected (or dragged) field draws its owner in the hover colour
  and a tether line from the owner's origin to the text (`View::draw_overlay`).
- Selected tracks and vias aren't drawn in the accent colour (blue, which reads as the
  bottom layer) but in their own, opaque and slightly brightened, inside a rim
  `SELECTED_RIM` wider, mixed strongly toward `selected_copper` (`style::selected_copper`;
  white in the dark theme, black in the light one, like `net_highlight`). The net
  highlight tints evenly, so the selection stands out of its net by the rim.
- Labels are drawn KiCad style: local with a dot, global as an outline around the name,
  hierarchical as a small arrow before it; `LabelShape::points` says which ends are
  pointed. Label text goes through `Text::upright` (turned 180° with swapped alignment
  instead of upside down or reading downwards).
- `cross_probe` (pure): selection ↔ symbols/footprints, wires ↔ copper of their nets;
  the routed net wins. Probes from the board go to every sheet view.
- Themes: `style::Palette` (`DARK`, `LIGHT`), baked into scene instances. The app's
  theme is a global (`set_theme`, `app_theme`); each `style::Editor` (Schematic, Pcb,
  Simulation) may have its own (`set_editor_theme`, `None` follows the app). Drawing
  code reads `pal()` / `theme()`, the palette in effect on the thread: a forced
  `with_palette` (prints), else an editor's `scope` (an RAII guard), else the app's.
  Every `View`, `ItemView` (a symbol the schematic's, a footprint the PCB's) and
  `GerberView` method that draws opens its editor's scope, so callers needn't; code
  outside them that bakes colours (text edits started by tools, `SimPanel::overlay`)
  uses `with_editor`. `layer_color` reads the theme in effect, so a light board in a dark
  app gets the light theme's layer colours. `Document::restyle_editor` rebuilds one
  editor's views with their overlays (`restyle` both, and only then drops the pending
  library redraws); `ItemView::sync` redraws by itself when its theme changed (its
  drawn key is revision and theme). Gotcha: the chrome colours (`app/src/theme.rs`)
  follow `theme()` too, so a scope must wrap only canvas drawing, never egui widgets
  meant to stay in the app's theme (the simulation tab is the one place that wants
  both: it sets `theme::visuals()` on its `Ui` inside the Simulation scope; popups use
  the context's, the app's).
- `project_libs::sync` loads listed library files (failures become `MissingLibrary`),
  keeps unsaved or file-less ones, refuses duplicate library ids (`check_new`);
  `after_save` relinks or adds a saved library; `copy_for` copies with a new id.
- `tree`: toolkit-independent tree model (`Node`, `TreeState`, keyboard, search that
  restores the expansion afterwards).
- Example projects (`demo::Example`: File → Open example, `--example NAME`).
  `led` (`demo_project`, plus a demo library the app writes to the temp folder) is built
  in code. `amp` (a discrete class-AB amplifier) is **stored**: a project folder
  `crates/view/demo/amp/` (the four files, no `.pcb-designer/`), embedded
  with `include_str!` (`Example::stored`) and parsed by `model::io::project_from_files`.
  It opens as a new, unsaved project, so it must use no external library files.
  `demo::tests` checks every stored example loads, validates, its net classes name
  existing nets (a renamed label leaves stale names), and that saving it writes its files
  unchanged (current format; `.gitattributes` keeps them LF, since Windows checkouts
  would otherwise get CRLF and embed it). After a format change, `cargo run -p view --example
  save_examples` re-saves them. To change one: open it, edit, Save As anywhere, and copy
  the four files over (not the `.pcb-designer` folder). `mcp/tests/all/examples.rs`
  keeps the amplifier ERC and DRC clean and, with ngspice, checks its bias (OUT ≈ 6.6 V),
  gain (≈ 12 dB at 1 kHz) and distortion. Look at the board with
  `render_board --example amp`.

## Libraries and KiCad (`kicad`)

- Pad openings: `Pad::mask_margin` / `paste_margin` (`None`: the board's
  `mask_expansion` / `paste_margin` rule) on top of the `mask` / `paste` flags;
  `fab::mask_margin` / `paste_margin` resolve them for Gerber and the board view. The
  footprint editor draws them on its Solder mask and Paste layers with the document's
  rules (the project's in the Project Library, defaults elsewhere); Properties edits
  them for all selected pads (the field shows the rule until typed, ↺ resets).
- Imports `.kicad_sym`, `.kicad_symdir` (KiCad 10) and `.pretty` (KiCad 6–10). Units
  from sub-symbols, `extends` merged, power symbols skipped (they are power labels
  here), symbols with hidden `power_in` pins refused (KiCad joins those implicitly to the
  global net of the pin name; none in KiCad's current libraries), other hidden pins shown
  with a warning, footprints y-flipped; `B.*` items and `B.Cu`-only pads become `back`
  ones, a pad's mask/paste come from its layer list, its `solder_mask_margin` /
  `solder_paste_margin` become `mask_margin` / `paste_margin` (a footprint-level value
  fills pads without their own; a paste ratio is warned about and dropped). A symbol requested without footprints gets a "No footprint" variant
  (with a warning); one whose footprints all fail to load makes no part. Deterministic ids (FNV of library, name, element), so re-imports
  update in place.
- Previews (`kicad::preview::PreviewCache`): the first unit of a symbol or a footprint,
  converted as an import would (power and refused symbols too), cached by name with
  errors (LRU, 32 items). The import window shows them from a scratch `LibraryDoc`,
  rebuilt when the target changes; it follows the last clicked search result or lone
  footprint, else the active pick. A clicked symbol result shows its first suggested
  footprint. The cache is dropped on rescan and when the window closes.
- Import window layout: the search and selection columns get exactly the height left
  between the rows above and the import row (lists scroll inside), so expanding
  anything never grows the window. The import result (summary, failures, notes) opens
  in the same report dialog as a project import (`ImportReport`).
- The catalog indexes names only (~0.1 s) and parses lazily. Footprint suggestion: the
  default footprint, else the best `ki_fp_filters` match. `Catalog::ranked_footprints`
  orders up to 300 filter matches by `Fit` (exact, extra pads, missing pins), then
  filter, pad count difference, everyday packages; it reads their pads (cached, ~30 ms
  for 180 footprints). A pick lists them with check boxes (one variant each), hiding
  those missing pins unless asked or none fits; long names truncate so nothing widens
  the window.
- Import window lists (`nav_list.rs`, cursor moves in `view::list_nav`): rows select on a
  click anywhere, double click / Enter activates (add a result, toggle a footprint;
  Space too), ↑↓ Home End PgUp PgDn move, Esc returns to the search field, whose ↑↓ and
  Enter drive the results. One selection (`Selected`) across the lists is what the
  previews show (captioned with the names); without one they show the active pick.
  Adding keeps the row selected, so ↓ Enter ↓ Enter adds several. An added item (or an
  already picked symbol added again, which becomes active) is scrolled into view
  (`Reveal`, the next time the list is drawn). A list's focus is a
  widget under its rows with a focus lock filter, as in the Project tree.
- Download (`kicad::download`): official GitLab release archives of `SUPPORTED_MAJOR`
  (10), unpacked into `<config>/kicad/<version>` via a temp folder and rename. Gotcha:
  cancel has its own error; `io::ErrorKind::Interrupted` would be retried forever by
  `std`'s and `tar`'s read loops.
- Verified against the full KiCad 9 and 10 libraries (`kicad/tests/all/corpus.rs`, enabled
  by `KICAD_SYMBOL_DIR`/`KICAD_FOOTPRINT_DIR`). 3.6% of parts have a pin without a pad,
  from KiCad's own data.
- Project import (`kicad::import_project`, `sch.rs` + `pcb.rs` → `project.rs`): a
  `.kicad_pro` with its `.kicad_sch` (KiCad 6+, `MIN_VERSION` 20211014; a legacy `.sch`
  fails with a message) and `.kicad_pcb` becomes a new, unsaved `Project`, checked with
  `validate`, plus a `Report` (warnings, notes, skipped features with counts). Entry
  points: File → "Import KiCad project…" (report window), a `.kicad_pro` argument,
  `pcb-designer import-kicad PRO OUT`, MCP `import_kicad_project` (the app asks about
  unsaved changes first; the agent gets an error if the user keeps the project).
  - Parts: one per symbol `lib_id` from the schematic's `lib_symbols`, all local to the
    project; one variant per distinct board footprint (content key: pads, sorted
    graphics, non-field texts with rotation mod 180). Footprints edited on the board
    become `Name (2)`. Symbols not on the board get a "No footprint" variant; footprints
    without symbol become board parts (duplicate or `REF**` references renumbered).
  - Schematic: KiCad files are y-down (`common::point(.., true)`), libraries y-up.
    `(mirror y)` → (−angle, mirrored), `(mirror x)` → (180 − angle, mirrored). Single
    `power_in` symbols become power labels (style from `power_style`), `PWR_FLAG`
    (a single `power_out` pin) becomes a power flag. `.kicad_pro` `erc.rule_severities`,
    `erc.pin_map` (KiCad's 12 pin kinds folded onto our 9) and the board's
    `rule_severities` map onto `ErcSettings` / `DrcSettings` where a rule matches. Every sheet instance becomes a sheet (reused files get ids derived from the
    instance path); sheet pins are laid out by name here, so wires ending on KiCad's pin
    positions are stretched to ours. Refdes from the symbol's `instances`, else the root
    `symbol_instances` (KiCad 6), else the Reference property. Each file's `paper`
    becomes its sheets' page and `to_pages` lifts the content by `common::page_lift` (the page
    height rounded down to the 1.27 mm grid: KiCad pages hang down from the origin and its
    grid starts at their top left). The root file's `title_block` is the project's.
    `common::plain` also undoes KiCad's `{slash}`-style escapes. Symbol Reference and
    Value properties (absolute positions) become `SymbolField`s in symbol coordinates
    (`Transform::inverse_point` / `inverse_angle`), hidden as KiCad hides them.
  - Board: `aux_axis_origin` (KiCad's drill/place origin) becomes our origin. Pad and
    text angles in `.kicad_pcb` are absolute (minus the footprint's
    orientation). Back footprints are stored mirrored: un-mirrored (negate y and angles,
    swap F./B. layers and chamfer corners), then placed with rotation + 180 on the
    bottom. Reference/Value become `${REF}`/`${VALUE}` templates plus per-instance
    fields. Zone nets are resolved through the pads of the KiCad net (KiCad names like
    `Net-(R3-Pad1)` differ from ours), else kept with a warning. Rules from the Default
    net class and board setup; the other classes with their nets (KiCad 6: `nets` in
    the class; 7+: `netclass_assignments` by name, a string or in 9 a list, and
    `netclass_patterns`). KiCad net names are kept, so they match ours only where the
    import keeps the name (labelled nets).
  - **Simulation** (`kicad::sim`): KiCad's `Sim.*` fields become the part's `SimModel`
    and the component's `ComponentSim`, following KiCad's own rules (`eeschema/sim`):
    only the instance's fields count, plus `Sim.*` the library symbol has and the
    instance lacks (never the library's `Spice_*`); KiCad 5 `Spice_*` fields are
    migrated first; R/C/L/V/I with two pins and no fields are inferred from the
    reference and value; a V/I with type DC takes its level from the value; a value
    that isn't a number makes an R/C/L behavioral (`r='expr'`). `Sim.Pins` maps symbol
    pin numbers to KiCad's model pin names (`+ -`, `A K`, `C B E`, `D G S`, subcircuit
    ports, or positions for numbered ones); without it, pins in sorted number order.
    Parts of one symbol whose models differ are split (`Q_NPN (Q2N2222)`, ids from
    `lib_id#2`). Library models (`Sim.Library` + `Sim.Name`, `${KICAD*_SYMBOL_DIR}` and
    `${KIPRJMOD}` expanded, `.include`s inlined) are embedded: a `.model` card, or the
    `.subckt` with the cards and subcircuits its lines name (`LibItems`, a token
    match). Kinds without an equivalent become SPICE lines as KiCad writes them (AM,
    SFFM, noise, random sources, behavioral R/C/L, E/F/G/H, KiCad's raw SPICE device);
    a pin on several model ports (`3=3 3=4`) or a 4-pin MOSFET/BJT gets a wrapper
    subcircuit beside the original (ngspice refuses some nested ones). IBIS, XSPICE,
    switches, transmission lines and potentiometers are reported, not simulated.
    Values in fields are SI (`M` mega; `1UF`, `1g`, `100uF;16V` read as KiCad does
    them or better), directives SPICE (`spice_number`: `M` milli). Text items with a
    known dot command (`is_directive`) give the analyses (`.op`, `.tran`, `.ac`
    oct/lin as points per decade, `.dc` of a source), `.temp`, and the setup's extra
    lines (`.include`/`.lib` inlined; `.control` blocks, `.probe`, `.save` dropped).
  - Tests (`kicad/tests/all/project_import.rs`): a hierarchical, routed fixture; with
    `KICAD_CLI` and `KICAD_TEMPLATE_DIR` set, every KiCad template is compared against
    `kicad-cli`: schematic nets (`sch export netlist`) and pad positions (`pcb export
    ipcd356`, after removing its constant origin offset). `sim_import.rs`: a fixture
    with each field form; with `KICAD_CLI`, KiCad's simulation demos (pinned commit,
    fetched sparse into `target/kicad-sim-demos` or `KICAD_SIM_DEMOS`) netlisted by
    `kicad-cli` and by us must give every device pin the same operating point
    (IBIS, the XSPICE counter and Q17, which `kicad-cli` can't netlist, are skipped).

- Project export (`kicad::export`, KiCad 9 formats: sch 20250114, pcb 20241229, sym
  20241209): `render_project` builds the files in memory, `export_project` writes them
  into a folder that is new, empty or holds `NAME.kicad_pro` (old `.kicad_mod`s of
  `NAME.pretty` are removed first). `sx.rs` is the writer (exact mm from nm). Entry
  points: File → Export KiCad project… (a save dialog names the folder), a library's
  "Export as KiCad library…", `pcb-designer export-kicad`, MCP `export_kicad_project`.
  - Ids are written as KiCad UUIDs; objects KiCad has and we don't (pins of placed
    symbols, pads, field texts, sub-sheet files) get `kid` ids derived from ours, so
    exports are byte-for-byte repeatable. A sub-sheet file's uuid must differ from its
    sheet symbol's (KiCad's hierarchy breaks otherwise).
  - Symbols: one per (part, variant), `Part_Variant` when a part has several; pins are
    numbered by the variant's pads (`pin_numbers`): a pin on several pads becomes stacked
    pins (the extra ones hidden passive), a pin without pad keeps its number unless a pad
    has it. Lib symbol texts turn in tenths of a degree, everything else in degrees.
  - Schematic: `(x, H − y)` with `H` = `common::page_lift` (the page height rounded down
    to the 1.27 mm grid, shared with the import), since KiCad's grid starts at the page's
    top left: content on our grid stays on KiCad's. Coordinates round to 100 nm.
    Mirrored symbols are `(mirror y)` at −rotation. Field positions are absolute, their
    angles relative to the symbol (0° on a symbol at 90° draws vertical; the import does
    the same now), and KiCad turns a field's justification with its symbol: fields are
    written centred on the text as we draw it (`text_center`, 0.82 cap heights per
    character). Power ports become power symbols `LIB:PWR_NAME` (a hidden power input
    pin named like the net, our glyph), power flags `LIB:PWR_FLAG`, both in the project
    library too. A `/` in a label is `{slash}`.
  - Board: KiCad's y is down and its page hangs from the origin, so the board is moved
    onto it (`Origin`, its top left 25 mm in, whole mm) and `aux_axis_origin` /
    `grid_origin` mark our origin; the import subtracts `aux_axis_origin`, so round trips
    are exact. Back footprints: orientation = rotation − 180, local y not negated, F/B
    swapped (`lib::Place`, the import's rules inverted). Nets: only the schematic's
    (pads without a pin get none), unlabelled ones named as KiCad 9 names them
    (`kicad_net_names`, from `SCH_PIN::GetDefaultNetName` and the driver sort: names
    without `-Pad` first, then the smallest), so `drc --schematic-parity` is clean.
    Zones unfilled, `min_thickness 0`; polygon pads are custom pads with a circle anchor
    inside the outline. DNP is the `dnp` attribute only (KiCad's own BOM rule).
  - Tests (`kicad/tests/all/project_export.rs`): both examples and the hier fixture
    import back the same (nets, pads, symbols, wires, labels, footprints, tracks, vias,
    zones by id). With `KICAD_CLI`: KiCad's netlist has our nets (by KiCad pin number,
    off-board symbols left out, as KiCad does), its IPC-D-356 our pads, schematic parity
    is clean and ERC finds nothing but `power_pin_not_driven`. With `KICAD_TEMPLATE_DIR`
    too (e.g. KiCad's `demos`), every project there is imported, exported and checked,
    ERC against the original's findings.

- **Built-in library** (`model::builtin`): a standard library of generic parts, the file
  `crates/model/library/builtin.pcblib` (4.7 MB of JSON), embedded and parsed once
  (`OnceLock`, callers get a clone). Each part is one symbol with its packages as
  variants, the common one first (placing takes the first): `Resistor`, `Capacitor`
  (chip, disc), `Capacitor_Polarized` (tantalum, SMD electrolytic, radial), `Inductor`
  (chip, shielded power), `FerriteBead`, `Fuse`, `Diode`, `Diode_Schottky`,
  `Diode_Zener`, `TVS`, `TVS_Bidirectional` (SOD, MiniMELF, SMA/B/C, DO-35/41/201AD),
  `LED` (chip, 3/5 mm), `NPN`/`PNP` (SOT-23, SOT-223, TO-92), `NMOS`/`PMOS` (SOT-23,
  SOT-223, DPAK, TO-220), `Conn_01x01`…`Conn_01x40` (pin header straight / right angle /
  SMD, socket; JST PH/XH 2–8, SH 2–6, screw terminals 2–4) and `Conn_02x01`…`Conn_02x20`
  (headers, socket; odd pins left), `Crystal`, `Crystal_GND24`, `TestPoint`, `SW_Push`,
  board parts `MountingHole_M2`…`M4` (plated / plain) and `Fiducial`; hand-made `OpAmp`,
  `LM358`, `74HC00`, `AMS1117-3.3`, `AP2112K-3.3`, `LM7805`.
  - Categories: `PartSpec::category`, also set on the part's symbol. Footprints are "SMD"
    or, with any plated through-hole pad, "Through hole" (`Fp::pad_category`), unless
    `Fp::category` overrides it (mounting holes and the fiducial are "Mechanical").
    Hand-made parts carry theirs in the file. The `categories` test checks them all.
  - Most of it is **generated** (`builtin::generate`: `fp::Fp` builds footprints with
    silkscreen clipped clear of the pads and a courtyard around everything, `sym::Sym`
    symbols; IPC-7351B nominal lands for standard packages, vendor-specific ones say
    "generic land pattern, check the datasheet"). Generated ids are FNV hashes of names
    (`gid("part/Resistor")`, `fp/NAME/pad/I`, `sym/NAME/pin/N`) in their own range
    (`is_generated`), so renaming gives new ids. Pins have numbers; `Map::ByNumber`
    connects pads to the pin of their name, `Map::Pairs` for package-specific pinouts
    (a pad name may repeat: SOT-223's tab is pad "2" too).
  - `cargo run -p model --example gen_builtin` rewrites the generated items and keeps the
    others; `bundled_library_is_up_to_date` fails until it's run. Tests also check pad
    spacing, courtyards, silkscreen off copper and symbol pins on the 2.54 mm grid.
  - **Hand-made** parts: `gen_builtin edit FILE` writes a copy with `EDIT_LIBRARY_ID`
    (the bundled id is refused next to the Built-in library), edit it in the app or
    through MCP (`edit_library`), save, `gen_builtin take FILE` merges it back (edits to
    generated items are reported and dropped). They use generated package footprints
    (`SOIC-8_3.9x4.9mm_P1.27mm`, `DIP-8_W7.62mm`, `SOT-23-5`, `SOT-223-3_TabPin2`,
    `TO-220-3_Vertical`) by name.
  - `cargo run -p mcp --example render_builtin -- OUT PREFIX…` renders contact sheets
    (board and schematic) of the parts whose names start with a prefix, one component
    per variant.

## MCP (`mcp`)

- rmcp Streamable HTTP on its own tokio thread, loopback only, browser Origins refused.
  Calls go over a channel to the app, run between frames (not mid-drag), and wake it
  with `request_repaint`; 60 s timeout (`simulate` 130 s). Headless: `--mcp-stdio` with a
  `HeadlessHost`.
- The `Host` trait is implemented by the app and the headless host. `document()` is
  read-only and `document_mut()` mutable, so requests can read the document while
  holding `libraries()` references (no library clones). `render_scene` takes an
  optional camera (the simulation plot is drawn in pixels). `Request` variants
  are handled in `describe` and `respond` (`host.rs`). Library requests may answer later
  (`agent::Job`) because they can add tabs.
- `execute` takes a batch of commands (one undo step, all-or-nothing) and advertises the
  full command JSON schema. Commands arrive as JSON and are parsed one by one just
  before they run (`run_commands`), so `sheet` / `parent` given by name or path can
  name a sheet an earlier command of the batch added. `screenshot`, `get_selection`
  and `set_selection` take a `sheet` (`ViewSpec::resolve`). `edit_library` takes the file format with merges by id or
  name, refuses edits that add new `Library::issues`, and reports pad coverage.
- Tools are listed in [docs/agents.md](docs/agents.md).

## App (`app`)

- **`PcbApp` state:** the project document is `doc()` / `doc_mut()` (it lives in the
  Project Library's `LibraryEditor`); sites that borrow other fields at the same time
  use the field path `self.project.lib.doc`. Loaded libraries are `Libraries`
  (`by_key`, `add`, `remove`, `sync` with the project's list, `resync` after a save).
  The tab tree is drawn by `TreeBehavior`, which borrows fields one by one; what it
  can't do itself goes into `deferred` (`Deferred`: close a tab, enter a sheet, open
  another item, Gerber actions), handled right after the tree is drawn.
- **Library item tabs** (`library.rs`): a `LibraryEditor` is a loaded library (document,
  issues, message) with its `ItemTab`s, one per open part, symbol or footprint (camera,
  selection, tool, array mode, part form); at most one tab per item. Libraries have no
  tab of their own. `ItemEditor` borrows a tab with its library (`LibraryEditor::editor`,
  `PcbApp::editor`) and holds the editing code. Tab ids (`Tab::Item`, `Focus::Item`)
  are unique in the run. Undo is the library's, shared by its tabs; an undo or redo
  brings the tab of an item it changed to the front (`front_tab_of`). A tab never shows
  another item, except the preview: "Edit footprint" and "New part" in a tab ask for
  another tab (`ItemTab::navigate`).
  - `open_item` is the one way in: the item's tab if it has one, else the preview tab
    (its tile gets a new `ItemTab`, even across libraries; it may hold a simulation's
    output, see "Output tabs" under Simulation) or a new tab right after the
    tab used last. A single click or arrow key in the Project panel previews
    (`project_panel::preview`), Enter and double-click open; everything else opens
    normal tabs; agents open none. A preview that covers the focused tab takes focus
    (else focus falls to another view and the Project panel's selection follows it
    away).
  - A preview becomes normal on a double-click of its tab or an edit made in it: the
    library's revision is compared around the tab's canvas (`pane_ui`), its panels and
    its actions (`keep_if_edited`), not undo/redo, so undo, MCP and Project panel edits
    don't count.
  - The limit (`Settings::max_library_tabs`, 1–50, default 10) counts item tabs of all
    libraries. `item_tabs::to_close` picks: hidden before visible, least recently used
    (`ItemTab::used`, from `tab_clock`; the focused tab is touched each frame) first,
    the preview last; never the focused or just opened one. Closing loses nothing.
  - `maintain_item_tabs` (each frame, `drop_orphan_tabs`) closes tabs of deleted items
    and drops `ItemTab`s without a tile; state and tiles must stay one-to-one.
- **Layout:** sheets and the PCB are tabs (View → Side by side splits them); a sheet's
  tab is titled with its name, the root's "Schematic". Panels and
  tab groups are rounded with 6 pt gaps of window background. GPU canvases can't be
  clipped, so their corners are masked afterwards (`theme::round_corners`).
- **Tabs:** every tab closes. A closed view keeps its pane (camera, selection);
  `show_view` / `insert_tab` reopen it in its remembered group (`view_groups`). Gotcha:
  `TileId`s are sequential, so `set_layout` clears `view_groups`.
- **Panes** (`Panes`): one per view, made on first use, sheet panes share their grid.
  `sheet_view()` is the sheet worked on (the active one, else the last shown); new
  sheets, placed parts and `focus_schematic` go there. After a revision change,
  `close_missing_sheets` closes tabs and drops panes of deleted sheets. Schematic tools
  follow the pointer: hovering a sheet's canvas while placing switches
  `SchPlacer::sheet` there (`switch_sheet` clears the old preview). Double-clicking a
  sheet symbol opens its tab. Deleting a non-empty sheet asks first (`confirm_sheet_delete`).
- **Move to sheet** (`sheet_picker.rs`): the action (no default key) opens a dialog of
  sheet paths with a filter, without the current sheet and selected sheets' subtrees.
  The move and the carry on the target sheet share one undo group, so Esc undoes the
  move. Sub-sheets dragged in the Project tree onto a sheet re-parent
  (`project_panel::can_drop_sheet`: not onto itself, its subtree or its parent; not
  while searching).
- **Canvas context menu** (`menu::canvas_menu`): a right click (no drag; right drag pans)
  selects the object under the pointer unless it's selected, and opens clipboard,
  rotate/mirror (flip on the board), delete and Move to sheet. Picks run as
  `Deferred::Action` after the tiles are drawn.
- **Floating tool bars** (`toolbar.rs`, icons in `icons.rs`) are drawn after the
  canvas and sense click-and-drag, so presses never reach the canvas. View buttons
  (layers, grid/units/snap, fit) sit at the bottom left. Buttons show 24 pt icons 4 pt inside
  their fill, like the side bars' buttons: a dock button is 32 pt (`DOCK_BUTTON`, bar
  40 pt), a tool bar button 34 pt (`toolbar::BUTTON`, its fill 1 pt inside so
  neighbours' fills don't touch); the header and view buttons follow `BUTTON`.
- **Icons** (`icons.rs`) are SVG files: generic ones copied from Tabler
  (`crates/app/icons/tabler/`, MIT, with its licence), electronics ones our own in
  Tabler's style (`crates/app/icons/`: 24×24 viewBox, stroke 2, round caps and joins).
  Everything is drawn in `currentColor`; faint parts use `fill-opacity` /
  `stroke-opacity`, a cut-out ring `#000`. `paint` replaces `currentColor` with white,
  rasterises with resvg (no text, so `default-features = false`) once per icon and pixel
  size (cached as textures in the context's temp data), snaps the square to pixels and
  tints it. So colours stay theme colours: there are no per-theme icon variants. A new
  icon is a file plus a line in the `icons!` table; the test renders every icon and
  checks it is non-empty and white only.
- **Tool windows** (`dock.rs`): icon bars on both edges, never opened automatically.
  Each bar has a top and a bottom `Group`; the left bar also holds the bottom area's group
  at its foot. One open panel per group; a side's two groups stack with a split
  (`left_split` / `right_split`). `Dock` keeps the groups' panel order and the open
  panel per group; `normalize` (on load) puts every panel in exactly one group (new
  panels go to `Panel::default_group`). Buttons sense click-and-drag: `dock_drag` holds
  the dragged panel, the bars record `DropZone`s each frame and `dock_drag_ui` (after
  both bars) draws the indicator and on release moves it (`drop_target`: the nearest
  group of the bar under the pointer). View → Reset panel layout keeps the sizes. Saved
  in settings when no pointer button is down; scripted runs never save settings.
  Panels: Project (⌘1), Messages (⌘3, unseen-problems badge), Properties, Tool settings,
  Board setup, Overview, Schematic setup, Nets (⌘9), Parts (⌘0), Agent activity (unbound). Panel content is drawn through `dock::fixed_size`, which clips
  overflow: egui grows a resizable panel to fit wider content and keeps that size, and
  grids overflow for a frame when they reuse the column widths of the previous
  selection.
- **Messages panel** (`messages.rs`): one tab per `Category` (Schematic: ERC; Board:
  live shorts, flagged copper and the last DRC; Libraries: the loaded libraries' issues),
  the tab headers in the title bar (`panel_frame_ui`), or on a row of their own when the
  panel is narrower than `TABS_WIDTH` plus the title. `status` counts each category's
  findings that aren't excluded by severity (shorts in the short rule's severity,
  flagged copper one warning, library issues warnings); the badge is a check, else the
  count in the colour of the worst. Opening the panel on a fine tab switches to one with
  problems (`tab_to_open`). The tab is session only.
- **Parts panel** (`parts_ui.rs`): `rows` lists, per library (Project Library first,
  Built-in last; libraries without such parts left out), the parts the last active
  editor places: `!is_board_part` for the schematic, board parts for the board. Search:
  every word in name, category, description, default value or properties. Under each library: a
  `Row::Category` per category (natural order, indented), its parts nested, then the
  uncategorised parts. Rows go through `nav_list::show_with_menu`; the selection is a
  (library key, part id), keys skip the library and category rows, a click on one folds
  it (`toggled`: a set of `Fold`s changed from the default, libraries open and
  categories closed; not while searching, when everything with matches is open). Labels
  in rows truncate: an overflowing row would widen the panel's ui, so the preview takes
  the width from before the list. Actions are the Project
  panel's `Act`s (`Place` imports into the Project Library like the tree). Placing
  surrenders the field's and list's focus so the canvas gets R / X / Esc. P is
  `SchFindPart` / `PcbFindPart` (two actions, as an action has one context); power ports
  moved to ⌘P. The preview is `preview::Preview`, invalidated when the library changes.
  A link between the search field and the list (`kicad_link`, always visible) counts
  the KiCad libraries' matches of the query
  (symbols, or footprints on the board; `KicadImport::match_count`, cached per query,
  at most `MATCH_LIMIT`) and opens the import window with the query. The catalog is
  scanned on a background thread the first time (reading every `.kicad_sym` takes
  seconds); `catalog()` joins that scan instead of starting another. Library rows get
  their own background through `nav_list::show_with_menu`'s `fill`.
- **Nets panel** (`nets_ui.rs`): rows from the schematic netlist (pins, sheets) and the
  board's ratsnest by net index (routed, n unrouted, no pads), cached per document
  revision and drawn with `show_rows`. A click is `PcbApp::select_net`: the net's
  wires, labels and junctions on the sheet worked on (else the first sheet with any;
  the parts when it has no wires); with the board active its selection is cleared so
  the cross-probe shows the net's copper. A double click also fits the active view.
  Rows show the net class; the context menu assigns one (`NetsAct::SetClass`).
- **Board setup** (`pcb_ui::board_setup`): the Default class's sizes and the board
  rules, then a card per net class (name and nets commit on Enter / focus loss through
  `committed_text`; sizes live while dragging, one undo group), then DRC severities,
  then "Reset to defaults" (rules and severities in one undo group; not the layer
  count, net classes or DRC exclusions).
- **Agent activity** (`activity_ui.rs`, `Agent::activity`, at most 500): each call's
  request name (snake case of the `Request` variant, not always the tool's name), its
  `describe`, the `Debug` arguments and `mcp::Outcome` (`handle` returns it: the result
  text, truncated, the error, or `Deferred` for calls the app answers later), plus the
  undo step when the revision changed. Session only. Rows are a table under column
  titles (When, Tool, Result, What; `Columns`: fixed columns as wide as their widest
  entry, the titles indented by the rows' expand arrows, `spacing.indent`); What is
  the summary, an error's first line and the undo step, truncated. The panel's header
  (`agent_switch.rs`) holds, after a gap behind the title: "Access", a switch that runs
  `Cmd::ToggleAgent` (a red dot and its tooltip tell why a start failed), and a
  read-only text field in the UI font, as wide as the URL in both states (shrinking in
  a narrow panel), with the URL the server listens on or "Access is disabled", then a
  copy button for the URL (disabled while off) and, while on, the number of calls
  (`activity_ui::count`). Clear sits at the right of the column titles. The body shows a failed start's error.
  While the server runs, the panel's bar button has a green dot (`Badge::Live`, tooltip with the
  URL; the status bar shows nothing).
- **Properties** (`properties.rs`): a symbol and a footprint both have the component's
  Footprint variant combo (`variant_combo`); on the board it leaves out "no footprint"
  variants, which would delete the selected footprint. Text fields keep their typed
  text in buffers keyed by (object, field): the object must be a selected one (others'
  buffers are dropped each frame), and fields being edited are tracked explicitly (focus
  can move to a field drawn earlier in the frame before the edited one sees it go).
- **Project tree** (`project_panel.rs`): project (the root sheet as "Schematic" with its
  sub-sheets nested by name, PCB, Simulation, Project Library, Outputs › Gerber), Built-in Library (read-only),
  External Libraries; new sheets come from the + menu or a sheet's menu (name dialog,
  then the symbol follows the cursor); menus and activation are pure functions returning `Act`s.
  Inside a library's Parts, Symbols and Footprints groups, `categorized` puts a folder
  per category first (`NodeId::Category(owner, group, text_hash(name))`, natural
  order), then the uncategorised items. Items carry their category as `Node::keywords`,
  so the tree search finds a category's items (it shows them, not every folder child).
  "Rename category…" on a folder is `LibraryDoc::rename_category`: one `Update` of all
  the group's items in it. The Category fields (part form; symbol and footprint
  summaries) are `Fields::category`, with suggestions from `fields::categories`: the
  library's and the Built-in library's (cached). Gotcha: egui
  drops focus on Esc before widgets run unless the focused widget's event filter claims
  Esc.
- **Main menu** (`menu.rs`): described once as data, drawn by egui (`bar_ui`) or, on
  macOS, natively with muda (`native_menu.rs`). Gotcha: macOS menus take their key
  equivalents before the window sees the key, so only ⌘ shortcuts become key
  equivalents, and picking one replays its shortcut as an egui key press (keymap context
  and typing rules still apply). The native menu syncs every frame, in place when only
  texts or states change. winit's default menu is off.
- **macOS window** (`title_bar.rs`): transparent title bar with full-size content; our
  strip shows the icon, the project name and "— Edited". Height and button extent come
  from the `NSWindow` (32 pt on macOS 26). Drag → `StartDrag`; double-click follows
  `AppleActionOnDoubleClick`; no strip in full screen; `setDocumentEdited` for the dot.
  The window appearance follows the app theme. Gotcha: `theme::apply` pins egui's theme,
  or a window appearance change swaps our visuals for egui's defaults. Gotcha: winit
  delivers a click on an unfocused window as press, release, release; in one frame egui
  saw a double-click and zoomed. `pointer_filter.rs` drops releases without a press.
- **App icon:** `cargo run -p render --example app_icon -- crates/app/assets/icon.png`
  (committed; deterministic). Drawn on black and white to recover alpha; the squircle is
  two offset triangle fans so their hairline gaps cover each other. Used as the window
  icon everywhere; `scripts/bundle-macos.sh` builds the `.app` with `AppIcon.icns`.
  Unbundled (`cargo run`) macOS shows the app as `pcb-designer`; `setProcessName` and an
  embedded `__info_plist` don't change that.
- **Clipboard:** egui-winit turns ⌘/Ctrl+X/C/V into `Event::Cut/Copy/Paste(text)`, not
  key events; `Keymap::triggered` maps them back to their shortcuts, and
  `Shortcut::event` does the reverse for native menu picks and scripts (so text fields
  still get them). Writing goes through `ctx.copy_text` (`copy_out`, end of frame);
  reading uses the paste event's text, else `arboard` (`clipboard.rs`). A paste goes to
  the view under the pointer; `carry_clip` re-pastes it on another sheet when the
  pointer moves there. Scripts touch the real system clipboard.
- **Keymap** (`keymap.rs`): actions are global or per context (library, schematic,
  board); contexts may share keys. `ToolMeasure` (M) and `ToolSelect` are global. The
  Gerber tab has no context of its own: `Focus::Gerber` (set by clicking or opening it)
  routes M, Select, Esc and fit to it (`gerber_action`) and blocks the other view keys.
  `PcbApp::focus` (`Project`, `Library(key)`, `Gerber`) picks who takes keys; `active`
  is the project pane and stays meaningful under any focus (paste target, sheet
  worked on). `follow_visible_tabs` (every frame) drops focus from hidden tabs. `works_while_typing` for window-level actions.
- **Settings** (`settings.rs`, `PCB_DESIGNER_CONFIG` overrides the folder): theme, unit,
  recent projects, changed key bindings, dock, window, MCP port and start-on-launch,
  KiCad download folder, remembered tool settings, the Settings window's page and size,
  the most library tabs.
  One broken field makes the whole file fall back to defaults, which is why the tool
  settings are kept as untyped JSON values (`tools`, by family).
- **Settings window** (`settings_ui.rs`): a category list with a search field, the page
  on the right; changes apply at once and come back to the app as `Changes`. Search:
  each row is in `ENTRIES` (page, label, keywords; `Rows::row` looks its keywords up by
  label, so a new row needs an entry), keymap rows match by action name and shortcut
  text. `--show settings:PAGE` opens it (screenshots). `MIN_SIZE` is wide enough for
  the widest page (Appearance's UI scale buttons): egui grows a window to its content, so
  a narrower minimum made switching pages widen the window. A wider page needs it raised;
  text of unbounded length (paths, messages) wraps or sits in fixed-width fields.
- **Remembered tool settings** (`view::tool_memory`): `SchSettings`, `PcbSettings`,
  `tools::Settings` and `ArrayForm` serialize their style fields; transient ones
  (rotation, the next name, the routing layer, the zone net) are `#[serde(skip)]` and come
  back from the struct's `Default` (container-level `#[serde(default)]`).
  `sync_tool_memory` (every frame) stores what differs from the defaults, saves a second
  after the last change and hands library editor changes to the other editors
  (`ItemTab::recall`). A new settings field is remembered unless skipped.
- **UI scale** is egui's zoom factor (`Settings::ui_scale`, 80–200 %); egui's own ⌘+/⌘−
  zoom keys are off (`zoom_with_keyboard`), those keys zoom the canvas.
- **Layer colours** (`view::style::LayerColor`, `set_layer_colors`): changed board layer
  colours per theme, global, read through `board_layer_color` (so board, footprint
  editor, Gerber viewer and MCP renders in the app follow); a change calls
  `PcbApp::restyle`. Stored as hex strings in the settings. Schematic colours are fixed.
  The page edits one theme's (`SettingsWindow::colors_of`, at first the PCB editor's),
  reading them under `with_palette` (the chrome keeps the app's).
  Edited with `color_edit` (app): a swatch with our own picker popup (egui's has a copy
  button whose glyph our fonts lack) and the hex code, editable (pasted codes apply while
  typing, `#` optional).
- **Window** size, position, maximized and fullscreen are tracked every frame
  (`track_window`) and saved in `on_exit` (eframe persistence stays off). Scripted and
  screenshot runs keep 1600×1000 and never write settings or workspaces. A position on a
  screen that is gone is left to the OS (macOS keeps titled windows on screen).
- **Workspace** (`workspace.rs`, `PROJECT/.pcb-designer/workspace.json`, the folder has
  a `.gitignore` of `*`): the tab tree as `Node` (tab groups with the front tab, splits
  with shares), tabs by `TabRef` (sheet id, board, item tabs by library file, item and
  preview flag, Gerber), each pane's camera and hidden layers (by layer name), the item
  tabs' cameras least recently used first (`items`, their recency), the routing layer.
  Saved on save, before another project replaces it and in `on_exit`; restored in
  `replace_document` and at startup (not with `--side-by-side`). Tabs that no longer
  resolve (deleted sheets, libraries not loaded) are dropped; a layout with nothing left
  keeps the default tree.
- **UI fonts** (`theme::install_fonts`): egui's defaults plus JetBrains Mono (the canvas
  font's bytes, `render::font::REGULAR`) as the last fallback of both families, for
  arrows and ⌘ ⇧ ⌥ (so `format_shortcut` shows symbols, not "Cmd+"). ↺ and other
  symbols are still missing: `theme::tests::ui_strings_have_glyphs` scans the string
  literals of `app` and `view` and fails on a character without a glyph.
- **Panel forms** (`fields.rs`): panels and windows draw in a `form_scope` (the dock
  wraps `panel_ui`, `theme` wraps window contents): every `fields::grid` in it shares
  one label column, last frame's widest `fields::label` (at most 45 % of the width,
  longer labels wrap; a change repaints). In a grid (`IN_GRID`, a thread local) value
  widgets fill the value column: `fill`, `combo_box`, `units::drag`, `Fields::number`,
  `Fields::text_hint`, `SiFields::field` (their `width` is for outside grids);
  `pair` splits it in halves, `sized` gives a field less room (a button after it). Use
  these, not `egui::ComboBox` / `ui.add(DragValue)`, in grids. Text: `subject` (a section
  title with a dim subtitle: the selection or tool; Tool settings reserve it with `reserve_subject` and draw it
  last, since the tool sets its help), `section` (bold 14 pt, no space at the panel
  top), `collapsing` / `collapsing_note` (long lists, rarely edited sections such
  as a component's Simulation; closed at first, with a note saying what's inside), `hint` (one line), `info_button` (help longer than
  a line: hover shows it, a click pins it). Every `CollapsingState` header goes through
  `collapsing_header`, which toggles on a click anywhere on the header (a click area
  registered after it, so on top of its widgets and the arrow; egui's own toggles on the
  arrow only); `egui::CollapsingHeader` already does. Bodies are unindented
  (`body_unindented`, `show_unindented`): no line on their left, so an open section looks
  like a plain one. The arrow starts where the labels around do: `collapsing_header`
  narrows egui's `indent` (the arrow's button) to the arrow while drawing the header,
  `tight_arrow` does it around an `egui::CollapsingHeader` (there it's the arrow plus the
  gap before the title, which starts at `indent`). `check_row` (boolean: label left, bare
  box), `search_field` + `summary` (list panels). Every search or filter field is
  `clearable`: a round × inside its right end while it has text (the field's right
  margin keeps text clear of it; added after the field so it gets the click; a click
  clears, keeps the focus and marks the field changed). The Settings window keeps its own
  wide layout (`Rows`), the part form its own field widths. Panel titles, tab titles and
  section titles share `theme::TITLE_SIZE`; panel title rows and the tab bar have the
  `panel_header` shade and `TITLE_ROW_HEIGHT` (the dock paints the shade behind the row
  once it's laid out), the
  active tab the panel's colour.
- **Dialogs:** use `theme::dialog` (modal, accent title; bold is faked by
  `theme::bold_label`) and, for non-modal resizable ones, `theme::window` (same frame
  and title bar, a × button, opens centred; Settings, KiCad import). Bus prompts are `name_prompt.rs`.
- **Recovery copies** (`recovery.rs`): saving stays explicit. Each running app has a
  slot in the settings folder's `recovery/` (unsaved projects too, nothing in project
  folders), with a recovery copy written 5 minutes (`INTERVAL`) after the first change
  not in it, on a thread (a clone of the project), into `project.next/` and then swapped
  in, the meta file last. The slot empties when the document is unmodified again (saved,
  undone) or replaced, and goes on a clean quit (`on_exit`). The app holds an exclusive
  lock on the slot's `lock` file (`File::try_lock`) while it runs; the OS lets go when it
  crashes, so unlocked slots are what `left_over` offers at start and when opening their
  project (Restore / Discard / Later). Restore opens the copy with `Document::mark_unsaved`
  and its folder, if it still exists; the copy then moves to this app's slot. Scripted
  runs keep copies only with `PCB_DESIGNER_CONFIG`.
- **Native file dialogs** (`file_dialog.rs`) never block the frame. A blocking
  `rfd::FileDialog` runs a nested event loop inside winit's event handler, and on macOS
  the save panel can deliver an event winit queued meanwhile: winit aborts ("tried to
  handle event while another event is currently being handled"). So `Dialog<T>` opens
  `rfd::AsyncFileDialog` (a sheet on macOS) and its owner polls it each frame; `T` says
  what to do with the path (`Chosen` in the app). A save that needs a dialog (an
  unsaved project, Save As) takes a `Then`: what the save interrupted (an export,
  opening another project, quitting) runs again once it saved (gotcha: save first,
  then run the `Then` if it worked; `then.filter(|_| save())` never saved when nothing
  waited). A script's `dialog PATH` / `dialog cancel` step answers the next dialog
  (`Dialog::answered`) instead of showing it, so saves can be tested headlessly. Alerts
  (`rfd::MessageDialog`) stay blocking: they run in the modal panel run loop mode,
  which winit's queued events don't run in.
- **GPU canvases** (`canvas.rs`): each `Canvas` takes a unique id and its own `GpuScene`
  in `CanvasResources`; dropping the canvas queues the id and the next `prepare` frees
  the scene. Gotcha: a new canvas only receives the scene's incremental uploads, so it
  must start with a scene that wasn't uploaded elsewhere (a fresh `ItemView`, or keep
  the canvas with its scene).
  - **Crosshair** (View → Crosshair, Shift+X, `Settings::crosshair`): `Canvas::ui`
    draws it with egui at the raw pointer (not snapped) while the canvas is hovered,
    under the overlays drawn after it. The switch is a global (`canvas::set_crosshair`,
    set each frame) so every `Canvas::ui` (editors, Gerber viewer) follows it without
    threading the setting through; previews only `paint` and never show them. Colour:
    `Palette::crosshair`, the 45° lines at half strength, one physical pixel wide;
    dashed rays from a hole around the pointer (`CROSSHAIR_*` constants).
- **Previews** (`preview.rs`): `Preview` is a view-only `ItemView` + `Canvas` in a box
  (fitted every frame to `ItemView::content_bounds`, no grid, axis or origin, square
  corners with a border, a hint or error text when empty), in its item's editor theme
  (`Canvas::editor` also gives the crosshair its colour). It restyles itself on a
  theme change. Previews switch items instead of being recreated, so their count stays
  fixed (two per part form, two in the import window). The part form shows the symbol
  of a chosen unit and the footprint of a chosen variant (`view::item::PartPreview`,
  falling back to the first); a click opens the item.
- **Dev aids:** `--script` steps (see AGENTS.md), `--screenshot` (captures egui output:
  the native window buttons don't show), `--panel`, `--show`, `--select`.

## Performance (Apple M4, release)

Budget: about 10k footprints at 60 fps, an edit → board → GPU update within one frame.

| Measurement (10k components unless noted) | Result |
|---|---|
| Schematic connectivity, full rebuild | ~13 ms |
| Board copper connectivity (20k pads) | ~6 ms |
| Zone fill + connectivity + render, whole-board zone | 0.10 s (2k) / 0.54 s (10k) |
| DRC | 8.5 ms (2k) / 13 ms (10k) |
| Redraw users of one library edit (3,333 capacitors) | ~21 ms + 18 ms (old full rebuild ~200 ms) |
| Drag step | ~0.035 ms |
| Drag step, symbol with wires (replayed from the start) | ~0.5 ms |
| Drag step, footprint with tracks (connectivity deferred while dragging) | ~0.2 ms |
| Both canvases animated, ~3 M primitives (stroke font era) | 90–170 fps uncapped |
| GPU frame, 10k board at 3200×2000 offscreen: TrueType texts (~2.9 M instances) / stroke font (0.6 M) | 3.9 ms / 1.1–1.4 ms |

Measure with `edit_bench` (`cargo run -p view --release --example edit_bench`) and
`--stress N`.

## Next milestones

Usability work collected from the Later list, in the suggested order. Remove a milestone
when it's done; what it defers goes back to **Later**.


## Later

Things decided to do later, by area. Remove an entry when it's done; add what a change
defers.

**Schematic**
- KiCad import gaps (counted in the import report today): schematic
  graphics, images, text boxes, tables and rule areas; text variables (`${TITLE}` etc. in
  texts stay as written; the frame shows the title block);
  bus aliases; directive labels (net class flags); stored sheet pin positions (laid out by name); reused
  sheets are duplicated (no reuse here); bus entries of unusual sizes.
- Drawing frames: custom KiCad drawing sheets (`.kicad_wks`; the default frame is used),
  an "update the date on save" option, text variables in schematic texts, printing.
- Schematic PDF: selectable and searchable text (an embedded TrueType font instead of
  outlines), sheet symbols linking to their page, a black-and-white option, the board
  as PDF (assembly drawings).
- After sheets: hierarchical bus pins (bus-named hierarchical labels are drawn as pins
  but don't connect), ERC of pin directions across sheet pins.
- KiCad's "unfold bus".
- ERC/DRC review: importing KiCad's exclusions (their keys are KiCad's own; reported as
  skipped); the flagged-copper line in Messages ignores DRC severities (only shorts
  follow them).
- Pins lying on a moved wire's inside (not at its ends) are left behind.
- Inner-layer footprint graphics.
- Reference and value texts: re-sync fields when the library templates change; fields
  of other texts than `${REF}` / `${VALUE}`; a Text tool placed field of a symbol
  (KiCad's extra fields).
- Texts: undo inside an in-place edit; the library editors' Text tool typing in place
  (it takes the text from its settings today); text DRC (silkscreen over pads, minimum
  stroke and size).
- Fonts: choosing the font in the settings (loading a TTF at runtime); italic (KiCad's
  `(italic yes)` is ignored); shaping, kerning and right-to-left scripts; a fallback
  font for glyphs JetBrains Mono lacks (CJK; `.notdef` boxes today); level of detail
  for texts on huge boards (a box when a few pixels high) if the GPU time matters.
- Schematic colours customisable like the board layer colours.
- Custom power symbols: power ports drawn from a symbol of their own (a part flagged
  power, no footprint), for glyphs beyond `PowerStyle`; maps to KiCad custom power
  symbols on import and export.

**Board**
- Net labels on arc tracks, vias and zones; pad labels follow the footprint editor's
  pads only as static numbers.
- Fabrication outputs: fab/courtyard Gerbers, Protel file extensions, JLCPCB's rotation
  offsets (their library turns some packages differently: a per-part offset in the
  position file), more assembly services, dielectric layers and impedance in the stack-up,
  assembly variants (several fitted/not-fitted sets per board; "do not populate" is one
  flag today), IPC-D-356
  netlist,
  silkscreen clipped at mask openings, mirrored bottom text in the editor; in the viewer:
  the mask area from the outline's contour (bounding box today), true clear polarity,
  G74 arcs, LM, thermal primitives, slots in drill files.
- Clipboard: carrying a paste between the schematic and the board.
- Parts panel: dragging a part from the list onto the canvas.
- Cutouts that split a zone into two pieces (refused today; would need a second zone).
- Routing: diff pairs (routing and shoving them as pairs), length tuning (meanders);
  shoving arc tracks, and the Route tool's arc corner styles while shoving or walking
  round (the router lays straight segments); an index of pushed lines (each check scans
  them all) if cascades get deeper; dangling track ends moving with a push (a line's ends
  are fixed, so a free end inside the head's hull makes it unpushable); MCP `move`
  shoving like a drag (a `shove` flag running `router::shove_moved`); shoving
  footprints' copper (moving parts out of the way) and KiCad's "smart pads" (choosing a
  pad's exit direction).
- Net classes: diff pairs and their gap, class colours on the board and ratsnest,
  per-class minimums checked by DRC (track width below the class's), a separate edge
  clearance, schematic directive labels (net class flags) and KiCad's own class rules
  (`.kicad_dru`).
- Routing checks: clearance to zone fills of other nets, checking the vias L adds against the next layer's copper
  only once the layer changes; blind and buried vias drawn differently from through
  vias; per-span via sizes; micro vias.
- Round track corners: corners at pads and vias, between arcs, and live radius
  editing of an existing arc.
- Track drags: a 45° fix-up where a joining track is added (two segments instead of one
  at any angle); tees of other tracks onto a moved track's inside don't follow;
  re-selecting "expand" (through vias, then the whole net) on a second double click.
- A move that detaches (KiCad's M versus G): drags always keep connections today.
- Copper graphics inside footprints (from KiCad custom pads) counting as connections.
- Incremental or background zone refill (only zones near the change).
- Zones: minimum width, hatched fill, a per-pad zone connection override; zones and
  keepouts inside footprints.
- Arc outlines in zone fills (circles are 16–64-segment polygons today).
- Exact courtyard polygons in DRC (bounding boxes today).
- A board drawing frame / fabrication drawing.
- From KiCad board import (skipped and counted today): dimensions, groups, images, text
  boxes, tables, alignment targets, graphics and texts on documentation and user layers
  (user layers in general), net ties, tuning patterns (their tracks are kept).
- Autoplacement and autorouting.
- Reintroduce optional kicad style single line vector font (instead of ttf)

**Libraries and parts**
- Categories: a category field in the KiCad import window, dragging items onto a
  category folder, setting the category of several selected items at once, category
  suggestions from all open libraries (only the item's own library and the Built-in
  library today).
- Built-in library: trimmer potentiometers (Bourns 3362P, TC33), SMD tactile switches
  (6×6, 3×4) and a slide switch, HC-49 SMD crystal, USB-C (USB 2.0) and Micro-B
  receptacles; 1.27 / 2.00 mm pin rows, RGB LEDs, dual diodes (BAV99), resistor arrays,
  crystal oscillators, barrel jacks, more logic and regulators. Check the generic land
  patterns (power inductors, SMD electrolytics, DPAK, JST, screw terminals) against
  datasheets.
- Footprint wizards.
- Circular pad arrays.
- Pad paste ratio (KiCad's `solder_paste_margin_ratio`, warned about on import) and
  footprint-level mask/paste margin defaults (the import copies them into the pads).
- KiCad import: pins a multi-unit symbol's units share (KiCad's unit 0) land on the first
  unit only, so wires and no-connect flags at them on the other units dangle (seen in
  KiCad's `complex_hierarchy` and `tinytapeout` demos).
- KiCad export: power flags where KiCad's ERC wants them (nets powered only by power
  ports), KiCad's standard power symbols (`power:GND`) instead of our own, filled zones,
  exclusions, custom pad primitives other than one polygon, De Morgan and alternate pins.
- KiCad import: back-side footprint content, text boxes, De Morgan bodies, alternate pin
  functions, exact oval drills, 3D models.

**Simulation**
- Simulation backends: only single runs stream (a sweep's steps, and processes, show
  nothing until done; a process could stream with ngspice's interactive mode); sweeps
  can't use the library (one instance per process: copies of the library file would
  give more); a crash in the library takes the app down (a watchdog process holding
  libngspice would isolate it).
- KiCad simulation import gaps: IBIS (KIBIS) and XSPICE code models, switches,
  transmission lines, coupled inductors (`K`), potentiometers; the workbook (`.wbk`:
  plotted signals, cursors) and `.probe` / `.save` become no probes; `.control` scripts
  are dropped; a subcircuit's instance parameters aren't passed through a pin-joining
  wrapper; nested DC sweeps keep the first.
- Our own simulation engine in Rust beside ngspice, chosen in Settings: modified nodal
  analysis, Newton–Raphson, trapezoidal/Gear integration, a sparse solver; R, C, L,
  sources, diode, BJT, MOS level 1 first. A port of ngspice isn't realistic (~730k lines
  of C; matching its convergence behaviour and the quirks vendor models rely on is the
  hard part), so ngspice would stay for vendor models and KiCad projects.
- Plot: overlays of earlier runs, PNG export from the GUI (MCP's screenshot renders it
  offscreen already), the zoom saved in the workspace; several plot panes per analysis
  (stacked, shared x), saved plot layouts.
- Schematic values: placement that avoids other texts (tags can overlap a part's
  reference or another tag), values at the cursor of a sweep (scrubbing), currents of
  bigger parts as tags; a pinned "always" (they only show while the Simulation panel or
  a simulation tab is open).
- Expressions of signals (`V(OUT)/V(IN)`, `abs(I(R1.1))`); complex power for AC.
- Simulation tree: drag to reorder simulations; an agent's run showing its tab (it only
  marks the tree row today).
- Per-unit models for multi-unit parts (a dual op amp as one subcircuit instance per
  unit): a model covers every unit's pins at once today.
- Syntax highlighting in the model text field; an inline analysis for `simulate`
  (agents add one with `SetSimulation` first).
- Source symbols that show the selected waveform (sine, square, ramp glyph), which needs
  symbols drawn from component data.
- `.include` / `.lib` of external model files (relative to the project), a project model
  library browser, vendor model import (pasting a whole `.lib` with many `.subckt`s and
  picking one).
- Noise, pole-zero, sensitivity, Monte Carlo with part tolerances, temperature sweeps,
  worst case.
- Distortion: THD over a sweep's steps (against the source amplitude: the clipping point;
  against frequency), which needs a measurement per step; a "precise" transient option
  (tighter `reltol`, a max step from the source frequency) instead of `.options` in
  Extra; a known fundamental (the source's) instead of the measured one; spectrum window
  choices (flat top for levels) and averaging; harmonic markers on the spectrum.
- Digital and mixed-signal (XSPICE digital models for 74HC parts, A/D bridges), logic
  traces in the viewer.
- Switch models (voltage/time-controlled) for `SW_Push` and relays, transformers and
  coupled inductors (`K` lines), transmission lines.
- Sweeps: two parameters (nested), a measurement plotted over the stepped value (the
  −3 dB point against R1), steps of a model parameter or the temperature; the Simulation
  panel's result table shows the first step only.
- Tuner: model parameter overrides as rows; agents have no tuner tool (`simulate`'s
  `values` do the same for one run).
- Probes and operating point values on sub-sheet instances once sheets can be reused.
- Board-aware simulation (track resistance, parasitic capacitance from the layout).
- KiCad export of simulation fields (`Sim.*`) and of the setup as directive texts.

**Performance**
- Ratsnest of huge nets (7k-pad GND, ~2.3 ms): incremental MST or a worker thread.
- Viewport culling per chunk and level of detail for tiny primitives.
- Same-layer transparency (render each layer to a texture, then composite).
- Incremental schematic connectivity (full rebuild today).
- Drawing symbols as shared, transformed instances (per-instance redraw is the floor of
  library edits).
- `Store` bulk removals (O(n) each), linear scans for refdes numbering and sync lookups.
- Cache the project tree nodes if big libraries make the per-frame rebuild slow.

**GUI**
- Item tabs: a "Keep preview open" action and a "Close tab" key; a way to discard a
  listed library's unsaved changes short of removing it from the project.
- macOS: a Liquid Glass app icon made with Icon Composer (needs Xcode's `actool`); more
  content in the title strip; "Fill" on title double-click (no public API, zooms today).
- Native menus don't show shortcuts without ⌘ (they would steal typing).
- Remove weird items in native macOS Edit menu: Autofill, Start dictation, Emoji and symbols?
  Is that even possible? It seems to be there in any app...

**Other**
- Recovery copies of modified external libraries (only the project is kept now), and a
  setting for the interval.
- Releases: a universal (x86_64 + arm64) macOS app (ngspice built twice, `lipo`), a
  Windows installer and a Linux AppImage. Trimming ngspice's unused device models
  (464k of its 730k lines) if its size ever matters.
- macOS signing and notarization: the Apple Developer Program (US$99 a year; an
  individual's legal name shows in the signature, an organisation needs a D-U-N-S
  number), a "Developer ID Application" certificate exported as `.p12`. In the release
  job: secrets for the `.p12` (base64), its password and an App Store Connect API key;
  import into a temporary keychain, `codesign --options runtime --timestamp` (hardened
  runtime) instead of the ad-hoc signature, `xcrun notarytool submit --wait`, `xcrun
  stapler staple`. The hardened runtime won't load another developer's libngspice
  (Homebrew's, KiCad's) without the `com.apple.security.cs.disable-library-validation`
  entitlement; the bundled one is signed with the app's identity, so it loads.
- Documentation: publish `docs/` as a site (GitHub Pages with a template such as
  just-the-docs), with screenshots.
- Opening a project from the file manager: register `project.pcbproj` as a document type
  (`CFBundleDocumentTypes` in the macOS bundle and the open-file event, a Windows file
  association, a Linux `.desktop` MIME type). The command line already accepts it.
