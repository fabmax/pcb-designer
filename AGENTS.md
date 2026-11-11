# AGENTS.md

PCB Designer: a schematic, PCB and part editor in Rust (egui + wgpu). Projects are folders
of JSON files, and an MCP server lets agents edit the open project.

- Requirements: [pcb-designer.md](pcb-designer.md).
- Decisions, the design of each area with its gotchas, and the Later list:
  [PLAN.md](PLAN.md). Read its section on the area you're touching before changing a
  design decision.
- User-facing usage (CLI, MCP setup, settings): [README.md](README.md).

## Workflow

Each change (a milestone or a refinement) runs through these steps:

1. Branch off `main`.
2. Implement, with tests for new behaviour. Logic goes in the GUI-independent crates
   (`editor-core`, `view`), where it's unit-testable headless.
3. Make it green: `cargo test --workspace`, `cargo clippy --workspace --all-targets` with
   zero warnings, and `cargo fmt`.
4. For anything visible, look at it in a real-window screenshot (see below): both themes,
   and `--side-by-side` when layout is involved.
5. Update PLAN.md. It is a reference, not a changelog (the commit message says what
   changed). Update the section of the area you touched with the current design and the
   gotchas a later change needs to know, keeping it short. Add what you defer to
   **Later**, and remove what you finished from it.
6. Commit with a message like `M9: copper zones …` or `GUI: floating icon tool bars …`.
   End it with the attribution trailer.
7. `git checkout main && git merge --no-ff <branch>`, with a `Merge …` message. Then
   `git push` main to `origin` (private GitHub repo `fabmax/pcb-designer`). Then carry
   on with the next milestone without asking.

Breaking changes are fine: the project is far from production. Rename or remove freely
(file formats, settings, keymaps, MCP tools), and update tests and docs to match.

## Crates

Dependency direction: `model` → `editor-core` → `view` → `mcp` → `app`. `render` is
standalone, `gerber` sits on `editor-core` and `render` (under `view`), and `kicad` sits on
`model` and `view`.

- `model`: pure data. Lengths are integer nanometres (`Coord`), every object has a UUID,
  and `Store` keeps insertion order so saved files diff well.
- `editor-core`: `Command`s run as atomic transactions with undo/redo. This crate also
  holds schematic → board sync, connectivity, copper/ratsnest, zones and DRC.
- `render`: the wgpu SDF instanced renderer, plus offscreen PNG rendering.
- `view`: draws the model into scenes incrementally. It also holds `Document` (editor
  plus both views), `Interaction` (select, drag, box, keys), the placing tools, and
  palettes (`style.rs`). It's GUI-toolkit independent.
- `mcp`: MCP tools (`server.rs`), and their execution against a `Host` (`host.rs`, with
  `describe` and `respond`). It also has the headless host and stdio mode.
- `gerber`: Gerber X2 and Excellon export of the board (and the zip and manifest).
- `kicad`: imports KiCad symbols and footprints.
- `app`: the egui shell (binary `pcb-designer`). It holds tabs (egui_tiles), panels,
  menus, settings, keymap, chrome theme and floating tool bars.
- `spike`: frozen milestone-0 benchmarks. Leave it alone.

## Invariants

- Every edit is an `editor-core` `Command`, whether it comes from the GUI or MCP. That
  one path is what makes undo, live sync and validation behave the same for agents and
  humans.
- Sync is one-way, schematic → board. Board edits never change connectivity. Copper whose
  net no longer matches is flagged, never deleted.
- Performance budget: about 10k footprints at 60 fps, and an edit → board → GPU update
  within one frame. Measure with `--release` and `--stress N`. Anything that scans all
  objects per item needs an index (see `RegionIndex`, `rstar`).

## Recipes

- **Keyboard action:** in `app/src/keymap.rs`, update the `Action` enum, `Action::ALL`,
  `label`, `context` and the default binding. Then handle it in `app.rs`; the matches
  are exhaustive, so the compiler lists every site.
- **MCP tool:** add a `#[tool]` in `mcp/src/server.rs`, plus a `Request` variant with
  arms in both `describe` and `respond` in `host.rs`. Test it in `mcp/tests`. Update
  the tool list in README.md.
- **Colours:** canvas palettes live in `view/src/style.rs` (`DARK`, `LIGHT`) and chrome
  in `app/src/theme.rs`. Scenes bake colours when drawn, so a theme switch calls
  `Document::restyle`. Read colours from these files, never as literals in UI code.

## egui gotchas

- **Hit testing:** the widget added last on top wins, judged against the previous
  frame's rects. Overlays on a canvas are drawn after it and sense `click_and_drag`.
  A click-only widget lets the canvas underneath still receive drags (`toolbar.rs`).
- **GPU canvases** are paint callbacks that egui can't clip. Rounded corners are masked
  afterwards (`theme::round_corners`).
- **egui 0.37 deprecations:** use `Id::unique` / `IdSalt` instead of `Id::new`, and
  `ui.scope_id()` instead of `ui.id()`.

## Seeing the app

Real-window screenshots (the window is 1600×1000 points and the PNG is 2×):

```sh
cargo build -p app
(PCB_DESIGNER_CONFIG=/tmp/pcbcfg ./target/debug/pcb-designer --demo \
   --screenshot /tmp/shots/x.png --frames 40 &)
# Poll until the file exists, then: pkill -f "debug/pcb-designer --demo"
sips -Z 1600 /tmp/shots/x.png --out /tmp/shots/x_small.png   # view at a readable size
sips -c H W --cropOffset Y X /tmp/shots/x.png --out crop.png  # close-up of the full-size image
```

- `PCB_DESIGNER_CONFIG` points at a throwaway settings folder. Add a `settings.json`
  with `{"theme": "light"}` there for the light theme.
- More flags:
  - `--side-by-side`
  - `--select R1`
  - `--show footprint:0805` (opens a library item)
  - `--panel messages` (repeatable; shows a panel for this run only, see `dock.rs`)
  - `--script "board; ui-click X,Y; ui-move X,Y; key W; click X,Y; wait N"`, plus
    `ui-double-click X,Y`, `ui-right-click X,Y` and `type TEXT`. Put a `wait 30` before a
    double-click: egui counts clicks less than 0.3 s apart as one triple-click.
  - World coordinates are in mm; `ui-*` steps take screen points.
  - A script brings the schematic tab to the front at its start and on `board` /
    `schematic` steps; clicks may switch tabs afterwards.
  - `ui-click` on menus is unreliable.
- Without a window: `cargo run -p mcp --example render_board -- out.png [w h] [--light]
  [--schematic] [--show-layer "Top Solder Mask"]`.
- Gerber files of the demo: `cargo run -p view --example export_demo -- DIR`.
- Call a running app's MCP tool: `cargo run -p mcp --example mcp_call -- URL TOOL '{…}'`.
