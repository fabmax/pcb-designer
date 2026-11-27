//! The browser build's stand-in for the MCP server: a browser can't listen on a port.

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use crate::host::Call;

/// Called after each queued call (see the desktop server).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Never running: [`McpServer::start`] fails.
pub struct McpServer {
    _never: std::convert::Infallible,
}

impl McpServer {
    pub fn start(_port: u16, _wake: Waker) -> Result<(McpServer, Receiver<Call>), String> {
        Err("The MCP server isn't available in the browser".into())
    }

    pub fn url(&self) -> String {
        match self._never {}
    }
}
