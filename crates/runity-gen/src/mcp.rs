//! The module's tools, for the MCP server to list beside its own.
//!
//! The server does not know what they do: it lists [`list`], hands a call
//! it has no answer to to [`call`], and calls [`Generator::poll`] before
//! every tool so a draft that finished lands no matter what the agent asks
//! next.

use std::time::Duration;

use runity::EntityId;
use runity_editor::Session;
use serde_json::{json, Value};

use crate::{drafts, Finished, Generator, Request};

pub fn list() -> Vec<Value> {
    vec![
        json!({
            "name": "generate",
            "description": "Make a draft model with a neural network — for prototyping, not final art — \
                from words or a picture, into assets/drafts/<name>.glb. With `fit`, it replaces a \
                greybox entity: scaled evenly into that entity's box, standing on its floor, painted \
                the draft's colour, one undo step. Takes 20–90 s; waits by default. render afterwards \
                to see it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "snake_case file name: what scenes will call the model, and what the real model replaces later" },
                    "prompt": { "type": "string", "description": "the thing, in English: `an old stone well with a wooden roof`" },
                    "image": { "type": "string", "description": "a picture of the thing instead of a prompt: .png/.jpg/.webp path" },
                    "fit": { "type": "string", "description": "an entity id: the greybox to replace" },
                    "wait": { "type": "boolean", "description": "wait for the model (default true); false returns at once — see generations" },
                },
                "required": ["name"],
            },
        }),
        json!({
            "name": "generations",
            "description": "Drafts being generated and finished this session, and every draft in the project — what a release still has to replace with real models.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
    ]
}

/// Answer one of this module's tools, or `None` if it is not one.
pub fn call(
    generator: Result<&mut Generator, String>,
    session: &mut Session,
    name: &str,
    args: &Value,
) -> Option<Result<String, String>> {
    match name {
        "generate" => Some(generator.and_then(|g| generate(g, session, args))),
        "generations" => Some(Ok(generations(generator.ok(), session))),
        _ => None,
    }
}

fn generate(
    generator: &mut Generator,
    session: &mut Session,
    args: &Value,
) -> Result<String, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_string);
    let name = text("name").ok_or("name is required")?;
    let mut request = match (text("prompt"), text("image")) {
        (Some(prompt), None) => Request::text(&name, prompt),
        (None, Some(image)) => Request::image(&name, image).map_err(|e| format!("{e:#}"))?,
        (Some(_), Some(_)) => return Err("a prompt or an image, not both".into()),
        (None, None) => return Err("a prompt or an image is required".into()),
    };
    if let Some(fit) = text("fit") {
        let id: EntityId = fit
            .parse()
            .map_err(|_| format!("fit is an entity id, 16 hex digits, not {fit:?}"))?;
        request = request.fit(id);
    }
    let id = generator.start(session, request)?;
    if args.get("wait").and_then(Value::as_bool) == Some(false) {
        return Ok(format!(
            "generating `{name}` (job {id}); generations says when it lands"
        ));
    }
    match generator.wait(session, id, Duration::from_secs(300)) {
        Some(finished) => report(&finished),
        None => Ok(format!(
            "`{name}` (job {id}) is still running after five minutes; generations says when it lands"
        )),
    }
}

fn report(finished: &Finished) -> Result<String, String> {
    match &finished.result {
        Ok(applied) => Ok(format!(
            "`{}` is ready in {:.0} s: {}, {:.2} × {:.2} × {:.2} m, colour #{:02x}{:02x}{:02x}{}",
            finished.name,
            finished.seconds,
            applied.file.display(),
            applied.size.x,
            applied.size.y,
            applied.size.z,
            applied.color[0],
            applied.color[1],
            applied.color[2],
            applied
                .fitted
                .map(|id| format!("; it replaced the greybox of {id} (undo takes it back)"))
                .unwrap_or_default()
        )),
        Err(e) => Err(format!(
            "`{}` failed after {:.0} s: {e}",
            finished.name, finished.seconds
        )),
    }
}

fn generations(generator: Option<&mut Generator>, session: &mut Session) -> String {
    let mut out = Vec::new();
    if let Some(generator) = generator {
        for job in generator.running() {
            out.push(format!(
                "running  `{}` (job {}), {:.0} s: {}",
                job.name,
                job.id,
                job.started.elapsed().as_secs_f32(),
                job.stage
            ));
        }
        for finished in generator.finished() {
            out.push(match report(finished) {
                Ok(line) => format!("done     {line}"),
                Err(line) => format!("failed   {line}"),
            });
        }
    }
    let drafts = session.project().map(drafts).unwrap_or_default();
    if drafts.is_empty() {
        out.push("no drafts in the project".into());
    } else {
        out.push(format!("{} draft(s) in assets/drafts/:", drafts.len()));
        for draft in drafts {
            let from = draft
                .record
                .map(|r| format!(" — {:?} via {}", r.from, r.model))
                .unwrap_or_default();
            out.push(format!("  {}{from}", draft.name));
        }
    }
    out.join("\n")
}
