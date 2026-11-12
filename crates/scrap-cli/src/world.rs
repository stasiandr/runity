//! `scrap world <command> [key=value …]`: a running game driven from a
//! shell (docs/world-control.md). Finds the game's control socket — the
//! `--socket` given, `SCRAP_CONTROL`, else the newest card
//! (`*.sock.json`) in the temporary folder whose process is alive — sends
//! one request with its token and prints the answer as one line of JSON.
//!
//! Arguments are `key=value`: a value that reads as JSON is JSON
//! (`n=3`, `value={"hp":5}`, `cell=[4,7]`), anything else text
//! (`where=hero.hp < 10` — quote it for the shell). A bare word after the
//! command is the first of the command's own: `scrap world get 42` is
//! `entity=42`; see [`first_arg`]. `scrap world help` lists every
//! command the game answers: the shell's, the world's, the game's own.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

/// The control cards in the temporary folder, newest first.
fn cards() -> Vec<PathBuf> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(std::env::temp_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".sock.json"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    found.sort_by_key(|a| std::cmp::Reverse(a.0));
    found.into_iter().map(|(_, p)| p).collect()
}

fn alive(pid: u64) -> bool {
    std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// The socket and its token: from the card beside the socket.
pub fn find(socket: Option<&Path>) -> Result<(PathBuf, String)> {
    let from_card = |card: &Path| -> Result<(PathBuf, String, u64)> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(card)?)?;
        let socket = PathBuf::from(v["socket"].as_str().context("a card without a socket")?);
        let token = v["token"].as_str().context("a card without a token")?.to_string();
        Ok((socket, token, v["pid"].as_u64().unwrap_or(0)))
    };
    let given = socket.map(Path::to_path_buf).or_else(|| std::env::var_os("SCRAP_CONTROL").map(PathBuf::from).filter(|p| p.as_os_str() != "1"));
    if let Some(socket) = given {
        let (s, t, _) = from_card(&socket.with_extension("sock.json")).with_context(|| format!("no control card beside {}", socket.display()))?;
        return Ok((s, t));
    }
    for card in cards() {
        if let Ok((s, t, pid)) = from_card(&card) {
            if alive(pid) && s.exists() {
                return Ok((s, t));
            }
        }
    }
    bail!("no running game with its control on: start one with its debug flag (a scrap game: SCRAP_CONTROL=1; Not Divinity: ND_DEBUG=1)")
}

/// Send one request and wait for its answer.
pub fn request(socket: &Path, token: &str, cmd: &str, args: Value) -> Result<Value> {
    let stream = std::os::unix::net::UnixStream::connect(socket).with_context(|| format!("{}", socket.display()))?;
    let mut out = stream.try_clone()?;
    let line = json!({ "v": 1, "token": token, "id": 1, "cmd": cmd, "args": args });
    writeln!(out, "{line}")?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer)?;
    Ok(serde_json::from_str(&answer)?)
}

/// What a bare word after a command stands for.
fn first_arg(cmd: &str) -> &'static str {
    match cmd {
        "get" | "set" | "add" | "remove" | "despawn" => "entity",
        "query" | "entities" => "with",
        "snapshot" => "name",
        "diff" => "a",
        "until" => "where",
        "step" => "steps",
        "frames" => "frames",
        "shot" | "script" => "path",
        "res" => "op",
        "call" => "command",
        "cheat" => "line",
        "walk" | "press" | "hold" | "fight" | "stick" => "player",
        "key" => "key",
        "text" => "text",
        "time_scale" => "scale",
        _ => "arg",
    }
}

/// The words after the command, as its arguments.
pub fn arguments(cmd: &str, words: &[String]) -> Value {
    let mut args = serde_json::Map::new();
    let mut bare = 0;
    for w in words {
        let (k, v) = match w.split_once('=') {
            Some((k, v)) if !k.is_empty() && !k.contains(' ') => (k.to_string(), v.to_string()),
            _ => {
                // Bare words: the command's first, then by position.
                let k = match (cmd, bare) {
                    (_, 0) => first_arg(cmd).to_string(),
                    ("get" | "set" | "add" | "remove", 1) => "component".into(),
                    ("get" | "set", 2) => "path".into(),
                    ("res", 1) => "name".into(),
                    ("press" | "hold", 1) => "action".into(),
                    (_, n) => format!("arg{n}"),
                };
                bare += 1;
                (k, w.clone())
            }
        };
        let v = serde_json::from_str::<Value>(&v).unwrap_or(Value::String(v));
        args.insert(k, v);
    }
    // `set E C path value`: the last bare word is the value.
    if cmd == "set" && !args.contains_key("value") {
        if let Some(v) = args.remove("arg3") {
            args.insert("value".into(), v);
        } else if let (Some(v), true) = (args.get("path").cloned(), args.contains_key("component")) {
            if bare == 3 {
                args.insert("value".into(), v);
                args.remove("path");
            }
        }
    }
    Value::Object(args)
}

/// `scrap world …`.
pub fn run(rest: &[String]) -> Result<std::process::ExitCode> {
    let mut socket: Option<PathBuf> = None;
    let mut words: Vec<String> = Vec::new();
    let mut it = rest.iter();
    while let Some(w) = it.next() {
        match w.as_str() {
            "--socket" => socket = Some(PathBuf::from(it.next().context("--socket wants a path")?)),
            _ => words.push(w.clone()),
        }
    }
    let Some((cmd, rest)) = words.split_first() else {
        bail!("scrap world <command> [key=value …] — `scrap world help` lists them");
    };
    let (socket, token) = find(socket.as_deref())?;
    let answer = request(&socket, &token, cmd, arguments(cmd, rest))?;
    let ok = answer.get("ok").and_then(Value::as_bool) == Some(true);
    if ok {
        println!("{}", answer.get("result").cloned().unwrap_or(Value::Null));
    } else {
        eprintln!("{}", answer);
    }
    Ok(if ok { std::process::ExitCode::SUCCESS } else { std::process::ExitCode::FAILURE })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_become_a_commands_arguments() {
        let w = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(arguments("get", &w(&["42", "hero", "hp"])), json!({ "entity": 42, "component": "hero", "path": "hp" }));
        assert_eq!(arguments("set", &w(&["42", "hero", "hp", "5"])), json!({ "entity": 42, "component": "hero", "path": "hp", "value": 5 }));
        assert_eq!(arguments("query", &w(&["hero", "where=hero.hp < 10", "limit=5"])), json!({ "with": "hero", "where": "hero.hp < 10", "limit": 5 }));
        assert_eq!(arguments("fight", &w(&["p1", "op=move", "cell=[3,4]"])), json!({ "player": "p1", "op": "move", "cell": [3, 4] }));
        assert_eq!(arguments("press", &w(&["p2", "confirm"])), json!({ "player": "p2", "action": "confirm" }));
    }
}
