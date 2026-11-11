//! `script`: a program in Rhai run against the running game — its steps
//! are the control socket's own commands, so a script does nothing a
//! client could not, and every command it uses is the one an agent tries
//! by hand first (docs/world-control.md).
//!
//! The program runs on a thread of its own. `cmd(name, #{…})` sends a
//! request to the loop through the desk like any client and blocks until
//! it is answered — after the frame, or many frames later for a wait — so
//! a wait in a script is a plain call, and the game runs on meanwhile. A
//! failed command is a script error at the calling line. The prelude
//! (`PRELUDE`) names the commands (`query`, `get`, `set`, `wait_frames`,
//! `until`, `shot`, `press`, `res`, `call`, …) and adds `test(name, fn)`
//! cases and assertions. No files, network or processes: Rhai has none
//! unless given, and none are.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use rhai::{Dynamic, Engine, EvalAltResult, Map, Position};
use scrap_core::web_time::Instant;

use super::control::{json, Request, Value};

/// What every script starts with: the commands by name, cases and
/// assertions. Its line numbers are not the script's: a script's errors
/// count from the script's own first line.
pub const PRELUDE: &str = r#"
fn query(comps) { cmd("query", #{ "with": comps }) }
fn query(comps, cond) { cmd("query", #{ "with": comps, "where": cond }) }
fn count(comps) { cmd("query", #{ "with": comps, count: true }).count }
fn first(comps) { let q = cmd("query", #{ "with": comps, limit: 1 }); if q.total == 0 { () } else { q.entities[0] } }
fn get(entity) { cmd("get", #{ entity: entity }).components }
fn get(entity, component) { cmd("get", #{ entity: entity, component: component }) }
fn get(entity, component, path) { cmd("get", #{ entity: entity, component: component, path: path }) }
fn set(entity, component, value) { cmd("set", #{ entity: entity, component: component, value: value }) }
fn patch(entity, component, path, value) { cmd("set", #{ entity: entity, component: component, path: path, value: value }) }
fn add(entity, component) { cmd("add", #{ entity: entity, component: component }) }
fn add(entity, component, value) { cmd("add", #{ entity: entity, component: component, value: value }) }
fn remove(entity, component) { cmd("remove", #{ entity: entity, component: component }) }
fn spawn_entity(components) { cmd("spawn", #{ components: components }).spawned }
fn despawn(entity) { cmd("despawn", #{ entity: entity }) }
fn snapshot(name) { cmd("snapshot", #{ name: name }) }
fn diff(a) { cmd("diff", #{ a: a }) }
fn revert(n) { cmd("revert", #{ n: n }) }
fn pause() { cmd("pause", #{}) }
fn resume() { cmd("resume", #{}) }
fn step(n) { cmd("step", #{ steps: n }) }
fn wait_frames(n) { cmd("frames", #{ frames: n }) }
fn wait_until(cond) { cmd("until", #{ "where": cond }) }
fn wait_until(cond, seconds) { cmd("until", #{ "where": cond, timeout: seconds }) }
fn shot(path) { cmd("shot", #{ path: path }) }
fn shot(path, half) { cmd("shot", #{ path: path, half: half }) }
fn perf() { cmd("perf", #{}) }
fn input(events) { cmd("input", #{ events: events }) }
fn res(name) { cmd("res", #{ op: "get", name: name }) }
fn res(name, path) { cmd("res", #{ op: "get", name: name, path: path }) }
fn res_set(name, path, value) { cmd("res", #{ op: "set", name: name, path: path, value: value }) }
fn game(command, args) { cmd("call", #{ command: command, args: args }) }
fn press(player, action) { cmd("press", #{ player: player, action: action }) }
fn hold(player, action, seconds) { cmd("hold", #{ player: player, action: action, seconds: seconds }) }
fn wait_for(args) { cmd("wait", args) }
fn assert_true(ok, what) { if !ok { throw "assertion failed: " + what } }
fn assert_eq(a, b, what) { if a != b { throw "assertion failed: " + what + ": " + a + " != " + b } }
fn assert_lt(a, b, what) { if !(a < b) { throw "assertion failed: " + what + ": " + a + " is not under " + b } }
fn test(name, f) {
    case_begin(name);
    try { f.call(); case_end(name, true, ""); } catch (e) { case_end(name, false, "" + e); }
}
"#;

/// A script under way: its thread's result when it ends.
pub(super) struct Running {
    pub done: std::sync::mpsc::Receiver<Value>,
    pub cancel: Arc<AtomicBool>,
}

/// Start `source` on a thread of its own; its commands go to `to_loop`.
pub(super) fn start(source: String, library: Option<String>, to_loop: Sender<Request>) -> Running {
    let (tell, done) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let _ = std::thread::Builder::new().name("scrap-script".into()).spawn(move || {
        let _ = tell.send(run_with(&source, library.as_deref(), to_loop, stop));
    });
    Running { done, cancel }
}

/// One command from a script: sent as a client's, waited for.
fn command(to_loop: &Sender<Request>, name: &str, args: Value) -> Result<Value, String> {
    let (reply, answer) = channel();
    let request = Request { id: Value::Null, cmd: name.to_string(), args, reply, began: Instant::now(), turn: 0, until: None, drawn_then: 0 };
    to_loop.send(request).map_err(|_| "the game is gone".to_string())?;
    let line = answer.recv().map_err(|_| "the game is gone".to_string())?;
    let v: Value = json::from_str(&line).map_err(|e| e.to_string())?;
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(v.get("error").and_then(Value::as_str).unwrap_or("failed").to_string())
    }
}

fn line_of(p: Position) -> Option<usize> {
    p.line()
}

/// Run a program; what it came to: `{status, value | error, line, cases, log, ms}`.
#[cfg(test)]
pub(super) fn run(source: &str, to_loop: Sender<Request>, cancel: Arc<AtomicBool>) -> Value {
    run_with(source, None, to_loop, cancel)
}

/// [`run`] with a library of the game's own helpers (`lib=file.rhai`):
/// its functions are the program's, its errors name it.
pub(super) fn run_with(source: &str, library: Option<&str>, to_loop: Sender<Request>, cancel: Arc<AtomicBool>) -> Value {
    let began = Instant::now();
    let mut engine = Engine::new();
    engine.set_max_expr_depths(64, 64);
    let log: Arc<Mutex<Vec<String>>> = Default::default();
    let cases: Arc<Mutex<Vec<Value>>> = Default::default();
    let case_began: Arc<Mutex<Option<Instant>>> = Default::default();
    {
        let log = log.clone();
        engine.on_print(move |s| log.lock().unwrap().push(s.to_string()));
    }
    {
        let log = log.clone();
        engine.on_debug(move |s, _, at| log.lock().unwrap().push(format!("line {}: {s}", at.line().unwrap_or(0))));
    }
    engine.on_progress(move |_| if cancel.load(Ordering::Relaxed) { Some("cancelled".into()) } else { None });
    {
        let to = to_loop.clone();
        engine.register_fn("cmd", move |name: &str, args: Map| -> Result<Dynamic, Box<EvalAltResult>> {
            let args: Value = rhai::serde::from_dynamic(&Dynamic::from_map(args))?;
            let v = command(&to, name, args).map_err(|e| format!("{name}: {e}"))?;
            rhai::serde::to_dynamic(v)
        });
    }
    {
        let to = to_loop.clone();
        engine.register_fn("cmd", move |name: &str| -> Result<Dynamic, Box<EvalAltResult>> {
            let v = command(&to, name, Value::Null).map_err(|e| format!("{name}: {e}"))?;
            rhai::serde::to_dynamic(v)
        });
    }
    {
        let began = case_began.clone();
        engine.register_fn("case_begin", move |_: &str| *began.lock().unwrap() = Some(Instant::now()));
    }
    {
        let (cases, began) = (cases.clone(), case_began.clone());
        engine.register_fn("case_end", move |name: &str, ok: bool, error: &str| {
            let ms = began.lock().unwrap().take().map_or(0.0, |b| b.elapsed().as_secs_f32() * 1e3);
            cases.lock().unwrap().push(json::json!({ "name": name, "ok": ok, "error": if ok { Value::Null } else { Value::String(error.to_string()) }, "ms": ms }));
        });
    }
    let prelude = match engine.compile(PRELUDE) {
        Ok(p) => p,
        Err(e) => return json::json!({ "status": "error", "error": format!("the prelude: {e}") }),
    };
    let program = match engine.compile(source) {
        Ok(ast) => ast,
        Err(e) => return json::json!({ "status": "error", "error": e.to_string(), "line": line_of(e.position()) }),
    };
    let prelude = match library.map(|l| engine.compile(l)) {
        Some(Ok(lib)) => prelude.merge(&lib),
        Some(Err(e)) => return json::json!({ "status": "error", "error": format!("the library: {e}"), "line": line_of(e.position()) }),
        None => prelude,
    };
    let ast = prelude.merge(&program);
    let result = engine.eval_ast::<Dynamic>(&ast);
    let cases = cases.lock().unwrap().clone();
    let failed = cases.iter().filter(|c| c["ok"] == Value::Bool(false)).count();
    let log = log.lock().unwrap().clone();
    let ms = began.elapsed().as_secs_f32() * 1e3;
    match result {
        Ok(v) => json::json!({
            "status": if failed > 0 { "failed" } else { "ok" },
            "value": rhai::serde::from_dynamic::<Value>(&v).unwrap_or(Value::Null),
            "cases": cases, "failed": failed, "log": log, "ms": ms,
        }),
        Err(e) => json::json!({
            "status": if e.to_string().contains("cancelled") { "cancelled" } else { "error" },
            "error": e.to_string(), "line": line_of(e.position()),
            "cases": cases, "failed": failed, "log": log, "ms": ms,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for the loop: answers each request as the desk would,
    /// `frames` a few milliseconds later (a wait), `query` with a count,
    /// anything else as unknown.
    fn fake_loop() -> Sender<Request> {
        let (to_loop, inbox) = channel::<Request>();
        std::thread::spawn(move || {
            let mut hp = 55;
            for r in inbox {
                let answer = match r.cmd.as_str() {
                    "frames" => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        Ok(json::json!({ "turn": 7 }))
                    }
                    "query" => Ok(json::json!({ "total": 2, "count": 2, "entities": [{ "id": 4, "components": ["hero"] }, { "id": 5 }] })),
                    "get" => Ok(json::json!(hp)),
                    "set" => {
                        hp = r.args["value"].as_i64().unwrap_or(0) as i32;
                        Ok(json::json!({}))
                    }
                    other => Err(format!("unknown command `{other}`")),
                };
                r.answer(answer);
            }
        });
        to_loop
    }

    /// A program's commands reach the loop and come back as values; a
    /// wait is a plain call; cases pass and fail on their own; an error
    /// says the script's own line.
    #[test]
    fn a_script_runs_commands_waits_and_cases() {
        let source = r#"
            let n = count("hero");
            wait_frames(3);
            test("two heroes", || assert_eq(count("hero"), 2, "heroes"));
            test("a set lands", || { patch(4, "hero", "hp", 9); assert_eq(get(4, "hero", "hp"), 9, "hp"); });
            test("this one fails", || assert_lt(get(4, "hero", "hp"), 5, "hp"));
            print("done");
            n
        "#;
        let out = run(source, fake_loop(), Default::default());
        assert_eq!(out["value"], 2, "{out}");
        assert_eq!(out["status"], "failed", "{out}");
        assert_eq!(out["failed"], 1);
        let cases = out["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 3);
        assert!(cases[0]["ok"] == true && cases[1]["ok"] == true && cases[2]["ok"] == false, "{out}");
        assert!(cases[2]["error"].as_str().unwrap().contains("is not under 5"), "{out}");
        assert_eq!(out["log"], json::json!(["done"]));

        let broken = run("let a = 1;\nlet b = 2;\ncmd(\"nothing\", #{});\n", fake_loop(), Default::default());
        assert_eq!(broken["status"], "error");
        assert_eq!(broken["line"], 3, "{broken}");
        assert!(broken["error"].as_str().unwrap().contains("unknown command"), "{broken}");
        let syntax = run("let = ;", fake_loop(), Default::default());
        assert_eq!(syntax["line"], 1, "{syntax}");
    }
}
