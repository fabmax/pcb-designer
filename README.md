# PCB Designer

A schematic and PCB editor written in Rust (egui + wgpu). Schematic changes show up on the
board immediately; projects are folders of JSON files. See [PLAN.md](PLAN.md) for the
architecture and roadmap.

```sh
cargo run --release -p app                 # opens a demo project
cargo run --release -p app -- my.pcbproj   # open a project folder
cargo run --release -p app -- --new        # empty project
cargo test --workspace
```

## Letting an agent use the app (MCP)

The app can run an [MCP](https://modelcontextprotocol.io) server so AI agents can inspect
and edit the open project while you watch. It is off by default.

1. Enable it with **Tools → Agent access (MCP server)**, or start the app with `--mcp`
   (port 7340) or `--mcp-port PORT`.
2. Connect your agent to `http://127.0.0.1:7340/mcp`. Tools → "Copy Claude Code command"
   copies `claude mcp add --transport http pcb-designer http://127.0.0.1:7340/mcp`;
   "Copy JSON config" gives an `mcpServers` entry for other clients.

The server only listens on 127.0.0.1 and rejects requests from web pages (browser
`Origin`) and foreign `Host` names. While it is enabled, any local program can edit the
open project, so only turn it on when you need it.

Tools: `get_project`, `list_library_parts`, `import_parts`, `list_objects`,
`get_objects`, `get_component`, `execute` (any editor command, as one undo step),
`undo`, `redo`, `get_selection`, `set_selection`, `screenshot` (schematic, board or
the whole window) and `save_project`. Agent edits go through the same commands as the
GUI, so they can be undone with ⌘Z, and the status bar shows what the agent just did.

For development, `cargo run -p mcp --example mcp_call -- URL TOOL '{"json":"args"}'`
calls a tool from the command line (`TOOL = list` lists them).
