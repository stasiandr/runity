//! The editor's actions, registered once (DNA, "Модули: манифест, поезд,
//! редактор": a module's action is seen by the menu and by the agent).
//!
//! An action is a name — what the agent calls as an MCP tool — a label and
//! a menu, a shortcut, a sentence for the agent, the arguments it takes,
//! and what it does to the [`Session`]. The studio's menus put it where
//! its `menu` says under its label and key; `scrap-mcp` lists it as a tool
//! with a schema made from its arguments. Without its entities named, an
//! action works on the selection: what a menu item means, and what an
//! agent that selected first can ask for too.
//!
//! What a module brings to the editor is actions here, by the module's
//! name ([`EditorAction::module`]); the list is [`registry`].

use std::collections::BTreeMap;

use scrap::EntityId;

use crate::Session;

/// What an argument is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One entity, by its id.
    Id,
    /// Entities, by their ids.
    Ids,
    Text,
    Flag,
}

/// One argument an action takes.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    /// For the agent: what it is, and what leaving it out means.
    pub about: &'static str,
}

/// An argument's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Id(EntityId),
    Ids(Vec<EntityId>),
    Text(String),
    Flag(bool),
}

/// The arguments an action was given, by name; a menu gives none.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Args(pub BTreeMap<String, Arg>);

impl Args {
    pub fn with(mut self, name: &str, arg: Arg) -> Self {
        self.0.insert(name.to_string(), arg);
        self
    }

    pub fn id(&self, name: &str) -> Option<EntityId> {
        match self.0.get(name) {
            Some(Arg::Id(id)) => Some(*id),
            _ => None,
        }
    }

    /// The ids given, `None` when the argument was left out (an empty list
    /// is a list).
    pub fn ids(&self, name: &str) -> Option<&[EntityId]> {
        match self.0.get(name) {
            Some(Arg::Ids(ids)) => Some(ids),
            _ => None,
        }
    }

    pub fn text(&self, name: &str) -> Option<&str> {
        match self.0.get(name) {
            Some(Arg::Text(text)) => Some(text),
            _ => None,
        }
    }

    pub fn flag(&self, name: &str) -> Option<bool> {
        match self.0.get(name) {
            Some(Arg::Flag(on)) => Some(*on),
            _ => None,
        }
    }
}

/// What an action says it did, or why it could not.
pub type Done = Result<String, String>;

/// One action of the editor.
#[derive(Clone, Copy)]
pub struct EditorAction {
    /// The agent's name for it, and the menu's: `undo`.
    pub name: &'static str,
    /// The module that brings it; `editor` for the bare editor's own.
    pub module: &'static str,
    /// Its menu in the menu bar: `Edit`.
    pub menu: &'static str,
    pub label: &'static str,
    /// Its key on macOS, and elsewhere.
    pub shortcut: Option<(&'static str, &'static str)>,
    /// For the agent: what it does.
    pub about: &'static str,
    pub params: &'static [Param],
    pub run: fn(&mut Session, &Args) -> Done,
}

impl std::fmt::Debug for EditorAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EditorAction({})", self.name)
    }
}

impl EditorAction {
    /// The key, as this platform writes it.
    pub fn key(&self) -> Option<&'static str> {
        self.shortcut
            .map(|(mac, other)| if cfg!(target_os = "macos") { mac } else { other })
    }
}

/// Every action the editor has: the bare editor's, then each module's.
pub fn registry() -> Vec<EditorAction> {
    EDITOR.to_vec()
}

/// The action called `name`.
pub fn find(name: &str) -> Option<EditorAction> {
    registry().into_iter().find(|a| a.name == name)
}

/// Run the action called `name`.
pub fn run(session: &mut Session, name: &str, args: &Args) -> Done {
    let action = find(name).ok_or_else(|| format!("no action `{name}`"))?;
    (action.run)(session, args)
}

const ID: &str = "an entity id: 16 hex digits, as scene_tree shows; the selection when left out";
const IDS: &str = "entity ids; the selection when left out";

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Put the given entities in the selection, when there are any.
fn select(session: &mut Session, ids: &[EntityId]) -> Result<(), String> {
    for (i, id) in ids.iter().enumerate() {
        if i == 0 {
            session.select(Some(*id)).map_err(err)?;
        } else {
            session.add_to_selection(*id).map_err(err)?;
        }
    }
    Ok(())
}

