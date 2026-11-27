# AGENTS.md

PCB Designer: a schematic, PCB and part editor in Rust (egui + wgpu). Projects are folders
of JSON files, and an MCP server lets agents edit the open project.

- Requirements: [pcb-designer.md](pcb-designer.md).
- Decisions, the design of each area with its gotchas, and the Later list:
  [PLAN.md](PLAN.md). Read its section on the area you're touching before changing a
  design decision.
- User documentation, a file per area: [docs/](docs/README.md). The top-level
  README.md stays short: overview, links to the docs, install, build, license.

## Workflow

Each change (a milestone or a refinement) runs through these steps:

Work on the branch that is checked out (often `main`); don't create branches.

1. Implement, with tests for new behaviour. Logic goes in the GUI-independent crates
   (`editor-core`, `view`), where it's unit-testable headless. Integration tests are one
   binary per crate: a new file in `crates/<crate>/tests/all/` needs a `mod` line in its
   `main.rs`. Run one file with `cargo test -p view --test all sheets::`. (Every test
   binary costs link time and, on macOS, a first-launch check of a second or more;
   `view/tests/theme.rs` stays separate because it switches the global palette.)
2. Make it green: `cargo test --workspace`, `cargo clippy --workspace --all-targets` with
   zero warnings, and `cargo fmt`. GitHub Actions runs the same on macOS, Linux and
   Windows once main is pushed (`.github/workflows/ci.yml`); keep code portable.
3. For anything visible, look at it in a real-window screenshot (see below): both themes,
   and `--side-by-side` when layout is involved.
4. Update PLAN.md. It is a reference, not a changelog (the commit message says what
   changed). Update the section of the area you touched with the current design and the
   gotchas a later change needs to know, keeping it short. Add what you defer to
   **Later**, and remove what you finished from it. When users see the change, update
   the matching file in `docs/` too (how to use it, not how it's built).
5. Commit on the current branch with a message like `M9: copper zones …` or `GUI:
   floating icon tool bars …`. End it with the attribution trailer.
6. Don't push: the user tries changes out first, and pushes to `origin` (public GitHub
   repo `fabmax/pcb-designer`) or asks for it. Then carry on with the next milestone
   without asking.

Saved projects and libraries stay loadable: the project is in use. A format change that
renames, removes or reinterprets a field bumps that file kind's version and adds a
migration (`model::io::MIGRATIONS`); a new field with a default needs neither.
`view/tests/all/saved_files.rs` loads each release's examples (`view/tests/saved/`) and
fails when something in them is lost. Settings: missing and unknown fields fall back to
defaults, but a value that no longer parses (a renamed enum variant) resets them all, so
don't rename those. Keymaps and MCP tools may still change; update tests and docs to match.

## Crates

Dependency direction: `model` → `editor-core` → `view` → `mcp` → `app`. `render` is
standalone, `gerber` sits on `editor-core` and `render` (under `view`), `sim` on
`editor-core` (under `mcp`), and `kicad` sits on `model` and `view`.

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
- `sim`: circuit simulation: the schematic as a SPICE netlist, run by ngspice (its
  shared library or a process), results read back by net and pin.
- `kicad`: imports KiCad symbols and footprints.
- `app`: the egui shell (binary `pcb-designer`). It holds tabs (egui_tiles), panels,
  menus, settings, keymap, chrome theme and floating tool bars.

## Invariants

- Every edit is an `editor-core` `Command`, whether it comes from the GUI or MCP. That
  one path is what makes undo, live sync and validation behave the same for agents and
  humans.
- Sync is one-way, schematic → board. Board edits never change connectivity. Copper whose
  net no longer matches is flagged, never deleted. The one exception to "the board
  follows the schematic" is board parts (parts without symbol: mounting holes, logos):
  they are placed and deleted on the board only, and their pads have no net.
- The app also builds for the browser: keep `cargo clippy -p app --target
  wasm32-unknown-unknown` clean. There, `std::thread::spawn` and
  `std::time::Instant::now()` panic: use `background::spawn` and `web_time`. Files go
  through `model::fs` (not `std::fs` or `Path::exists`), dialogs through
  `file_dialog::FileDialog`, message boxes through `ask`. The
  desktop has priority: ask before a change for the web makes the desktop worse.
- Performance budget: about 10k footprints at 60 fps, and an edit → board → GPU update
  within one frame. Measure with `--release` and `--stress N`. Anything that scans all
  objects per item needs an index (see `RegionIndex`, `rstar`).

## Recipes

- **Keyboard action:** in `app/src/keymap.rs`, update the `Action` enum, `Action::ALL`,
  `label`, `context` and the default binding. Then handle it in `app.rs`; the matches
  are exhaustive, so the compiler lists every site.
- **MCP tool:** add a `#[tool]` in `mcp/src/server.rs`, plus a `Request` variant with
  arms in both `describe` and `respond` in `host.rs`. Test it in `mcp/tests/all`. Update
  the tool list in `docs/agents.md`.
- **Icon:** add an SVG to `crates/app/icons/` and a line to the `icons!` table in
  `app/src/icons.rs`. Prefer a Tabler outline icon: copy the file unchanged from
  `https://raw.githubusercontent.com/tabler/tabler-icons/main/icons/outline/NAME.svg`
  into `icons/tabler/`. Draw our own in Tabler's style (24×24, stroke 2, round caps and
  joins), in `currentColor` only, faint parts with an opacity; `paint` tints it.
