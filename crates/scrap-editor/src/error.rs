//! What can go wrong in a session, as values rather than a string on the
//! side.
//!
//! The C boundary this replaces returned `false` and left a sentence in a
//! thread-local for whoever thought to ask. In Rust the reason travels with
//! the failure, and the sentence is still the point: an agent that tried
//! something and got an error has to be able to tell from the text alone
//! what it did wrong (DNA, postulate 5).

use std::fmt;

use scrap::EntityId;

/// Why a session refused or failed to do something.
#[derive(Debug, Clone, PartialEq)]
pub enum EditError {
    /// An edit while the scene is being simulated. Refused rather than
    /// allowed and thrown away on stop, which is the version people lose an
    /// hour to.
    Playing,
    /// No entity with this ID in the document.
    NoEntity(EntityId),
    /// A name that has to be something was empty.
    EmptyName(&'static str),
    /// Saving with no path given and no file the scene came from.
    NoPath,
    /// An operation that needs a library before one was set.
    NoLibrary,
    /// An operation that writes into a project — a prefab, a material — on
    /// a scene that is not in one.
    NotInProject,
    /// Placing an instance of a prefab nobody has.
    UnknownPrefab(String),
    /// No adapter to render with.
    Gpu(String),
    /// The file system said no.
    Io(String),
    /// A scene file that would not read or write.
    Scene(String),
    /// The importer could not turn a source into an asset.
    Import(String),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::Playing => {
                f.write_str("stop playing first — an edit made in play mode is an edit you lose")
            }
            EditError::NoEntity(id) => write!(f, "no entity with id {id} in the scene"),
            EditError::EmptyName(what) => write!(f, "{what} needs a name"),
            EditError::NoPath => f.write_str("no path to save to, and the scene has no file yet"),
            EditError::NoLibrary => f.write_str("no library — call set_library first"),
            EditError::NotInProject => f.write_str(
                "this scene is not in a scrap project — prefabs/ and materials/ belong to a \
                 project, the folder with scrap.ron (scrap::Project::create makes one)",
            ),
            EditError::UnknownPrefab(name) => write!(f, "no prefab named {name}"),
            EditError::Gpu(e) => write!(f, "no GPU to render with: {e}"),
            EditError::Io(e) => write!(f, "{e}"),
            EditError::Scene(e) => write!(f, "scene: {e}"),
            EditError::Import(e) => write!(f, "import: {e}"),
        }
    }
}

impl std::error::Error for EditError {}

impl From<std::io::Error> for EditError {
    fn from(e: std::io::Error) -> Self {
        EditError::Io(e.to_string())
    }
}
