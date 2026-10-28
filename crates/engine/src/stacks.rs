//! Stacks: preview images grouped onto the model (or ZIP) they show, like Lightroom's stacks.
//!
//! Images and models are grouped per folder. A `files`/`images` style subfolder counts as part
//! of its parent, so the common download layout `Thing/files/x.stl` + `Thing/images/x.jpg`
//! stacks too. Within a group an image goes to the model whose name it starts with (the longest
//! such name wins); images that match nothing go to the only model of the group, if there is
//! exactly one, and otherwise stay on their own.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use polyroom_catalog::{Model, ModelId};
use polyroom_formats::Format;

/// Subfolders that belong to their parent's group (lowercase).
const PART_DIRS: &[&str] = &[
    "files",
    "file",
    "stl",
    "stls",
    "models",
    "model",
    "images",
    "image",
    "pictures",
    "photos",
    "renders",
    "img",
    "imgs",
    "thumbnails",
    "preview",
    "previews",
];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stacks {
    owner_of: HashMap<ModelId, ModelId>,
    photos: HashMap<ModelId, Vec<ModelId>>,
}

impl Stacks {
    pub fn build(models: &[Model]) -> Self {
        let mut groups: BTreeMap<PathBuf, Group> = BTreeMap::new();
        for m in models {
            let Some(group) = group_of(&m.path) else {
                continue;
            };
            let entry = groups.entry(group).or_default();
            let stem = stem_of(&m.path);
            match m.format {
                Format::Image => entry.images.push((m.path.as_path(), m.id, stem)),
                // Files without a preview neither own photos nor are one.
                Format::Other => {}
                _ => entry.owners.push((m.id, stem)),
            }
        }
        let mut stacks = Self::default();
        for mut g in groups.into_values() {
            g.images.sort_by(|a, b| a.0.cmp(b.0));
            g.owners.retain(|(_, stem)| !stem.is_empty());
            // (owner, exact match) for each image, in path order.
            let mut matched: Vec<(ModelId, ModelId, bool)> = Vec::new();
            let mut leftovers = Vec::new();
            for (_, image, stem) in &g.images {
                let best = g
                    .owners
                    .iter()
                    .filter(|(_, owner)| starts_with_word(stem, owner))
                    .max_by_key(|(_, owner)| owner.len());
                match best {
                    Some((owner, owner_stem)) => {
                        matched.push((*owner, *image, owner_stem == stem));
                    }
                    None => leftovers.push(*image),
                }
            }
            if let [(only, _)] = g.owners.as_slice() {
                matched.extend(leftovers.into_iter().map(|image| (*only, image, false)));
            }
            // Exact-name images first: they make the best cover.
            matched.sort_by_key(|(_, _, exact)| !exact);
            for (owner, image, _) in matched {
                stacks.owner_of.insert(image, owner);
                stacks.photos.entry(owner).or_default().push(image);
            }
        }
        stacks
    }

    /// The model an image is stacked on.
    pub fn owner(&self, image: ModelId) -> Option<ModelId> {
        self.owner_of.get(&image).copied()
    }

    /// The images stacked on `owner`, cover first.
    pub fn photos(&self, owner: ModelId) -> &[ModelId] {
        self.photos.get(&owner).map_or(&[], Vec::as_slice)
    }
}

#[derive(Default)]
struct Group<'a> {
    /// (id, normalized stem) of models and archives.
    owners: Vec<(ModelId, String)>,
    images: Vec<(&'a Path, ModelId, String)>,
}

/// The folder whose group `path` belongs to.
fn group_of(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    let is_part = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| PART_DIRS.contains(&n.to_lowercase().as_str()));
    match dir.parent() {
        Some(parent) if is_part => Some(parent.to_owned()),
        _ => Some(dir.to_owned()),
    }
}

