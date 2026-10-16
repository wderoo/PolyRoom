//! PolyRoom engine: owns the session state and the command registry.
//!
//! Every user-visible action is a [`Command`] dispatched by id. The egui front-end, the CLI and
//! future automation layers all go through [`CommandRegistry::run`], so they behave the same.

#![forbid(unsafe_code)]

mod command;
mod commands;
mod error;
mod folders;
mod library;
mod session;

pub use command::{Command, CommandRegistry, Key, Shortcut};
pub use error::EngineError;
pub use folders::FolderNode;
pub use polyroom_catalog::{Catalog, ColorLabel, Flag, Model, ModelId};
pub use polyroom_formats::Format;
pub use polyroom_mesh::{Mesh, MeshStats};
pub use polyroom_render::Image;
pub use session::{Collection, Loupe, Module, Panels, Paths, Request, Session, ViewMode};

/// Builds a registry with all built-in commands.
pub fn default_registry() -> CommandRegistry {
    let mut registry = CommandRegistry::default();
    commands::register_all(&mut registry);
    registry
}
