//! Categories, tags and favorites: the organising data attached to models and folders.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ModelId;

pub type CategoryId = u32;

/// Longest tag kept, in characters.
const MAX_TAG_CHARS: usize = 40;

/// A user-defined group such as "Miniatures". "Uncategorized" is the absence of one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: CategoryId,
    pub name: String,
    /// RGB colour of the category dot and pill.
    pub color: [u8; 3],
}

/// What an organising edit applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Model(ModelId),
    Folder(PathBuf),
}

/// Organising data of one model or folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub category: Option<CategoryId>,
    #[serde(default)]
    pub tags: BTreeSet<String>,
    #[serde(default)]
    pub favorite: bool,
}

impl Meta {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Trims and lowercases a tag; `None` if nothing is left.
pub fn clean_tag(tag: &str) -> Option<String> {
    let tag: String = tag
        .trim()
        .to_lowercase()
        .chars()
        .take(MAX_TAG_CHARS)
        .collect();
    let tag = tag.trim().to_owned();
    (!tag.is_empty()).then_some(tag)
}

/// The categories a new catalog starts with.
pub(crate) fn default_categories() -> Vec<Category> {
    [
        ("Miniatures", [0x4c, 0xaf, 0x50]),
        ("Functional", [0xff, 0xa7, 0x26]),
        ("Art", [0xef, 0x53, 0x50]),
        ("Terrain", [0x8d, 0x6e, 0x63]),
    ]
    .into_iter()
    .zip(1..)
    .map(|((name, color), id)| Category {
        id,
        name: name.to_owned(),
        color,
    })
    .collect()
}
