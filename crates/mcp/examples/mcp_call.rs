//! Tiny MCP client for development:
//! `cargo run -p mcp --example mcp_call -- URL TOOL ['{"json": "args"}'] [--out image.png]`
//! With TOOL = `list` it prints the available tools.

use base64::Engine;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig, ContentBlock};
use rmcp::transport::StreamableHttpClientTransport;

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.iter().position(|a| a == "--out").map(|i| {
        let path = args.remove(i + 1);
        args.remove(i);
        path
    });
    let [url, tool, rest @ ..] = &args[..] else {
        eprintln!("usage: mcp_call URL TOOL [JSON_ARGS] [--out FILE.png]");
        std::process::exit(2);
    };
    let transport = StreamableHttpClientTransport::from_uri(url.clone());
    let client = ClientConfig::default().serve(transport).await.expect("connect");
    if tool == "list" {
        for t in client.list_all_tools().await.expect("list tools") {
            println!("{}: {}", t.name, t.description.unwrap_or_default());
        }
    } else {
        let mut params = CallToolRequestParams::new(tool.clone());
        if let Some(json) = rest.first() {
            let v: serde_json::Value = serde_json::from_str(json).expect("arguments must be JSON");
            params = params.with_arguments(v.as_object().cloned().expect("arguments must be an object"));
        }
        let result = client.call_tool(params).await.expect("call");
        if result.is_error == Some(true) {
            eprint!("ERROR: ");
        }
        for c in &result.content {
            match c {
                ContentBlock::Text(t) => println!("{}", t.text),
                ContentBlock::Image(img) => {
                    let png = base64::engine::general_purpose::STANDARD.decode(&img.data).unwrap();
                    let path = out.clone().unwrap_or_else(|| "mcp_screenshot.png".into());
                    std::fs::write(&path, &png).unwrap();
                    println!("image ({} bytes) saved to {path}", png.len());
                }
                other => println!("{other:?}"),
            }
        }
    }
    client.cancel().await.ok();
}
