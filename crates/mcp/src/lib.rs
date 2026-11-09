//! MCP interface: lets AI agents inspect and edit the open project.
//!
//! * [`server`]: the MCP tools and a Streamable HTTP server bound to 127.0.0.1.
//! * [`host`]: requests sent from the server to the thread owning the document, and how
//!   they are answered ([`host::handle`]) given a [`host::Host`].
//! * [`headless`]: a host without GUI (tests, headless mode).
//!
//! All edits go through editor-core commands, so they are undoable and show up live.

pub mod headless;
pub mod host;
pub mod server;
pub mod stdio;

pub use host::{Call, Host, Reply, Request, handle};
pub use server::{McpServer, Waker};
