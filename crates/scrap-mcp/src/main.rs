//! `scrap-mcp` — the editor as an MCP server, over stdio.
//!
//! ```text
//! claude mcp add scrap -- cargo run -q -p scrap-mcp
//! ```
//!
//! One JSON-RPC message per line in, one reply per line out. Only protocol
//! goes to stdout; anything else goes to stderr.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn main() {
    let mut server = scrap_mcp::Server::new();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => server.handle(&message),
            Err(e) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("not JSON: {e}") },
            })),
        };
        if let Some(reply) = reply {
            if writeln!(stdout, "{reply}")
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
        }
    }
}
