//! Finding entities by what they are: the hierarchy's search box.
//!
//! ```text
//! tree                 name contains "tree", any case
//! "tree near"          a phrase
//! c:door               has the game's `door` component
//! m:stone              drawn with the material named `stone`
//! p:campfire           an instance of the `campfire` prefab
//! model:pine_large     draws that model
//! body:dynamic         moved by physics (static, dynamic, kinematic, trigger, none)
//! ```
//!
//! Terms combine with "and": `c:door m:bark` is every bark door. The same
//! words in the editor, over MCP and in a game's debug console, because an
//! agent asked "which doors are still locked?" should not have to walk the
//! tree itself. Unity's `t:` filters by class; there are no classes here,
//! only the components a line carries, so `c:` is the one that matters.

use crate::id::EntityId;
use crate::scene::{Body, EntityDesc, MaterialRef, Scene};

/// A parsed search.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    terms: Vec<Term>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Term {
    Name(String),
    Component(String),
    Material(String),
    Prefab(String),
    Model(String),
    Body(Body),
}

const PREFIXES: &str = "c: (component), m: (material), p: (prefab), model:, body:";

impl std::str::FromStr for Query {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let mut terms = Vec::new();
        for word in words(text)? {
            let term = match word.split_once(':') {
                None => Term::Name(word.to_lowercase()),
                Some((prefix, value)) => {
                    if value.is_empty() {
                        return Err(format!("`{word}` says what to look for but not which"));
                    }
                    match prefix.to_lowercase().as_str() {
                        "c" | "component" => Term::Component(value.to_lowercase()),
                        "m" | "material" => Term::Material(value.to_string()),
                        "p" | "prefab" => Term::Prefab(value.to_string()),
                        "model" => Term::Model(value.to_string()),
                        "body" => Term::Body(match value.to_lowercase().as_str() {
                            "none" => Body::None,
                            "static" => Body::Static,
                            "dynamic" => Body::Dynamic,
                            "kinematic" => Body::Kinematic,
                            "trigger" => Body::Trigger,
                            other => {
                                return Err(format!(
                                    "no body `{other}`: body:static, body:dynamic, body:kinematic, body:trigger or body:none"
                                ))
                            }
                        }),
                        other => {
                            return Err(format!(
                                "no filter `{other}:` — there are {PREFIXES}, and a plain word matches names"
                            ))
                        }
                    }
                }
            };
            terms.push(term);
        }
        Ok(Self { terms })
    }
}

impl Query {
    /// Whether one entity matches, given the prefab its line names (empty
    /// for anything that is not an instance).
    pub fn matches(&self, desc: &EntityDesc, prefab: &str) -> bool {
        self.terms.iter().all(|term| match term {
            Term::Name(part) => desc.name.to_lowercase().contains(part),
            Term::Component(name) => desc.components.keys().any(|k| k.to_lowercase() == *name),
            Term::Material(name) => {
                matches!(&desc.material, MaterialRef::Named(n) if n == name)
            }
            Term::Prefab(name) => prefab == name,
            Term::Model(name) => desc.model == *name,
            Term::Body(body) => desc.body == *body,
        })
    }

    /// Every match in `expanded`, depth first — prefab parts included, so
    /// `ember` finds the ember inside every campfire. `document` is the
    /// scene before expansion, which is where an instance's prefab is
    /// written; without prefabs the two are the same scene.
    pub fn search(&self, document: &Scene, expanded: &Scene) -> Vec<EntityId> {
        let mut out = Vec::new();
        let mut stack: Vec<&EntityDesc> = expanded.entities.iter().rev().collect();
        while let Some(desc) = stack.pop() {
            let prefab = document
                .get(desc.id)
                .map(|line| line.prefab.as_str())
                .unwrap_or("");
            if self.matches(desc, prefab) {
                out.push(desc.id);
            }
            stack.extend(desc.children.iter().rev());
        }
        out
    }
}

/// Words, with "quoted phrases" kept whole.
fn words(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if quoted {
        return Err(format!("`{text}` opens a quote it does not close"));
    }
    if !current.is_empty() {
        out.push(current);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Scene {
        let mut scene: Scene = ron::from_str(
            r#"(entities: [
                (name: "tree near", model: "pine_large", material: "needle", body: Static),
                (name: "Tree far", model: "pine_large", material: "needle"),
                (name: "front door", model: "builtin:cube", material: "bark",
                 components: { "door": (locked: true) }),
                (name: "back door", model: "builtin:cube", material: "stone",
                 components: { "Door": (locked: false) }),
                (name: "west fire", prefab: "campfire", children: [
                    (name: "ember", model: "builtin:sphere", material: "ember"),
                ]),
            ])"#,
        )
        .unwrap();
        scene.assign_ids();
        scene
    }

    fn names(scene: &Scene, query: &str) -> Vec<String> {
        let query: Query = query.parse().unwrap();
        query
            .search(scene, scene)
            .into_iter()
            .map(|id| scene.get(id).unwrap().name.clone())
            .collect()
    }

    #[test]
    fn words_and_filters_find_what_they_say() {
        let scene = scene();
        assert_eq!(names(&scene, "tree"), ["tree near", "Tree far"]);
        assert_eq!(names(&scene, "\"tree near\""), ["tree near"]);
        assert_eq!(names(&scene, "c:door"), ["front door", "back door"]);
        assert_eq!(names(&scene, "c:door m:bark"), ["front door"]);
        assert_eq!(names(&scene, "p:campfire"), ["west fire"]);
        assert_eq!(names(&scene, "model:pine_large body:static"), ["tree near"]);
        assert_eq!(names(&scene, "ember"), ["ember"], "children too");
        assert_eq!(names(&scene, "").len(), 6, "nothing asked is everything");
        assert!(names(&scene, "m:granite").is_empty());
    }

    #[test]
    fn a_query_that_cannot_mean_anything_says_why() {
        let e = "t:Door".parse::<Query>().unwrap_err();
        assert!(
            e.contains("no filter `t:`") && e.contains("c: (component)"),
            "{e}"
        );
        let e = "body:floaty".parse::<Query>().unwrap_err();
        assert!(e.contains("body:static"), "{e}");
        let e = "\"tree".parse::<Query>().unwrap_err();
        assert!(e.contains("quote"), "{e}");
        let e = "m:".parse::<Query>().unwrap_err();
        assert!(e.contains("not which"), "{e}");
    }
}