/// The file name without extension, lowercased, with separators folded to single spaces.
/// A model extension left in a ZIP's name (`part.3mf.zip`) is dropped too.
fn stem_of(path: &Path) -> String {
    let mut stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(inner) = Path::new(&stem)
        .file_stem()
        .filter(|_| Format::from_path(Path::new(&stem)).is_some())
    {
        stem = inner.to_string_lossy().into_owned();
    }
    stem.to_lowercase()
        .split(|c: char| c.is_whitespace() || matches!(c, '_' | '-' | '+' | '.'))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `name` is `prefix`, or starts with it followed by something other than a letter or
/// digit (`dragon (2)` and `dragon preview` start with `dragon`; `dragonfly` doesn't).
fn starts_with_word(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use polyroom_catalog::{Catalog, ScannedFile};

    use super::*;

    fn catalog(files: &[&str]) -> Catalog {
        let mut c = Catalog::in_memory();
        let scanned = files
            .iter()
            .map(|f| {
                let path = Path::new("C:/3D").join(f);
                ScannedFile {
                    format: Format::from_path(&path).unwrap(),
                    path,
                    size: 1,
                    modified: None,
                }
            })
            .collect();
        c.import(Path::new("C:/3D"), scanned).unwrap();
        c
    }

    /// Each stack as owner name → photo names, cover first.
    fn stacks(files: &[&str]) -> Vec<(String, Vec<String>)> {
        let c = catalog(files);
        let s = Stacks::build(c.models());
        let name = |id: ModelId| c.get(id).unwrap().name();
        let mut out: Vec<_> = c
            .models()
            .iter()
            .filter(|m| !s.photos(m.id).is_empty())
            .map(|m| (m.name(), s.photos(m.id).iter().map(|&i| name(i)).collect()))
            .collect();
        out.sort();
        out
    }

    fn stack(owner: &str, photos: &[&str]) -> (String, Vec<String>) {
        (
            owner.to_owned(),
            photos.iter().map(|p| (*p).to_owned()).collect(),
        )
    }

    #[test]
    fn images_named_like_a_model_stack_on_it() {
        assert_eq!(
            stacks(&[
                "Dragon.stl",
                "dragon (2).jpg",
                "Dragon.png",
                "dragon_preview.webp",
                "dragonfly.jpg",
                "boat.3mf",
            ]),
            [stack(
                "Dragon.stl",
                &["Dragon.png", "dragon (2).jpg", "dragon_preview.webp"]
            )],
            "dragonfly.jpg is not a picture of the dragon; boat stays alone"
        );
    }

    #[test]
    fn the_longest_matching_model_name_wins() {
        assert_eq!(
            stacks(&[
                "2x2.stl",
                "2x2_CommandStrip.stl",
                "2x2 CommandStrip.jpg",
                "2x2.jpg"
            ]),
            [
                stack("2x2.stl", &["2x2.jpg"]),
                stack("2x2_CommandStrip.stl", &["2x2 CommandStrip.jpg"]),
            ]
        );
    }

    #[test]
    fn files_and_images_subfolders_form_one_group() {
        assert_eq!(
            stacks(&[
                "Wolf - 4834209/files/Wolf_Buff.stl",
                "Wolf - 4834209/images/Muscle-Wolf-1.jpg",
                "Wolf - 4834209/images/Wolf_Buff.png",
            ]),
            [stack(
                "Wolf_Buff.stl",
                &["Wolf_Buff.png", "Muscle-Wolf-1.jpg"]
            )]
        );
    }

    #[test]
    fn unmatched_images_stay_alone_unless_the_group_has_one_model() {
        assert_eq!(
            stacks(&["a.stl", "b.stl", "cover.jpg", "lonely/photo.jpg"]),
            [] as [(String, Vec<String>); 0]
        );
    }

    #[test]
    fn images_stack_on_archives_too() {
        assert_eq!(
            stacks(&["Bleed block.3MF.zip", "bleed_block.jpg", "other.stl"]),
            [stack("Bleed block.3MF.zip", &["bleed_block.jpg"])]
        );
    }
}
