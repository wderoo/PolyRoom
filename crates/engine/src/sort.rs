//! Sort order of the grid.

use std::cmp::Ordering;

use polyroom_catalog::{Catalog, Model, ModelId};
use serde::{Deserialize, Serialize};

use crate::folders::FolderNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    #[default]
    Name,
    DateAdded,
    Modified,
    Size,
    Rating,
    Format,
}

impl SortKey {
    pub const ALL: [Self; 6] = [
        Self::Name,
        Self::DateAdded,
        Self::Modified,
        Self::Size,
        Self::Rating,
        Self::Format,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::DateAdded => "Date added",
            Self::Modified => "Modified",
            Self::Size => "Size",
            Self::Rating => "Rating",
            Self::Format => "Type",
        }
    }

    /// The id used in the `sort.by` command.
    pub fn id(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::DateAdded => "date_added",
            Self::Modified => "modified",
            Self::Size => "size",
            Self::Rating => "rating",
            Self::Format => "format",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }

    /// Names and types read best A to Z; dates, sizes and ratings biggest or newest first.
    fn natural_direction(self) -> SortDir {
        match self {
            Self::Name | Self::Format => SortDir::Ascending,
            _ => SortDir::Descending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDir {
    #[default]
    Ascending,
    Descending,
}

impl SortDir {
    pub fn flipped(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }

    fn apply(self, ordering: Ordering) -> Ordering {
        match self {
            Self::Ascending => ordering,
            Self::Descending => ordering.reverse(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sort {
    pub key: SortKey,
    pub dir: SortDir,
}

impl Sort {
    /// Choosing the active key again flips the direction; a new key starts in its natural one.
    pub fn choose(&mut self, key: SortKey) {
        if self.key == key {
            self.dir = self.dir.flipped();
        } else {
            *self = Self {
                key,
                dir: key.natural_direction(),
            };
        }
    }

    /// Orders models by the key, with the name and then the id breaking ties.
    pub fn sort_models(&self, ids: &mut [ModelId], catalog: &Catalog) {
        let name = |m: &Model| m.name().to_lowercase();
        let mut rows: Vec<(&Model, String)> = ids
            .iter()
            .filter_map(|&id| catalog.get(id))
            .map(|m| (m, name(m)))
            .collect();
        rows.sort_by(|(a, an), (b, bn)| {
            let by_key = match self.key {
                SortKey::Name => an.cmp(bn),
                SortKey::DateAdded => a.added.cmp(&b.added),
                SortKey::Modified => a.modified.cmp(&b.modified),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Rating => a.rating.cmp(&b.rating),
                SortKey::Format => a.format.label().cmp(b.format.label()),
            };
            self.dir
                .apply(by_key)
                .then_with(|| an.cmp(bn))
                .then_with(|| a.id.cmp(&b.id))
        });
        for (slot, (m, _)) in ids.iter_mut().zip(rows) {
            *slot = m.id;
        }
    }

    /// Folders sort by size when asked to, and by name otherwise.
    pub fn sort_folders(&self, folders: &mut [&FolderNode]) {
        folders.sort_by(|a, b| {
            let by_name = a.name.to_lowercase().cmp(&b.name.to_lowercase());
            match self.key {
                SortKey::Name => self.dir.apply(by_name),
                SortKey::Size => self.dir.apply(a.bytes.cmp(&b.bytes)).then(by_name),
                _ => by_name,
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use polyroom_catalog::ScannedFile;

    use super::*;

    fn catalog() -> Catalog {
        let mut c = Catalog::in_memory();
        let file = |name: &str, size: u64, modified: u64| ScannedFile {
            path: PathBuf::from("C:/m").join(name),
            format: polyroom_formats::Format::Stl,
            size,
            modified: Some(modified),
        };
        c.import(
            std::path::Path::new("C:/m"),
            vec![
                file("b.stl", 10, 300),
                file("A.stl", 30, 100),
                file("c.stl", 20, 200),
            ],
        )
        .unwrap();
        c
    }

    fn order(c: &Catalog, sort: Sort) -> Vec<String> {
        let mut ids: Vec<ModelId> = c.models().iter().map(|m| m.id).collect();
        sort.sort_models(&mut ids, c);
        ids.iter()
            .filter_map(|&i| c.get(i))
            .map(Model::name)
            .collect()
    }

    #[test]
    fn each_key_orders_the_models() {
        let c = catalog();
        let by = |key| {
            let mut s = Sort {
                key: SortKey::Format,
                dir: SortDir::Ascending,
            };
            s.choose(key);
            order(&c, s)
        };
        assert_eq!(by(SortKey::Name), ["A.stl", "b.stl", "c.stl"]);
        assert_eq!(
            by(SortKey::Size),
            ["A.stl", "c.stl", "b.stl"],
            "biggest first"
        );
        assert_eq!(
            by(SortKey::Modified),
            ["b.stl", "c.stl", "A.stl"],
            "newest first"
        );
        // Same added time and rating: ties fall back to the name.
        assert_eq!(by(SortKey::Rating), ["A.stl", "b.stl", "c.stl"]);
    }

    #[test]
    fn choosing_the_active_key_flips_the_direction() {
        let c = catalog();
        let mut s = Sort::default();
        s.choose(SortKey::Name);
        assert_eq!(s.dir, SortDir::Descending);
        assert_eq!(order(&c, s), ["c.stl", "b.stl", "A.stl"]);
        s.choose(SortKey::Size);
        assert_eq!(
            s.dir,
            SortDir::Descending,
            "a new key starts in its natural direction"
        );
    }

    #[test]
    fn keys_round_trip_through_their_ids() {
        for k in SortKey::ALL {
            assert_eq!(SortKey::from_id(k.id()), Some(k));
        }
        assert_eq!(SortKey::from_id("nope"), None);
    }
}