const EDITOR: &[EditorAction] = &[
    EditorAction {
        name: "save_scene",
        module: "editor",
        menu: "File",
        label: "Save",
        shortcut: Some(("⌘S", "Ctrl+S")),
        about: "Write the scene to its file, or to path.",
        params: &[Param { name: "path", kind: Kind::Text, about: "where to write it; its own file when left out" }],
        run: |s, args| {
            let path = args.text("path").map(std::path::PathBuf::from);
            s.save_scene(path.as_deref()).map_err(err)?;
            Ok("saved".into())
        },
    },
    EditorAction {
        name: "undo",
        module: "editor",
        menu: "Edit",
        label: "Undo",
        shortcut: Some(("⌘Z", "Ctrl+Z")),
        about: "Take back the last edit; says what it was (\"move `crate`\").",
        params: &[],
        run: |s, _| {
            let what = s.undo_label();
            Ok(match (s.undo().map_err(err)?, what) {
                (true, Some(what)) => format!("undone: {what}"),
                (true, None) => "undone".into(),
                (false, _) => "nothing to undo".into(),
            })
        },
    },
    EditorAction {
        name: "redo",
        module: "editor",
        menu: "Edit",
        label: "Redo",
        shortcut: Some(("⇧⌘Z", "Ctrl+Y")),
        about: "Put back the last edit taken back.",
        params: &[],
        run: |s, _| {
            let what = s.redo_label();
            Ok(match (s.redo().map_err(err)?, what) {
                (true, Some(what)) => format!("redone: {what}"),
                (true, None) => "redone".into(),
                (false, _) => "nothing to redo".into(),
            })
        },
    },
    EditorAction {
        name: "duplicate_entity",
        module: "editor",
        menu: "Edit",
        label: "Duplicate",
        shortcut: Some(("⌘D", "Ctrl+D")),
        about: "Copy an entity with its children, as its next sibling. Returns the copy's id.",
        params: &[Param { name: "id", kind: Kind::Id, about: ID }],
        run: |s, args| match args.id("id") {
            Some(id) => Ok(s.duplicate(id).map_err(err)?.to_string()),
            None => {
                let copies = s.duplicate_selection().map_err(err)?;
                Ok(copies.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))
            }
        },
    },
    EditorAction {
        name: "delete_entity",
        module: "editor",
        menu: "Edit",
        label: "Delete",
        shortcut: Some(("Delete", "Delete")),
        about: "Delete an entity and everything under it.",
        params: &[Param { name: "id", kind: Kind::Id, about: ID }],
        run: |s, args| match args.id("id") {
            Some(id) => {
                s.delete(id).map_err(err)?;
                Ok(format!("deleted {id}"))
            }
            None => Ok(format!("deleted {}", s.delete_selection().map_err(err)?)),
        },
    },
    EditorAction {
        name: "drop_to_ground",
        module: "editor",
        menu: "Edit",
        label: "Drop to Ground",
        shortcut: Some(("End", "End")),
        about: "Put entities down on whatever is beneath them — the real shape of it: a slope, a terrain — as one undo step.",
        params: &[Param { name: "ids", kind: Kind::Ids, about: IDS }],
        run: |s, args| {
            if let Some(ids) = args.ids("ids") {
                select(s, ids)?;
            }
            let count = s.selection().len();
            let landed = s.drop_to_ground().map_err(err)?;
            Ok(format!("{landed} of {count} found ground"))
        },
    },
    EditorAction {
        name: "isolate",
        module: "editor",
        menu: "View",
        label: "Isolate Selection",
        shortcut: Some(("⇧H", "Shift+H")),
        about: "Show only these entities (and what is under them) in `render`; an empty list shows everything again, hidden ones too. A view setting, like `hide`.",
        params: &[Param { name: "ids", kind: Kind::Ids, about: IDS }],
        run: |s, args| {
            let ids = match args.ids("ids") {
                Some(ids) => ids.to_vec(),
                None => s.selection(),
            };
            if ids.is_empty() {
                s.show_all();
                return Ok("everything is shown".into());
            }
            s.isolate(&ids).map_err(err)?;
            Ok(format!("showing {} alone", ids.len()))
        },
    },
];

#[cfg(test)]
mod tests {
    #[test]
    fn every_action_has_one_name_and_a_menu() {
        let all = super::registry();
        for (i, a) in all.iter().enumerate() {
            assert!(!a.menu.is_empty() && !a.label.is_empty() && !a.about.is_empty(), "{a:?}");
            assert!(all[i + 1..].iter().all(|b| b.name != a.name), "{} twice", a.name);
        }
        assert!(super::find("undo").is_some());
    }
}
