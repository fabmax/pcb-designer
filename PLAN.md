# PCB Designer – Plan and design notes

Requirements are in [pcb-designer.md](pcb-designer.md). This file holds the decisions,
the current design of each area with the gotchas a change needs to know, and the list of
things to do **later**.

## Decisions

| Topic | Decision |
|---|---|
| UI | egui / eframe, egui_tiles for tabs and splits. Dark and light themes |
| Canvas | Custom wgpu renderer (SDF instanced primitives) inside egui paint callbacks |
| Schematic → PCB | Always live, inside the same command transaction. PCB → schematic: never |
| Copper | Tracks have no stored net; nets come from the pads they touch. Copper whose net no longer fits is flagged, never deleted |
| Library parts | Copied into the project (Project Library). Each copy keeps `source` (library id, part id, content hash) for "update from library" |
| Schematic scope | KiCad's concepts, so KiCad projects can be imported and exported later: buses, label scopes (local / global / hierarchical / power), net names and hierarchical sheets with sheet pins as KiCad does. Multi-unit parts work |
| Files | JSON, pretty, stable order, integer nm. A project is a folder (`<name>.pcbproj/`: `project.json`, `library.json`, `schematic.json`, `board.json`); a library is one `*.pcblib`. No migrations until production use (`model::io::MIGRATIONS` stays empty) |
| MCP | In the app (Streamable HTTP on 127.0.0.1, opt-in) and headless over stdio. Every tool goes through editor-core commands |
| Platforms | macOS first. macOS-only chrome (native menu, title strip) is `cfg`'d; Windows/Linux keep the egui menu bar and system title bar |

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
- The `ChangeSet` drives incremental updates. Board setup and the library list ride
  along as before/after (`setup`, `libraries`); views ignore them. Gotcha:
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
  `at - anchor`. Sheet symbols and footprints aren't copied. Library item clips (pins,
  pads, graphics, texts) are pasted by `view::item::paste_elems` as an `Update`.
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
  {message, objects, pos}`.
- **Sheet pins** are derived, never stored: one per hierarchical label name of the
  sub-sheet (`Schematic::hier_ports`, first label's shape), laid out by `Sheet::layout`
  (inputs left, the rest right, by name, 2.54 mm from the top; the drawn rectangle grows
  downwards past the stored `size` to fit). On the parent they connect like component
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
  `problems`): per wired net (lone pins are left to "Unconnected pin"), from
  `NetPin::kind`: two hard drivers (output, power output), or one with a tri-state or
  open collector pin, conflict; inputs need a driver (any other kind but power input
  and not connected, or a power port); power inputs need a power output pin or a power
  port (power ports stand in for KiCad's power symbols; there is no PWR_FLAG); wired
  pins of kind not connected. Plus components sharing a reference (`R?` and empty
  ones aside). Messages name pins as `U1-14 (VCC)`, not nets.
- **Arc tracks** are a `Track` with `mid` (a point on the arc, KiCad's start/mid/end);
  `a`/`b` stay the ends, so anchors, ratsnest and routing don't know about arcs.
  `Track::arc()` gives the exact `Shape::Arc` (drawing, Gerber G02/G03 with exact ends),
  `Track::path(err)` chords. Copper uses `Geo::Path` (capsules along chords at most
  `ARC_ERROR` = 1 µm off, widened by that), split into capsules by `Geo::capsules` for
  all tests. Gotcha: once board moves drag connected track ends (not yet), an arc's mid
  must move to keep its bulge (sagitta / chord), not stay put.
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
  none; thermal gap/spoke overridable per zone), flood own tracks/vias and netless vias,
  priority order, drop islands per `IslandRemoval`. A zone with net `""` is isolated
  copper: clearance to everything, all islands kept, no `ZoneNet`. Fills are derived,
  never saved.
  Two-pass connectivity: copper without zones → fill → rebuild with fills.
  `RegionIndex` grids fill edges (without it, touch tests on 80k vertices took 0.7 s).
- **DRC** (`drc::run`, pure, most severe first): shorts, clearance (pads within one
  footprint and fills skipped), keepouts, board edge, courtyard overlaps (bounding
  boxes), track width and drill minimums, zone net, unconnected copper, unrouted.
- **Keepouts** (`Keepout`, KiCad rule areas) are separate from zone holes on purpose: a
  hole is part of one zone's shape (other zones may fill it, it moves with the zone), a
  keepout is a rule for all zones on its layers (`layers` empty = all copper layers).
  `no_pour` is enforced by the fill; tracks, vias, pads and footprints (front courtyard
  bounding box, on the footprint's side) by DRC only, not while routing.
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
- **Reference and value texts** (`FootprintField`, `board.fields`): a placed footprint
  gets its own copy of each library footprint text that is exactly `${REF}` / `${VALUE}`
  (`FootprintText::is_field`), in footprint coordinates, so it moves, turns and flips
  with the footprint but can be dragged and turned on its own. Created and removed with
  the footprint (`insert_fields` / `remove_fields`: sync and board parts); deleting one
  is refused, hide it instead. `draw_footprint_at` skips the templates for placed
  footprints (the place ghost draws them); Gerber exports the fields. Hit testing is the
  text's bounding box, which beats the footprint's courtyard. Gotcha: a library edit of
  the templates doesn't reach footprints already placed.
- **Slow rebuilds wait:** when connectivity (or redrawing library users) took over 4 ms
  (`Document::slow`), it is deferred during a drag and caught up afterwards.

## Gerber export (`gerber`)

- `gerber::generate` writes one Gerber X2 file per layer (copper, mask, paste, silk,
  outline; mm, format 4.6, so coordinates are nm) and Excellon drills (plated: TH pads
  and vias; non-plated), named like KiCad's (`Demo-F_Cu.gbr`, `-PTH.drl`). `export`
  replaces `<project>/output/gerber/`, writes `output/<name>-gerber.zip` (own minimal
  zip writer, fixed dates) and `export.json` (`Manifest`: files, labels and
  `design_hash` of board + schematic + library, for "outdated"). `Document::export_gerber`
  refills, runs DRC (violations are warnings) and needs a saved project and an outline.
  Used by the menu (⇧⌘E), MCP `export_gerber` and `pcb-designer export-gerber DIR`.
- Pads are flashes; rectangles and ovals at quarter turns use R/O with swapped sides,
  everything else the `RoundRect` macro with the corner centres pre-rotated (readers
  such as PyGerber ignore the rotation of circle primitives). Polygon pads and zone
  fills are regions; holes are bridged into the outline with cut-in lines
  (`bridge_holes`), never drawn clear, as clear polarity would also erase other copper
  inside the hole. Mask files are written with negative polarity (drawn = opening).
- Text is stroked from the stroke font and mirrored around its anchor on bottom layers
  (the editor never mirrors text).
- Checked against PyGerber renders (in a throwaway venv); no files are committed.
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
  R-tree over objects, exact `Prim::contains`) names the file, D code, aperture and
  `.AperFunction` under the pointer. The app's Gerber tab (`Tab::Gerber`, not a
  `ViewId`: it has no document) uses its own `Canvas`; MCP `screenshot` target
  `gerber` (optional `layers`) renders the same view offscreen.
- **Outputs in the tree** (`app/src/outputs.rs`): Outputs › Gerber › files, scanned
  when the project folder changes, after an export and when the window regains focus
  (no file watching). "Outdated" compares the manifest's hash with `design_hash`,
  recomputed once per document revision and never while an interaction is running.

## Rendering (`render`)

- Instanced primitives in one 32-byte instance: capsule, circle/ring, rounded rect, arc,
  triangle. Triangles mark outer edges (anti-aliased); inner edges are hard so polygons
  show no seams (but a fan's hairline gaps can show: see the app icon).
- Polygons: ear clipping (`render::tess`) for small ones, i_triangle for zone fills
  (holes, O(n log n); earcut took 1.9 s on a fill with thousands of holes).
- Stroke font → capsules (also used by Gerber); `\n` starts a line (`LINE_SPACING`).
  Small details hide when zoomed out. Per-layer buffers; only dirty items upload;
  pan/zoom is a uniform.
- `Scene::set_item` gives up a shrunk item's tail as garbage (else `instance_count`
  drifts from a rebuild). `Scene::snapshot` doesn't consume the incremental upload.
- `Offscreen` renders to PNG (tests, MCP screenshots, `render_board`, the app icon). Its
  background is opaque.

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
- Library edits redraw only their users (`View::users_of`): symbols depend on symbol,
  part and the component's footprint (pad names); footprints on footprint and part.
  Edits that don't change drawing (name, prefix…) redraw nothing.
- `Document` = editor + folder + a view per sheet + the board view. `changed` adds or
  drops sheet views when sheets come and go (commands, undo); `view(id)` of a missing
  sheet returns an empty view. `LibraryDoc` wraps a document whose
  project holds only a library; the Project Library's `LibraryDoc` wraps the project
  document itself (one undo history).
- `Interaction`: click/shift/cmd select, box (left→right inside, right→left crossing),
  drag with snap of the grabbed reference point (one undo step; R rotates while
  dragging), `snap` toggle. Locked footprints don't move. A press becomes a drag when
  the *hand* moves the pointer 4 points on screen (`PointerInput::screen`), not when the
  view shifts under it (a panel changing width); the drag then follows the hand's motion.
- Dragging keeps schematic wires attached (`keep_connected`; `rubber_band` over wires and
  buses): orthogonal wires shift and stay orthogonal, the far end stretches or gets a
  dogleg; rotate/mirror stretch.
- Tools: `tools::Placer` (library: pin/pad, line, rect, circle, text; names
  auto-increment), `sch_tools` (wire, bus, bus entry, label, power, junction, no connect, place;
  `/` flips the corner; `SchPlacer::sheet` is the sheet they work on, `set_sheet` ends a
  wire begun elsewhere; the Sheet tool places a new sheet's symbol with one
  `AddSheet`), `pcb_tools` (route 45°/90° with copper snapping, L changes
  layer + via, via, outline, zone, measure). Bus naming prompts: `SchPlacer::take_prompt`
  returns a `SchPrompt` for unnamed buses and entry signals.
- `measure`: the measure tool's state (`Measure`: click, live end, click pins, next
  click starts anew, Esc clears) and snapping (`snap`: nearest object point within the
  pixel tolerance, else the grid point; Alt free). Object points: `board_points` (pad and
  via centres, track ends, board graphic vertices), `footprint_points`,
  `GerberView::snap_points` (flash centres, stroke ends, via the R-tree). One
  measurement per editor, kept in `PcbPlacer` / `Placer` / `GerberPane`, cleared by
  `set_tool`; not saved, no commands. ⌘C copies `Reading::text` when the focused
  editor's Measure tool shows one (`App::measure_reading`), before object copy. The app draws it (`canvas::draw_measure`).
  Gotcha: `board_points` scans the whole board per hover.
- `clipboard`: copy (anchor = snapped reference point of the first copyable object) and
  paste into a view, checking the clip's domain. A paste is carried:
  `Interaction::carry` is a drag without a button in the paste's undo group (moves,
  R, X join it; a click drops; Esc undoes the paste). Carried edits are wrapped in
  `Edit::Carry` so schematic wires the paste passes over aren't rubber-banded.
- Labels are drawn KiCad style: local with a dot, global as an outline around the name,
  hierarchical as a small arrow before it; `LabelShape::points` says which ends are
  pointed. Label text goes through `Text::upright` (turned 180° with swapped alignment
  instead of upside down or reading downwards).
- `cross_probe` (pure): selection ↔ symbols/footprints, wires ↔ copper of their nets;
  the routed net wins. Probes from the board go to every sheet view.
- Themes: `style::Palette` (`DARK`, `LIGHT`), a global baked into scene instances;
  `Document::restyle` / `ItemView::restyle` rebuild everything with overlays.
- `project_libs::sync` loads listed library files (failures become `MissingLibrary`),
  keeps unsaved or file-less ones, refuses duplicate library ids (`check_new`);
  `after_save` relinks or adds a saved library; `copy_for` copies with a new id.
- `tree`: toolkit-independent tree model (`Node`, `TreeState`, keyboard, search that
  restores the expansion afterwards).

## Libraries and KiCad (`kicad`)

- Imports `.kicad_sym`, `.kicad_symdir` (KiCad 10) and `.pretty` (KiCad 6–10). Units
  from sub-symbols, `extends` merged, power symbols skipped (they are power labels
  here), symbols with hidden `power_in` pins refused (KiCad joins those implicitly to the
  global net of the pin name; none in KiCad's current libraries), other hidden pins shown
  with a warning, footprints y-flipped; `B.*` items and `B.Cu`-only pads become `back`
  ones, a pad's mask/paste come from its layer list. A symbol requested without footprints gets a "No footprint" variant
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
    `power_in` symbols become power labels (style from `power_style`), `PWR_FLAG` is
    dropped. Every sheet instance becomes a sheet (reused files get ids derived from the
    instance path); sheet pins are laid out by name here, so wires ending on KiCad's pin
    positions are stretched to ours. Refdes from the symbol's `instances`, else the root
    `symbol_instances` (KiCad 6), else the Reference property. Each file's `paper`
    becomes its sheets' page and `to_pages` lifts the content by the page height (KiCad
    pages hang down from the origin). The root file's `title_block` is the project's.
    `common::plain` also undoes KiCad's `{slash}`-style escapes.
  - Board: pad and text angles in `.kicad_pcb` are absolute (minus the footprint's
    orientation). Back footprints are stored mirrored: un-mirrored (negate y and angles,
    swap F./B. layers and chamfer corners), then placed with rotation + 180 on the
    bottom. Reference/Value become `${REF}`/`${VALUE}` templates plus per-instance
    fields. Zone nets are resolved through the pads of the KiCad net (KiCad names like
    `Net-(R3-Pad1)` differ from ours), else kept with a warning. Rules from the Default
    net class and board setup.
  - Tests (`kicad/tests/all/project_import.rs`): a hierarchical, routed fixture; with
    `KICAD_CLI` and `KICAD_TEMPLATE_DIR` set, every KiCad template is compared against
    `kicad-cli`: schematic nets (`sch export netlist`) and pad positions (`pcb export
    ipcd356`, after removing its constant origin offset).

## MCP (`mcp`)

- rmcp Streamable HTTP on its own tokio thread, loopback only, browser Origins refused.
  Calls go over a channel to the app, run between frames (not mid-drag), and wake it
  with `request_repaint`; 60 s timeout. Headless: `--mcp-stdio` with a `HeadlessHost`.
- The `Host` trait is implemented by the app and the headless host. `document()` is
  read-only and `document_mut()` mutable, so requests can read the document while
  holding `libraries()` references (no library clones). `Request` variants
  are handled in `describe` and `respond` (`host.rs`). Library requests may answer later
  (`agent::Job`) because they can add tabs.
- `execute` takes a batch of commands (one undo step, all-or-nothing) and advertises the
  full command JSON schema. Commands arrive as JSON and are parsed one by one just
  before they run (`run_commands`), so `sheet` / `parent` given by name or path can
  name a sheet an earlier command of the batch added. `screenshot`, `get_selection`
  and `set_selection` take a `sheet` (`ViewSpec::resolve`). `edit_library` takes the file format with merges by id or
  name, refuses edits that add new `Library::issues`, and reports pad coverage.
- Tools are listed in README.md.

## App (`app`)

- **`PcbApp` state:** the project document is `doc()` / `doc_mut()` (it lives in the
  Project Library's `LibraryEditor`); sites that borrow other fields at the same time
  use the field path `self.project.lib.doc`. Loaded libraries are `Libraries`
  (`by_key`, `add`, `remove`, `sync` with the project's list, `resync` after a save).
  The tab tree is drawn by `TreeBehavior`, which borrows fields one by one; what it
  can't do itself goes into `deferred` (`Deferred`: close a tab, enter a sheet, Gerber
  actions), handled right after the tree is drawn.
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
- **Floating tool bars** (`toolbar.rs`, vector icons in `icons.rs`) are drawn after the
  canvas and sense click-and-drag, so presses never reach the canvas. View buttons
  (layers, grid/units/snap, fit) sit at the bottom left.
- **Tool windows** (`dock.rs`): icon bars on both edges, one open panel per side, never
  opened automatically. Project (⌘1), Messages (⌘3, with an unseen-problems badge),
  Properties, Tool settings, Board setup, Overview. Saved in settings when no pointer
  button is down. Panel content is drawn through `dock::fixed_size`, which clips
  overflow: egui grows a resizable panel to fit wider content and keeps that size, and
  grids overflow for a frame when they reuse the column widths of the previous
  selection.
- **Properties** (`properties.rs`): a symbol and a footprint both have the component's
  Footprint variant combo (`variant_combo`); on the board it leaves out "no footprint"
  variants, which would delete the selected footprint. Text fields keep their typed
  text in buffers keyed by (object, field): the object must be a selected one (others'
  buffers are dropped each frame), and fields being edited are tracked explicitly (focus
  can move to a field drawn earlier in the frame before the edited one sees it go).
- **Project tree** (`project_panel.rs`): project (the root sheet as "Schematic" with its
  sub-sheets nested by name, PCB, Project Library, Outputs › Gerber), Built-in Library (read-only),
  External Libraries; new sheets come from the + menu or a sheet's menu (name dialog,
  then the symbol follows the cursor); menus and activation are pure functions returning `Act`s. Gotcha: egui
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
  recent projects, changed key bindings, dock.
- **Dialogs:** use `theme::dialog` (modal, accent title; bold is faked by
  `theme::bold_label`) and, for non-modal resizable ones, `theme::window` (same frame
  and title bar, a × button, opens centred; Settings, KiCad import). Bus prompts are `name_prompt.rs`.
- **GPU canvases** (`canvas.rs`): each `Canvas` takes a unique id and its own `GpuScene`
  in `CanvasResources`; dropping the canvas queues the id and the next `prepare` frees
  the scene. Gotcha: a new canvas only receives the scene's incremental uploads, so it
  must start with a scene that wasn't uploaded elsewhere (a fresh `ItemView`, or keep
  the canvas with its scene).
- **Previews** (`preview.rs`): `Preview` is a view-only `ItemView` + `Canvas` in a box
  (fitted every frame to `ItemView::content_bounds`, no grid, axis or origin, square
  corners with a border, a hint or error text when empty). It restyles itself on a
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
| Live sync per schematic edit (spike, 600 random edits) | p50 2.5 ms, max 3.6 ms |
| Schematic connectivity, full rebuild | ~13 ms |
| Board copper connectivity (20k pads) | ~6 ms |
| Zone fill + connectivity + render, whole-board zone | 0.10 s (2k) / 0.54 s (10k) |
| DRC | 8.5 ms (2k) / 13 ms (10k) |
| Redraw users of one library edit (3,333 capacitors) | ~21 ms + 18 ms (old full rebuild ~200 ms) |
| Drag step | ~0.035 ms |
| Both canvases animated, ~3 M primitives | 90–170 fps uncapped |

Measure with `edit_bench` (`cargo run -p view --release --example edit_bench`),
`sync_bench` (`spike`) and `--stress N`.

## Later

Things decided to do later, by area. Remove an entry when it's done; add what a change
defers.

**Schematic**
- KiCad project export. Import gaps (counted in the import report today): schematic
  graphics, images, text boxes, tables and rule areas; text variables (`${TITLE}` etc. in
  texts stay as written; the frame shows the title block);
  bus aliases; directive labels (net class flags); per-instance symbol field positions
  (fields follow the symbol); stored sheet pin positions (laid out by name); reused
  sheets are duplicated (no reuse here); bus entries of unusual sizes.
- Multi-line text: the font stacks lines, but the Text properties field is single-line.
- Drawing frames: custom KiCad drawing sheets (`.kicad_wks`; the default frame is used),
  an "update the date on save" option, text variables in schematic texts, printing and
  PDF export of the pages.
- After sheets: re-parenting sheets, moving objects between sheets, hierarchical bus
  pins (bus-named hierarchical labels are drawn as pins but don't connect), ERC of pin
  directions across sheet pins.
- ERC: a PWR_FLAG equivalent (a power input fed from a connector's passive pin needs a
  power port today), configurable severities and a pin conflict matrix, and excluding
  single findings.
- KiCad's "unfold bus".
- Wire posture that flips automatically with the cursor.
- Dragging wire segments themselves.
- Double click on a wire to select entire wire instead of only the clicked segment
- When wires overlap during drag they should not connect.
- Editing text on the canvas.
- Per-pad mask and paste margins (board-wide today); inner-layer footprint graphics.
- Footprint texts: re-sync fields when the library footprint's templates change; keep
  texts upright (readable) when a footprint turns past 90°; fields of other texts than
  `${REF}` / `${VALUE}`.
- Custom power symbols: power ports drawn from a symbol of their own (a part flagged
  power, no footprint), for glyphs beyond `PowerStyle`; maps to KiCad custom power
  symbols on import and export.
- More corner styles for wires and buses
- Icon buttons for corner styles in tool panel

**Board**
- Board parts: copy/paste and duplicate on the board; a "Place board part" picker in the
  PCB tool bar; editing their reference and value on the board (read-only today).
- Fabrication outputs: fab/courtyard Gerbers, `.gbrjob`, pick and place, BOM, Protel
  file extensions, `%TO` object attributes (nets, pads),
  silkscreen clipped at mask openings, mirrored bottom text in the editor; in the viewer:
  the mask area from the outline's contour (bounding box today), true clear polarity,
  G74 arcs, LM, thermal primitives, slots in drill files.
- Clipboard: copying sheet symbols (a deep copy of the sub-sheet), carrying a paste
  between the schematic and the board.
- Measure tool: edge-to-edge (clearance) measuring, snapping to a pad's centre from anywhere inside it, an index for
  `board_points` on huge boards.
- Per-pad mask and paste overrides; mask/paste openings in the footprint editor.
- Clearance checking while routing; push-and-shove; dragging tracks that stay connected.
- Double click on a routed track to select entire track instead of only the clicked segment
- Drawing arc tracks: an "Arc" corner style while routing, rounding the corner between
  two tracks (arcs come from import and MCP today).
- Blind and buried vias in the UI.
- Highlight corresponding part when draging texts
- Copper graphics inside footprints (from KiCad custom pads) counting as connections.
- Incremental or background zone refill (only zones near the change).
- Zones: drawing holes into an existing zone (cutout tool), editing corners on the
  canvas, minimum width, hatched fill, a per-pad zone connection override; zones and
  keepouts inside footprints; keepouts enforced while routing.
- Arc outlines in zone fills (circles are 16–64-segment polygons today).
- Exact courtyard polygons in DRC (bounding boxes today).
- A board drawing frame / fabrication drawing.
- Per-net clearance classes (KiCad net classes other than Default are only listed by the
  import).
- From KiCad board import (skipped and counted today): dimensions, groups, images, text
  boxes, tables, alignment targets, graphics and texts on documentation and user layers
  (user layers in general), net ties, tuning patterns (their tracks are kept).
- Autoplacement and autorouting.

**Libraries and parts**
- Drawing tools for arcs and polygons (numeric editing only today).
- Pad arrays and footprint wizards.
- KiCad import: back-side footprint content, text boxes, De Morgan bodies, alternate pin
  functions, exact oval drills, 3D models.

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
- Save the window layout and pane sizes (eframe persistence is off).
- Per-layer colour customisation; UI scale.
- Tool window bars: drag buttons to reorder or move between bars, a Nets panel, two
  panels stacked on one side.
- MCP in settings: Port, enable on startup
- Replace the placeholder stroke font.
- UI icons as SVG files instead of code (`icons.rs`): `egui_extras` `svg` feature
  (resvg) with `install_image_loaders`. Monochrome icons drawn white and tinted
  (`Image::tint`) to follow the theme; multi-colour ones (tab icons) as stacked tinted
  layers or with colours substituted before parsing. Possibly the app icon too.
- An MCP activity log panel listing agent edits.
- macOS: a Liquid Glass app icon made with Icon Composer (needs Xcode's `actool`); more
  content in the title strip; "Fill" on title double-click (no public API, zooms today).
- Native menus don't show shortcuts without ⌘ (they would steal typing).
- Remove weird items in native macOS Edit menu: Autofill, Start dictation, Emoji and symbols?
  Is that even possible? It seems to be there in any app...

**Other**
- Simulation.
