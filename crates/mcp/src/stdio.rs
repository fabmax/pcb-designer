//! Headless mode: MCP over stdin/stdout for agents without a GUI
//! (`pcb-designer --mcp-stdio [project]`).
//!
//! The protocol runs on a background thread with a small tokio runtime; the calling
//! thread owns the host (document) and answers calls until the client disconnects.
//! Logs must go to stderr: stdout carries the protocol.

use std::sync::mpsc;

use rmcp::ServiceExt;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::host::{Host, handle};
use crate::server::PcbTools;

/// Serve MCP on stdin/stdout until the client disconnects.
pub fn serve_stdio(host: &mut dyn Host) -> Result<(), String> {
    serve(host, rmcp::transport::stdio)
}

/// Serve MCP over any byte stream (`transport` is created on the protocol thread).
pub fn serve<R, W>(host: &mut dyn Host, transport: impl FnOnce() -> (R, W) + Send + 'static) -> Result<(), String>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let (tx, rx) = mpsc::channel();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let protocol = std::thread::Builder::new()
        .name("mcp-stdio".into())
        .spawn(move || {
            runtime.block_on(async move {
                let service = PcbTools::headless(tx).serve(transport()).await.map_err(|e| e.to_string())?;
                service.waiting().await.map_err(|e| e.to_string())?;
                Ok::<(), String>(())
            })
        })
        .map_err(|e| e.to_string())?;
    // Ends when the service (holding the sender) is gone.
    for call in rx {
        handle(call, host);
    }
    protocol.join().map_err(|_| "the MCP thread panicked".to_string())?
}
