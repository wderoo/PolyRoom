//! PolyRoom engine: owns the session state and the command registry.
//!
//! Every user-visible action is a [`Command`] dispatched by id. The egui front-end, the CLI and
//! future automation layers all go through [`CommandRegistry::run`], so they behave the same.

#![forbid(unsafe_code)]

mod attributes;
mod command;
mod commands;
mod error;
mod folders;
mod library;
mod organize;
mod progress;
mod session;
mod settings;
mod sort;
mod stacks;

pub use attributes::AttributeFilter;
pub use command::{Command, CommandRegistry, Key, Shortcut};
pub use error::EngineError;
pub use folders::FolderNode;
pub use polyroom_catalog::{
    Catalog, Category, CategoryId, ColorLabel, Flag, Meta, Model, ModelId, Target,
};
pub use polyroom_formats::Format;
pub use polyroom_formats::archive::ArchiveEntry;
pub use polyroom_mesh::{Mesh, MeshStats};
pub use polyroom_render::Image;
pub use progress::Progress;
pub use session::{Collection, LibraryStats, Loupe, Panels, Paths, Request, Session};
pub use settings::{PreviewSettings, THUMB_SIZES};
pub use sort::{Sort, SortDir, SortKey};

/// Builds a registry with all built-in commands.
pub fn default_registry() -> CommandRegistry {
    let mut registry = CommandRegistry::default();
    commands::register_all(&mut registry);
    registry
}
