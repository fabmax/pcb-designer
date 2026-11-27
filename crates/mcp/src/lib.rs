//! MCP interface: lets AI agents inspect and edit the open project.
//!
//! * [`server`]: the MCP tools and a Streamable HTTP server bound to 127.0.0.1.
//! * [`host`]: requests sent from the server to the thread owning the document, and how
//!   they are answered ([`host::handle`]) given a [`host::Host`].
//! * [`library`]: the library tools (create, open, read, edit, save, render libraries).
//! * [`simulation`]: simulate, the SPICE netlist and the plot for agents.
//! * In the browser `server` is a stub that can't start (no sockets) and there's no
//!   stdio mode.
//! * [`headless`]: a host without GUI (tests, headless mode).
//!
//! All edits go through editor-core commands, so they are undoable and show up live.

pub mod headless;
pub mod host;
pub mod library;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
#[cfg(target_arch = "wasm32")]
#[path = "server_web.rs"]
pub mod server;
pub mod simulation;
#[cfg(not(target_arch = "wasm32"))]
pub mod stdio;

pub use host::{Call, Host, Outcome, Reply, Request, handle};
pub use server::{McpServer, Waker};
