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

The simulation tests run against ngspice when it's installed (process and shared library;
see [Simulation](simulation.md#installing-ngspice)) and skip otherwise. Tests that render
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

## Calling MCP tools by hand

`cargo run -p mcp --example mcp_call -- URL TOOL '{"json":"args"}'` calls a tool of a
running app (`TOOL = list` lists them).