- **Built-in library part:** generated families live in `model/src/builtin/generate/`;
  run `cargo run -p model --example gen_builtin` after a change. Hand-made parts: `…
  -- edit FILE`, draw them in the app or through MCP, then `… -- take FILE` (PLAN.md,
  "Built-in library"). Look at them with `cargo run -p mcp --example render_builtin --
  OUT PREFIX…`.
- **Colours:** canvas palettes live in `view/src/style.rs` (`DARK`, `LIGHT`) and chrome
  in `app/src/theme.rs`. Scenes bake colours when drawn, so a theme switch calls
  `Document::restyle`. Read colours from these files, never as literals in UI code.

## egui gotchas

- **Hit testing:** the widget added last on top wins, judged against the previous
  frame's rects. Overlays on a canvas are drawn after it and sense `click_and_drag`.
  A click-only widget lets the canvas underneath still receive drags (`toolbar.rs`).
  A clickable container (a card with buttons inside) senses through
  `UiBuilder::new().sense(…)` in `scope_builder`, which registers it before its children;
  `response.interact(…)` afterwards puts it on top and its buttons stop working.
- **GPU canvases** are paint callbacks that egui can't clip. Rounded corners are masked
  afterwards (`theme::round_corners`).
- **File dialogs:** never call a blocking `rfd::FileDialog` (it can crash winit on
  macOS); use `file_dialog::Dialog` (PLAN.md, "Native file dialogs").
- **Buttons:** `ui.button`, not `small_button` (it looks squashed next to our fields);
  only the keyboard settings' shortcut chips are small. Menus draw buttons without a
  background: a menu that wants real buttons draws them with the global style
  (`toolbar::layer_menu`).
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
  with `{"theme": "light"}` there for the light theme, plus e.g. `"editor_themes":
  {"pcb": "dark"}` to give an editor its own (`schematic`, `pcb`, `simulation`).
- More flags:
  - `--side-by-side`
  - `--example amp` (instead of `--demo`, which is `--example led`): the audio amplifier
  - `--select R1`
  - `--show footprint:SOT-23` (opens an item by name, searching the libraries from the
    last loaded one: with `--demo` the demo parts, then the built-in library, e.g.
    `--show part:74HC00`; `--show gerber` opens the Gerber tab of a saved project with an
    export; `--show simulation:NAME` a simulation's output tab, `--show simulation` the first one's)
  - `--panel messages` (repeatable; shows a panel for this run only, see `dock.rs`)
  - `--script "board; ui-click X,Y; ui-move X,Y; key W; click X,Y; wait N"`, plus
    `press X,Y` / `move X,Y` / `release X,Y` (a drag), `double-click X,Y`,
    `triple-click X,Y`, `ui-double-click X,Y`, `ui-right-click X,Y`, `ui-press X,Y` / `ui-release X,Y` (a drag
    in screen points, with `ui-move` between), `type TEXT` and `run [ANALYSIS]` (runs
    a simulation; later steps wait for it; `start [ANALYSIS]` doesn't wait), and `zoom F` (zooms the canvas under the
    pointer, after a `move` / `ui-move`) and `scroll DY` (scrolls a panel or form under
    the pointer down by DY points), and `hold Alt` (modifiers held for the pointer steps
    that follow, e.g. an Alt-click; `hold` lets go), and `dialog PATH` / `dialog cancel`
    (answers the next file dialog instead of showing it: put it before the step that
    opens one, e.g. `dialog /tmp/x.pcblib; key Cmd+Shift+S`). Hover the canvas
    (`move X,Y`) before keys meant for it; after a click in a panel, keys go to the
    panel (click a tool bar button instead of its key). Keys take modifiers
    (`key Cmd+D`); `key Cmd+C` / `Cmd+V` use the real system clipboard. Put a `wait 30` before a
    double-click: egui counts clicks less than 0.3 s apart as one triple-click.
  - World coordinates are in mm (schematic and board only; a sheet's origin is its page's
    lower left corner, the demo's parts sit around 100,100); `ui-*` steps take screen
    points (use them in library and Gerber tabs).
  - A script brings the schematic tab to the front at its start (unless `--show` opened
    a tab) and on `board` / `schematic` steps; clicks may switch tabs afterwards.
  - `ui-click` on menus is unreliable.
- The browser build (PLAN.md, "Web"): `cd crates/app && trunk build --release`, serve
  `target/web/` (`python3 -m http.server -d target/web 8765`), then `node
  crates/app/web-shot.mjs http://127.0.0.1:8765/ OUT.png "click X,Y; drag X,Y X2,Y2;
  key r; wait MS; shot F.png; eval JS"` (headless Chrome, CSS pixels; prints the
  console).
- Without a window: `cargo run -p mcp --example render_board -- out.png [w h] [--light]
  [--schematic] [--example amp] [--show-layer "Top Solder Mask"]`.
- Gerber files of the demo: `cargo run -p view --example export_demo -- DIR`.
- A KiCad export of an example or of an imported KiCad project: `cargo run -p kicad
  --example export_kicad -- led|amp|PROJECT.kicad_pro DIR`. With `kicad-cli` installed,
  `KICAD_CLI=$(which kicad-cli) KICAD_TEMPLATE_DIR=…/kicad/demos cargo test -p kicad --test
  all project_export` checks exports against KiCad itself.
- Call a running app's MCP tool: `cargo run -p mcp --example mcp_call -- URL TOOL '{…}'`.
