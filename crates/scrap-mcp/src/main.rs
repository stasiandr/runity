//! `scrap-mcp` — the editor as an MCP server, over stdio.
//!
//! ```text
//! claude mcp add scrap -- cargo run -q -p scrap-mcp
//! scrap-mcp [--project PATH] [--headless]
//! ```
//!
//! One JSON-RPC message per line in, one reply per line out. Only protocol
//! goes to stdout; anything else goes to stderr.
//!
//! When a scrap studio has the project open — the one named by `--project`
//! or `SCRAP_PROJECT`, else the one the working folder is in or holds —
//! every line goes on to that window ([`scrap_mcp::live`]): the agent edits
//! what the person sees. Otherwise, or with `--headless`
//! (`SCRAP_MCP_HEADLESS=1`), the server is an editor of its own, headless.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

fn main() {
    let mut project = std::env::var_os("SCRAP_PROJECT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let mut headless = std::env::var("SCRAP_MCP_HEADLESS").is_ok_and(|v| v != "0");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project" => project = args.next().map(PathBuf::from),
            "--headless" => headless = true,
            other => {
                eprintln!(
                    "scrap-mcp: unknown argument {other}; scrap-mcp [--project PATH] [--headless]"
                );
                std::process::exit(2);
            }
        }
    }

    let mut server = scrap_mcp::Server::new();
    let mut studio = Studio::new(project, headless);
    studio.look(&mut server);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        // A studio opened after this started is attached to until this
        // server has a session of its own.
        if !server.has_session() {
            studio.look(&mut server);
        }
        let reply = match studio.pass(&line, &mut server) {
            Some(reply) => reply,
            None => match serde_json::from_str::<Value>(&line) {
                Ok(message) => server.handle(&message).map(|v| v.to_string()),
                Err(e) => Some(
                    json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": { "code": -32700, "message": format!("not JSON: {e}") },
                    })
                    .to_string(),
                ),
            },
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

/// The live studio this passes lines to, if any.
struct Studio {
    project: Option<PathBuf>,
    headless: bool,
    #[cfg(unix)]
    attached: Option<scrap_mcp::live::Attached>,
    looked: Option<std::time::Instant>,
}

impl Studio {
    fn new(project: Option<PathBuf>, headless: bool) -> Self {
        Self {
            project,
            headless,
            #[cfg(unix)]
            attached: None,
            looked: None,
        }
    }

    /// Find a studio serving the project and attach to it; the server is
    /// told why not. At most once a second.
    fn look(&mut self, server: &mut scrap_mcp::Server) {
        if self.headless {
            server.set_headless_reason("started with --headless (or SCRAP_MCP_HEADLESS)");
            return;
        }
        if self
            .looked
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(1))
        {
            return;
        }
        self.looked = Some(std::time::Instant::now());
        #[cfg(unix)]
        {
            if self.attached.is_some() {
                return;
            }
            let Some(dir) = scrap_cli::hub::config_dir() else {
                server.set_headless_reason("no home folder, so no studio to find");
                return;
            };
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let found =
                scrap_mcp::live::find(&dir, self.project.as_deref(), &cwd).and_then(|studio| {
                    scrap_mcp::live::Attached::connect(studio.clone()).map_err(|e| {
                        format!(
                            "the studio on {} did not answer: {e}",
                            studio.project.display()
                        )
                    })
                });
            match found {
                Ok(attached) => {
                    eprintln!(
                        "scrap-mcp: attached to the live studio on {} (process {})",
                        attached.studio.project.display(),
                        attached.studio.pid
                    );
                    self.attached = Some(attached);
                }
                Err(why) => server.set_headless_reason(why),
            }
        }
        #[cfg(not(unix))]
        server
            .set_headless_reason("attaching to a studio needs Unix sockets, and this is not Unix");
    }

    /// The studio's reply to a line when attached (`Some(None)` for a
    /// notification); `None` when not — the server answers. A studio that
    /// went away leaves the server headless, and the request in flight is
    /// answered with that.
    #[cfg_attr(not(unix), allow(unused_variables))]
    fn pass(&mut self, line: &str, server: &mut scrap_mcp::Server) -> Option<Option<String>> {
        #[cfg(unix)]
        {
            let attached = self.attached.as_mut()?;
            match attached.exchange(line) {
                Ok(reply) => Some(reply),
                Err(e) => {
                    let why = format!(
                        "the studio on {} went away ({e})",
                        attached.studio.project.display()
                    );
                    eprintln!("scrap-mcp: {why}; headless from now on");
                    self.attached = None;
                    server.set_headless_reason(why.clone());
                    let id = serde_json::from_str::<Value>(line)
                        .ok()
                        .and_then(|v| v.get("id").cloned());
                    Some(id.map(|id| {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32000,
                                "message": format!("{why}: this server is headless now — open_scene to go on here"),
                            },
                        })
                        .to_string()
                    }))
                }
            }
        }
        #[cfg(not(unix))]
        None
    }
}
