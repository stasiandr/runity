//! Subgraphs: `shaders/<name>.subgraph.ron`, a piece of a graph with named
//! inputs and outputs that any graph — a material's or a particle
//! effect's — calls as one node, Unity's Sub Graph.
//!
//! ```ron
//! (
//!     inputs: { "at": "position", "scale": 1.0 },
//!     nodes: {
//!         "cells": Voronoi(at: "at.xz", scale: "scale"),
//!         "crack": Subtract(a: "cells.y", b: "cells.x"),
//!     },
//!     outputs: { "crack": "crack", "cell": "cells.x" },
//! )
//! ```
//!
//! and in a graph `"rock": Subgraph(name: "cracks", inputs: { "scale": 3.0 })`,
//! read as `rock.crack` and `rock.cell` (a subgraph with one output is
//! read as `rock`). An input left out takes the subgraph's default — a
//! value, or a built-in (`position`) of the graph it is put in.
//!
//! A call is put in before the graph is compiled: its nodes under the
//! call's name (`rock__cells`), each input a node of its own
//! (`rock__scale`), each output one (`rock__out__crack`), and every read of
//! `rock.crack` made a read of that. So what the subgraph does wrong is
//! said of those names, and a subgraph may call another — not itself.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::expr::{self, Input, Node};

/// The file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubGraph {
    /// Its inputs by name, each with what it is when a call leaves it out.
    #[serde(default)]
    pub inputs: BTreeMap<String, Input>,
    #[serde(default)]
    pub nodes: BTreeMap<String, Node>,
    /// What it gives, by name.
    pub outputs: BTreeMap<String, Input>,
}

/// What a file of this name is called as a subgraph: `cracks.subgraph.ron`
/// is `cracks`. `None` for any other file.
pub fn subgraph_name(file_name: &str) -> Option<&str> {
    file_name
        .strip_suffix(".subgraph.ron")
        .filter(|n| !n.is_empty())
}

/// Read a subgraph from its text; the error says where.
pub fn parse(text: &str) -> Result<SubGraph, String> {
    expr::from_ron(text).map_err(|e| e.to_string())
}

/// Where a graph's subgraphs are found, by name.
pub trait Library {
    fn subgraph(&self, name: &str) -> Result<SubGraph, String>;
}

/// No subgraphs at all.
pub struct NoSubgraphs;

impl Library for NoSubgraphs {
    fn subgraph(&self, name: &str) -> Result<SubGraph, String> {
        Err(format!("no subgraph `{name}`: this graph has none to call"))
    }
}

/// The subgraphs in a folder: `<name>.subgraph.ron` beside the graph.
pub struct Folder(pub std::path::PathBuf);

