# Development

PCB Designer is a Cargo workspace of crates; the dependency order, the invariants and the
workflow for changes are in [AGENTS.md](../AGENTS.md), and the design of each area with
its gotchas, plus what's planned, in [PLAN.md](../PLAN.md). Requirements are in
[pcb-designer.md](../pcb-designer.md).

## Building and testing

```sh
cargo run --release -p app -- --demo     # the app with the demo project
cargo test --workspace
cargo clippy --workspace --all-targets   # zero warnings
cargo fmt
```

Linux needs the window system's development packages; on Debian/Ubuntu:

```sh
sudo apt install libwayland-dev libxkbcommon-dev libx11-dev libxcursor-dev libxi-dev \
  libxrandr-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

The simulation tests run against ngspice (in the test process and in worker processes)
and skip without it (with `REQUIRE_NGSPICE` set, as in CI, they fail instead). Build the version the app bundles once with
`scripts/build-ngspice.sh` (a C compiler, make, bison, flex and patch; on Windows in an
MSYS2 UCRT64 shell): it goes to `target/ngspice/`, where tests and `cargo run` find it.
Without it, an installed libngspice is used. Tests that render
offscreen skip without a GPU adapter (on Linux, Mesa's `mesa-vulkan-drivers` provides a
software one).

## macOS app bundle

`scripts/bundle-macos.sh` builds `target/release/bundle/PCB Designer.app` (`--install`
copies it to /Applications). Run from the bundle, the app is named "PCB Designer" in the
menu bar and Dock; run with `cargo run`, macOS uses the binary's name, `pcb-designer`.

## CI and releases

GitHub Actions runs the tests on macOS, Linux and Windows for every push to `main` and
every pull request (**CI**), plus fmt and clippy on Linux.

The **Release** workflow builds the macOS app (Apple Silicon) and the Linux and Windows
(x86_64) binaries. Run by hand, it keeps them as workflow artifacts; a version tag
publishes them as a GitHub Release:

1. Set the version in the workspace `Cargo.toml` (`version = "0.3.0"`) and commit.
2. `git tag -a v0.3.0 -m "pcb-designer 0.3.0"`, then push `main` and the tag.

## Example projects

The audio amplifier example is a project folder, `crates/view/demo/amp.pcbproj`, built
into the app. To change it, open it (`--example amp`), edit, save it anywhere, and copy
`project.json`, `library.json`, `schematic.json` and `board.json` over the stored ones.
After a file format change, `cargo run -p view --example save_examples` re-saves the
stored examples (a test fails until then).

The README's screenshots (`docs/images/amplifier-*.png`) are of this example, in the
light theme (a config folder with `{"theme": "light"}` in its `settings.json`), taken
with `--example amp --screenshot FILE` and halved to 1600×1000: the schematic with
`--script "ui-move 690,560; zoom 1.35; ui-move 1240,960"`, the board with `--script
board`, the spectrum with `--show simulation:Transient --script "run; ui-click 430,85"`.

## Calling MCP tools by hand

`cargo run -p mcp --example mcp_call -- URL TOOL '{"json":"args"}'` calls a tool of a
running app (`TOOL = list` lists them).
