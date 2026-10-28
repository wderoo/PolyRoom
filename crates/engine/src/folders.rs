//! The folder tree under each imported folder, derived from the models in the catalog.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use polyroom_catalog::{Model, ModelId};
use polyroom_formats::Format;

use crate::stacks::Stacks;

/// Thumbnails shown on a folder's grid cell.
const SAMPLES: usize = 4;

/// One folder that contains library files, directly or in a subfolder.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderNode {
    pub path: PathBuf,
    pub name: String,
    pub children: Vec<FolderNode>,
    /// Files directly in this folder.
    pub direct: usize,
    /// Files in this folder and all subfolders.
    pub total: usize,
    /// How many of `total` are images, and how many ZIP archives.
    pub images: usize,
    pub archives: usize,
    /// Size in bytes of all files in this folder and its subfolders.
    pub bytes: u64,
    /// A few files from this folder or below for the folder's preview; images and models with
    /// photos come first. Photos stacked on a model are left out (the model shows them).
    pub samples: Vec<ModelId>,
}

impl FolderNode {
    /// Finds the node for `path` in this subtree.
    pub fn find(&self, path: &Path) -> Option<&FolderNode> {
        if self.path == path {
            return Some(self);
        }
        if !path.starts_with(&self.path) {
            return None;
        }
        self.children.iter().find_map(|c| c.find(path))
    }
}

#[derive(Default)]
struct Counts {
    direct: usize,
    total: usize,
    images: usize,
    archives: usize,
    bytes: u64,
    /// Sample candidates with a picture, and without one.
    pictured: Vec<ModelId>,
    plain: Vec<ModelId>,
}

/// Builds one tree per imported root, in the roots' import order.
pub fn build(roots: &[PathBuf], models: &[Model], stacks: &Stacks) -> Vec<FolderNode> {
    roots
        .iter()
        .map(|root| {
            let mut counts: BTreeMap<PathBuf, Counts> = BTreeMap::new();
            counts.entry(root.clone()).or_default();
            for m in models.iter().filter(|m| &m.folder == root) {
                let Some(parent) = m.path.parent() else {
                    continue;
                };
                counts.entry(parent.to_owned()).or_default().direct += 1;
                // Count the model in every folder from its own up to the root.
                for dir in parent.ancestors() {
                    if !dir.starts_with(root) {
                        break;
                    }
                    let c = counts.entry(dir.to_owned()).or_default();
                    c.total += 1;
                    c.bytes = c.bytes.saturating_add(m.size);
                    match m.format {
                        Format::Image => c.images += 1,
                        Format::Zip => c.archives += 1,
                        _ => {}
                    }
                    // Previews only show files that load.
                    if m.error.is_some() || stacks.owner(m.id).is_some() {
                        continue;
                    }
                    let pictured = m.format == Format::Image || !stacks.photos(m.id).is_empty();
                    let list = if pictured {
                        &mut c.pictured
                    } else {
                        &mut c.plain
                    };
                    if list.len() < SAMPLES {
                        list.push(m.id);
                    }
                }
            }
            node(root, &mut counts)
        })
        .collect()
}

fn node(path: &Path, counts: &mut BTreeMap<PathBuf, Counts>) -> FolderNode {
    let mut c = counts.remove(path).unwrap_or_default();
    c.pictured.extend(c.plain);
    c.pictured.truncate(SAMPLES);
    let mut child_paths: Vec<PathBuf> = counts
        .keys()
        .filter(|p| p.parent() == Some(path))
        .cloned()
        .collect();
    child_paths.sort_by_key(|p| name_of(p).to_lowercase());
    let children = child_paths.iter().map(|p| node(p, counts)).collect();
    FolderNode {
        path: path.to_owned(),
        name: name_of(path),
        children,
        direct: c.direct,
        total: c.total,
        images: c.images,
        archives: c.archives,
        bytes: c.bytes,
        samples: c.pictured,
    }
}

/// How many folders the trees hold, roots included.
pub fn count(nodes: &[FolderNode]) -> usize {
    nodes.iter().map(|n| 1 + count(&n.children)).sum()
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use polyroom_catalog::{Catalog, ScannedFile};
    use polyroom_formats::Format;

    use super::*;

    fn catalog(files: &[&str]) -> Catalog {
        let mut c = Catalog::in_memory();
        let scanned = files
            .iter()
            .map(|f| ScannedFile {
                path: Path::new("C:/3D").join(f),
                format: Format::Stl,
                size: 1,
                modified: None,
            })
            .collect();
        c.import(Path::new("C:/3D"), scanned).unwrap();
        c
    }

    #[test]
    fn tree_counts_direct_and_nested_models() {
        let c = catalog(&[
            "a.stl",
            "Halloween/ghost.stl",
            "Halloween/skeleton/arm.stl",
            "Halloween/skeleton/leg.stl",
            "cars/bmw/body.stl",
        ]);
        let trees = build(c.folders(), c.models(), &Stacks::default());
        let root = &trees[0];
        assert_eq!((root.direct, root.total), (1, 5));
        let names: Vec<_> = root.children.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["cars", "Halloween"]); // case-insensitive order
        let halloween = root.find(Path::new("C:/3D/Halloween")).unwrap();
        assert_eq!((halloween.direct, halloween.total), (1, 3));
        let skeleton = root.find(Path::new("C:/3D/Halloween/skeleton")).unwrap();
        assert_eq!(
            (skeleton.direct, skeleton.total, skeleton.samples.len()),
            (2, 2, 2)
        );
        // "cars" holds no models itself but appears because a subfolder does.
        let cars = root.find(Path::new("C:/3D/cars")).unwrap();
        assert_eq!((cars.direct, cars.total), (0, 1));
        assert!(root.find(Path::new("C:/elsewhere")).is_none());
    }

    #[test]
    fn folder_previews_prefer_pictures_and_count_kinds() {
        let mut c = Catalog::in_memory();
        let names = ["a.stl", "b.stl", "c.zip", "wolf.stl", "wolf.png", "z.jpg"];
        let scanned = names
            .iter()
            .map(|f| {
                let path = Path::new("C:/3D/sub").join(f);
                ScannedFile {
                    format: Format::from_path(&path).unwrap(),
                    path,
                    size: 1,
                    modified: None,
                }
            })
            .collect();
        c.import(Path::new("C:/3D"), scanned).unwrap();
        let stacks = Stacks::build(c.models());
        let trees = build(c.folders(), c.models(), &stacks);
        let sub = trees[0].find(Path::new("C:/3D/sub")).unwrap();
        assert_eq!((sub.total, sub.images, sub.archives), (6, 2, 1));
        let sample_names: Vec<_> = sub
            .samples
            .iter()
            .map(|&id| c.get(id).unwrap().name())
            .collect();
        // wolf.png is shown through wolf.stl, so it isn't a sample of its own.
        assert_eq!(sample_names, ["wolf.stl", "z.jpg", "a.stl", "b.stl"]);
    }
}