impl Library for Folder {
    fn subgraph(&self, name: &str) -> Result<SubGraph, String> {
        let path = self.0.join(format!("{name}.subgraph.ron"));
        let text = scrap_core::files::read_to_string(&path).map_err(|_| {
            let known: Vec<String> = scrap_core::files::list(&self.0)
                .unwrap_or_default()
                .iter()
                .filter_map(|p| {
                    p.file_name()
                        .and_then(|f| f.to_str())
                        .and_then(subgraph_name)
                        .map(str::to_string)
                })
                .collect();
            let hint = scrap_core::spelling::closest(name, known.iter().map(String::as_str))
                .map(|n| format!(" — did you mean `{n}`?"))
                .unwrap_or_default();
            format!("no subgraph `{name}`: there is no {}{hint}", path.display())
        })?;
        parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The subgraphs a graph can call: `<name>.subgraph.ron` beside it first,
/// then of that name anywhere in its project (docs/layout.md) — a feature's
/// graph calls a shared subgraph where it lies.
pub struct InProject {
    /// The graph's own folder.
    pub beside: std::path::PathBuf,
    /// The project's root, when the graph is in one.
    pub root: Option<std::path::PathBuf>,
}

impl InProject {
    /// For the graph at `path`: its folder, and the nearest folder above it
    /// with a `scrap.ron`.
    pub fn of(path: &std::path::Path) -> Self {
        let beside = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        let root = beside
            .ancestors()
            .find(|dir| scrap_core::files::is_file(dir.join(scrap_core::project::FILE)))
            .map(std::path::Path::to_path_buf);
        Self { beside, root }
    }
}

impl Library for InProject {
    fn subgraph(&self, name: &str) -> Result<SubGraph, String> {
        let file = format!("{name}.subgraph.ron");
        if scrap_core::files::is_file(self.beside.join(&file)) {
            return Folder(self.beside.clone()).subgraph(name);
        }
        let Some(root) = &self.root else {
            return Folder(self.beside.clone()).subgraph(name);
        };
        let all = scrap_core::layout::files(root, scrap_core::layout::Kind::Shader);
        if let Some(path) = all.iter().find(|p| p.file_name().and_then(|f| f.to_str()) == Some(file.as_str())) {
            let text = scrap_core::files::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            return parse(&text).map_err(|e| format!("{}: {e}", path.display()));
        }
        let known: Vec<String> = all
            .iter()
            .filter_map(|p| p.file_name().and_then(|f| f.to_str()).and_then(subgraph_name).map(str::to_string))
            .collect();
        let hint = scrap_core::spelling::closest(name, known.iter().map(String::as_str))
            .map(|n| format!(" — did you mean `{n}`?"))
            .unwrap_or_default();
        Err(format!("no subgraph `{name}`: no {file} beside the graph or anywhere in the project{hint}"))
    }
}

/// A call's output node: what `call.output` reads.
fn output_node(call: &str, output: &str) -> String {
    format!("{call}__out__{output}")
}

/// `nodes` with every subgraph call put in, and `outputs` — what the graph
/// sets — with their reads of calls made reads of what was put in.
pub fn expand(
    nodes: &BTreeMap<String, Node>,
    outputs: &mut [&mut Input],
    library: &dyn Library,
) -> Result<BTreeMap<String, Node>, String> {
    expand_in(nodes, outputs, library, &mut Vec::new())
}

fn expand_in(
    nodes: &BTreeMap<String, Node>,
    outputs: &mut [&mut Input],
    library: &dyn Library,
    calling: &mut Vec<String>,
) -> Result<BTreeMap<String, Node>, String> {
    // Each call's outputs, by the call's name.
    let mut calls: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut out: BTreeMap<String, Node> = BTreeMap::new();
    for (call, node) in nodes {
        let Node::Subgraph { name, inputs } = node else {
            out.insert(call.clone(), node.clone());
            continue;
        };
        if calling.contains(name) {
            let mut round = calling.clone();
            round.push(name.clone());
            return Err(format!(
                "the subgraphs call each other round in a circle: {}",
                round.join(" → ")
            ));
        }
        let sub = library
            .subgraph(name)
            .map_err(|e| format!("node `{call}` (Subgraph): {e}"))?;
        for given in inputs.keys() {
            if !sub.inputs.contains_key(given) {
                let hint =
                    scrap_core::spelling::closest(given, sub.inputs.keys().map(String::as_str))
                        .map(|n| format!(" — did you mean `{n}`?"))
                        .unwrap_or_default();
                return Err(format!(
                    "node `{call}` (Subgraph): `{name}` has no input `{given}`{hint}"
                ));
            }
        }
        if sub.outputs.is_empty() {
            return Err(format!("node `{call}` (Subgraph): `{name}` has no outputs"));
        }
        // Its own calls first, inside it.
        let mut sub_outputs: Vec<(String, Input)> = sub.outputs.clone().into_iter().collect();
        calling.push(name.clone());
        let inner = {
            let mut refs: Vec<&mut Input> = sub_outputs.iter_mut().map(|(_, i)| i).collect();
            expand_in(&sub.nodes, &mut refs, library, calling)?
        };
        calling.pop();
        let prefix = format!("{call}__");
        // A read inside it: of its node, of its input, or of a built-in.
        let rename = |input: &mut Input| {
            if let Input::Name(s) = input {
                let (base, swizzle) = expr::split(s);
                if inner.contains_key(base) || sub.inputs.contains_key(base) {
                    *s = match swizzle {
                        Some(sw) => format!("{prefix}{base}.{sw}"),
                        None => format!("{prefix}{base}"),
                    };
                }
            }
        };
        for (k, mut n) in inner.clone() {
            for (_, i) in n.inputs_mut() {
                rename(i);
            }
            out.insert(format!("{prefix}{k}"), n);
        }
        // Each input a node: what the call gives, or the default — read in
        // the graph it is put in, not inside it.
        for (k, default) in &sub.inputs {
            let given = inputs.get(k).cloned().unwrap_or_else(|| default.clone());
            out.insert(format!("{prefix}{k}"), same(given));
        }
        let mut names = Vec::new();
        for (o, mut source) in sub_outputs {
            rename(&mut source);
            out.insert(output_node(call, &o), same(source));
            names.push(o);
        }
        calls.insert(call.clone(), names);
    }
    // Reads of `call.output` — and of `call` when it has one output.
    let reroute = |input: &mut Input| -> Result<(), String> {
        let Input::Name(s) = input else {
            return Ok(());
        };
        let mut parts = s.splitn(3, '.');
        let base = parts.next().unwrap_or_default();
        let Some(outs) = calls.get(base) else {
            return Ok(());
        };
        let second = parts.next();
        let rest = parts.next();
        let (output, swizzle) = match second {
            Some(o) if outs.iter().any(|x| x == o) => (o.to_string(), rest.map(str::to_string)),
            _ if outs.len() == 1 => (
                outs[0].clone(),
                second.map(|sw| match rest {
                    Some(r) => format!("{sw}.{r}"),
                    None => sw.to_string(),
                }),
            ),
            _ => {
                return Err(format!(
                    "`{s}`: subgraph call `{base}` gives {} — say which, `{base}.{}`",
                    outs.join(", "),
                    outs[0]
                ))
            }
        };
        let node = output_node(base, &output);
        *s = match swizzle {
            Some(sw) => format!("{node}.{sw}"),
            None => node,
        };
        Ok(())
    };
    for node in out.values_mut() {
        for (_, i) in node.inputs_mut() {
            reroute(i)?;
        }
    }
    for i in outputs.iter_mut() {
        reroute(i)?;
    }
    Ok(out)
}

/// A node that is its input as it is: a number stays a number.
fn same(of: Input) -> Node {
    Node::Add {
        a: of,
        b: Input::Number(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Two;
    impl Library for Two {
        fn subgraph(&self, name: &str) -> Result<SubGraph, String> {
            match name {
                "ripple" => parse(
                    r#"(
                        inputs: { "at": "uv", "rings": 8.0 },
                        nodes: { "d": Distance(a: "at", b: (0.5, 0.5)), "r": Multiply(a: "d", b: "rings"), "s": Sine(of: "r") },
                        outputs: { "wave": "s", "far": "d" },
                    )"#,
                ),
                "twice" => parse(
                    r#"(
                        inputs: { "x": 1.0 },
                        nodes: { "inner": Subgraph(name: "ripple", inputs: { "rings": "x" }) },
                        outputs: { "out": "inner.wave" },
                    )"#,
                ),
                "loop" => {
                    parse(r#"(nodes: { "me": Subgraph(name: "loop") }, outputs: { "o": "me" })"#)
                }
                _ => Err(format!("no subgraph `{name}`")),
            }
        }
    }

    fn nodes(text: &str) -> BTreeMap<String, Node> {
        expr::from_ron(text).unwrap()
    }

    #[test]
    fn a_call_is_put_in_under_its_name_and_its_outputs_are_read_through_it() {
        let n = nodes(
            r#"{ "rip": Subgraph(name: "ripple", inputs: { "rings": 4.0 }), "use": Multiply(a: "rip.wave", b: "rip.far.x") }"#,
        );
        let mut albedo = Input::from("rip.wave");
        let out = expand(&n, &mut [&mut albedo], &Two).unwrap();
        assert!(
            out.contains_key("rip__d")
                && out.contains_key("rip__rings")
                && out.contains_key("rip__out__wave")
        );
        assert_eq!(albedo, Input::from("rip__out__wave"));
        assert!(
            matches!(&out["use"], Node::Multiply { a, b } if *a == Input::from("rip__out__wave") && *b == Input::from("rip__out__far.x"))
        );
        assert!(matches!(&out["rip__d"], Node::Distance { a, .. } if *a == Input::from("rip__at")));
        assert!(
            matches!(&out["rip__at"], Node::Add { a, .. } if *a == Input::from("uv")),
            "the default, a built-in outside"
        );
    }

    #[test]
    fn subgraphs_call_subgraphs_but_not_round_in_a_circle_and_mistakes_are_named() {
        let n = nodes(r#"{ "t": Subgraph(name: "twice", inputs: { "x": 2.0 }) }"#);
        let mut o = Input::from("t");
        let out = expand(&n, &mut [&mut o], &Two).unwrap();
        assert!(
            out.contains_key("t__inner__s"),
            "{:?}",
            out.keys().collect::<Vec<_>>()
        );
        assert_eq!(o, Input::from("t__out__out"), "one output: the call itself");
        let e = expand(&nodes(r#"{ "l": Subgraph(name: "loop") }"#), &mut [], &Two).unwrap_err();
        assert!(e.contains("circle") && e.contains("loop → loop"), "{e}");
        let e = expand(
            &nodes(r#"{ "r": Subgraph(name: "ripple", inputs: { "ring": 1.0 }) }"#),
            &mut [],
            &Two,
        )
        .unwrap_err();
        assert!(
            e.contains("no input `ring`") && e.contains("did you mean `rings`"),
            "{e}"
        );
        let mut o = Input::from("r");
        let e = expand(
            &nodes(r#"{ "r": Subgraph(name: "ripple") }"#),
            &mut [&mut o],
            &Two,
        )
        .unwrap_err();
        assert!(e.contains("say which"), "{e}");
    }
}
