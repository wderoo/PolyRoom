//! The model library.
//!
//! Persistence follows lightcraft's idea: a JSON snapshot plus an append-only log of
//! operations (`ops.jsonl`). Every change is one appended line, so a crash loses at most the
//! line being written; corrupt lines are skipped on load. The log is folded into the snapshot
//! when it grows large.

#![forbid(unsafe_code)]

mod meta;
mod scan;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use polyroom_formats::Format;
use polyroom_mesh::MeshStats;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use meta::{Category, CategoryId, Meta, Target, clean_tag};
pub use scan::{ScannedFile, scan_folder};

pub type ModelId = u64;

const SNAPSHOT: &str = "snapshot.json";
const LOG: &str = "ops.jsonl";
/// Fold the log into the snapshot once it holds this many operations.
const COMPACT_AFTER: usize = 2000;

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("catalog I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not encode catalog data: {0}")]
    Encode(#[from] serde_json::Error),
}

fn yes() -> bool {
    true
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> CatalogError + '_ {
    move |source| CatalogError::Io {
        path: path.to_owned(),
        source,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Flag {
    #[default]
    None,
    Pick,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorLabel {
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ColorLabel {
    pub const ALL: [Self; 5] = [
        Self::Red,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
        }
    }
}

/// A file in the library: a 3D model, a preview image or a ZIP archive (see `format`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: ModelId,
    pub path: PathBuf,
    /// The imported folder this model was found in.
    pub folder: PathBuf,
    pub format: Format,
    /// File size in bytes.
    pub size: u64,
    /// Modification time, seconds since the Unix epoch.
    pub modified: Option<u64>,
    /// Import time, seconds since the Unix epoch.
    pub added: u64,
    /// Import batch; the newest batch is "Previous Import".
    pub import: u32,
    #[serde(default)]
    pub rating: u8,
    #[serde(default)]
    pub flag: Flag,
    #[serde(default)]
    pub label: Option<ColorLabel>,
    #[serde(default)]
    pub stats: Option<MeshStats>,
    #[serde(default)]
    pub meta: Meta,
    /// Why the file could not be loaded, if it couldn't.
    #[serde(default)]
    pub error: Option<String>,
}

impl Model {
    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }
}

/// One change to the catalog. Ops carry explicit ids so replaying the log is deterministic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    AddFolder {
        path: PathBuf,
    },
    AddModel {
        model: Box<Model>,
    },
    SetRating {
        id: ModelId,
        rating: u8,
    },
    SetFlag {
        id: ModelId,
        flag: Flag,
    },
    SetLabel {
        id: ModelId,
        label: Option<ColorLabel>,
    },
    SetStats {
        id: ModelId,
        stats: MeshStats,
    },
    SetError {
        id: ModelId,
        error: Option<String>,
    },
    /// The file changed on disk: new size/date, and stats must be recomputed.
    UpdateFile {
        id: ModelId,
        size: u64,
        modified: Option<u64>,
    },
    Remove {
        id: ModelId,
    },
    /// Adds the default categories once per catalog.
    SeedCategories,
    AddCategory {
        category: Category,
    },
    RenameCategory {
        id: CategoryId,
        name: String,
    },
    /// Also clears the category from everything that used it.
    RemoveCategory {
        id: CategoryId,
    },
    SetCategory {
        target: Target,
        category: Option<CategoryId>,
    },
    AddTag {
        target: Target,
        tag: String,
    },
    RemoveTag {
        target: Target,
        tag: String,
    },
    SetFavorite {
        target: Target,
        favorite: bool,
    },
    /// Drops a folder and its models from the library. A subfolder stays excluded, so
    /// refreshing its parent does not bring it back. Files on disk are untouched.
    RemoveFolder {
        path: PathBuf,
        /// Keep a removed subfolder out of later scans. A folder deleted from disk needs no
        /// exclusion.
        #[serde(default = "yes")]
        exclude: bool,
    },
}

/// What [`Catalog::sync`] changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub added: usize,
    pub removed: Vec<ModelId>,
    /// Models whose file changed; their thumbnails are stale.
    pub updated: Vec<ModelId>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Snapshot {
    folders: Vec<PathBuf>,
    models: Vec<Model>,
    #[serde(default)]
    categories: Vec<Category>,
    #[serde(default)]
    seeded: bool,
    #[serde(default)]
    folder_meta: Vec<(PathBuf, Meta)>,
    #[serde(default)]
    excluded: Vec<PathBuf>,
}

#[derive(Debug, Default)]
pub struct Catalog {
    dir: Option<PathBuf>,
    log: Option<File>,
    ops_in_log: usize,
    folders: Vec<PathBuf>,
    models: Vec<Model>,
    categories: Vec<Category>,
    seeded: bool,
    folder_meta: HashMap<PathBuf, Meta>,
    /// Subfolders removed from the library; scans skip them.
    excluded: Vec<PathBuf>,
    index: HashMap<ModelId, usize>,
    next_id: ModelId,
    last_import: u32,
    /// Bumped on every change; lets views cache derived data.
    version: u64,
    /// Bumped only when models or folders are added, removed or replaced, not on edits like
    /// ratings or stats; lets views cache lists and trees.
    layout_version: u64,
    /// Bumped by edits of categories, tags and favorites.
    meta_version: u64,
}

impl Catalog {
    /// A catalog that is never written to disk (tests, CLI).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Opens (or creates) the catalog stored in `dir`.
    pub fn open(dir: &Path) -> Result<Self, CatalogError> {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
        let mut catalog = Self::default();

        let snapshot_path = dir.join(SNAPSHOT);
        match std::fs::read(&snapshot_path) {
            Ok(bytes) => match serde_json::from_slice::<Snapshot>(&bytes) {
                Ok(s) => {
                    catalog.folders = s.folders;
                    catalog.categories = s.categories;
                    catalog.seeded = s.seeded;
                    catalog.folder_meta = s.folder_meta.into_iter().collect();
                    catalog.excluded = s.excluded;
                    for m in s.models {
                        catalog.insert(m);
                    }
                }
                Err(e) => {
                    // Keep the bad file for inspection rather than overwriting it.
                    log::error!("catalog snapshot is corrupt ({e}); starting from the op log");
                    let _ = std::fs::rename(&snapshot_path, dir.join("snapshot.corrupt.json"));
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(&snapshot_path)(e)),
        }

        let log_path = dir.join(LOG);
        if let Ok(file) = File::open(&log_path) {
            for (n, line) in BufReader::new(file).lines().enumerate() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Op>(&line) {
                    Ok(op) => {
                        catalog.apply_in_memory(op);
                        catalog.ops_in_log += 1;
                    }
                    Err(e) => log::warn!("skipping bad catalog op on line {}: {e}", n + 1),
                }
            }
        }

        catalog.dir = Some(dir.to_owned());
        if catalog.ops_in_log >= COMPACT_AFTER {
            catalog.compact()?;
        }
        catalog.open_log()?;
        if !catalog.seeded {
            catalog.apply(Op::SeedCategories)?;
        }
        Ok(catalog)
    }

    fn open_log(&mut self) -> Result<(), CatalogError> {
        if let Some(dir) = &self.dir {
            let path = dir.join(LOG);
            self.log = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .map_err(io_err(&path))?,
            );
        }
        Ok(())
    }

    /// Writes a fresh snapshot (atomically, via a temp file) and empties the log.
    pub fn compact(&mut self) -> Result<(), CatalogError> {
        let Some(dir) = self.dir.clone() else {
            return Ok(());
        };
        let snapshot = Snapshot {
            folders: self.folders.clone(),
            models: self.models.clone(),
            categories: self.categories.clone(),
            seeded: self.seeded,
            folder_meta: self
                .folder_meta
                .iter()
                .map(|(p, m)| (p.clone(), m.clone()))
                .collect(),
            excluded: self.excluded.clone(),
        };
        let tmp = dir.join("snapshot.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&snapshot)?).map_err(io_err(&tmp))?;
        let final_path = dir.join(SNAPSHOT);
        std::fs::rename(&tmp, &final_path).map_err(io_err(&final_path))?;
        self.log = None;
        let log_path = dir.join(LOG);
        File::create(&log_path).map_err(io_err(&log_path))?;
        self.ops_in_log = 0;
        self.open_log()
    }

    /// Applies an op and appends it to the log. The in-memory change happens even if writing
    /// fails, so the UI stays consistent; the error tells the caller the change isn't saved.
    pub fn apply(&mut self, op: Op) -> Result<(), CatalogError> {
        let line = serde_json::to_string(&op)?;
        self.apply_in_memory(op);
        if let Some(log) = &mut self.log {
            let path = self.dir.as_deref().unwrap_or(Path::new(LOG));
            writeln!(log, "{line}").map_err(io_err(path))?;
            self.ops_in_log += 1;
        }
        if self.ops_in_log >= COMPACT_AFTER {
            self.compact()?;
        }
        Ok(())
    }

    /// Like [`Catalog::apply`] for many ops, with a single write to the log.
    pub fn apply_all(&mut self, ops: Vec<Op>) -> Result<(), CatalogError> {
        let mut text = String::new();
        for op in &ops {
            text.push_str(&serde_json::to_string(op)?);
            text.push('\n');
        }
        let count = ops.len();
        for op in ops {
            self.apply_in_memory(op);
        }
        if let Some(log) = &mut self.log {
            let path = self.dir.as_deref().unwrap_or(Path::new(LOG));
            log.write_all(text.as_bytes()).map_err(io_err(path))?;
            self.ops_in_log += count;
        }
        if self.ops_in_log >= COMPACT_AFTER {
            self.compact()?;
        }
        Ok(())
    }

    fn insert(&mut self, model: Model) {
        self.next_id = self.next_id.max(model.id + 1);
        self.last_import = self.last_import.max(model.import);
        if let Some(&i) = self.index.get(&model.id) {
            if let Some(slot) = self.models.get_mut(i) {
                *slot = model;
            }
        } else {
            self.index.insert(model.id, self.models.len());
            self.models.push(model);
        }
    }

    fn apply_in_memory(&mut self, op: Op) {
        self.version += 1;
        if matches!(
            op,
            Op::SeedCategories
                | Op::AddCategory { .. }
                | Op::RenameCategory { .. }
                | Op::RemoveCategory { .. }
                | Op::SetCategory { .. }
                | Op::AddTag { .. }
                | Op::RemoveTag { .. }
                | Op::SetFavorite { .. }
        ) {
            self.meta_version += 1;
        }
        if matches!(
            op,
            Op::AddFolder { .. }
                | Op::AddModel { .. }
                | Op::UpdateFile { .. }
                | Op::Remove { .. }
                | Op::RemoveFolder { .. }
        ) {
            self.layout_version += 1;
        }
        match op {
            Op::AddFolder { path } => {
                if !self.folders.contains(&path) {
                    self.folders.push(path);
                }
            }
            Op::AddModel { model } => self.insert(*model),
            Op::SetRating { id, rating } => {
                if let Some(m) = self.get_mut(id) {
                    m.rating = rating.min(5);
                }
            }
            Op::SetFlag { id, flag } => {
                if let Some(m) = self.get_mut(id) {
                    m.flag = flag;
                }
            }
            Op::SetLabel { id, label } => {
                if let Some(m) = self.get_mut(id) {
                    m.label = label;
                }
            }
            Op::SetStats { id, stats } => {
                if let Some(m) = self.get_mut(id) {
                    m.stats = Some(stats);
                    m.error = None;
                }
            }
            Op::SetError { id, error } => {
                if let Some(m) = self.get_mut(id) {
                    m.error = error;
                }
            }
            Op::UpdateFile { id, size, modified } => {
                if let Some(m) = self.get_mut(id) {
                    m.size = size;
                    m.modified = modified;
                    m.stats = None;
                    m.error = None;
                }
            }
            Op::Remove { id } => {
                if let Some(i) = self.index.remove(&id) {
                    self.models.remove(i);
                    self.reindex();
                }
            }
            Op::SeedCategories => {
                if !self.seeded {
                    self.seeded = true;
                    let next = self.next_category_id();
                    for (offset, mut c) in (0..).zip(meta::default_categories()) {
                        c.id = next + offset;
                        self.categories.push(c);
                    }
                }
            }
            Op::AddCategory { category } => {
                self.seeded = true;
                if !self.categories.iter().any(|c| c.id == category.id) {
                    self.categories.push(category);
                }
            }
            Op::RenameCategory { id, name } => {
                if let Some(c) = self.categories.iter_mut().find(|c| c.id == id) {
                    c.name = name;
                }
            }
            Op::RemoveCategory { id } => {
                self.categories.retain(|c| c.id != id);
                for m in &mut self.models {
                    if m.meta.category == Some(id) {
                        m.meta.category = None;
                    }
                }
                for meta in self.folder_meta.values_mut() {
                    if meta.category == Some(id) {
                        meta.category = None;
                    }
                }
                self.folder_meta.retain(|_, m| !m.is_empty());
            }
            Op::SetCategory { target, category } => {
                if category.is_none_or(|id| self.categories.iter().any(|c| c.id == id)) {
                    self.edit_meta(&target, |m| m.category = category);
                }
            }
            Op::AddTag { target, tag } => {
                if let Some(tag) = clean_tag(&tag) {
                    self.edit_meta(&target, |m| {
                        m.tags.insert(tag);
                    });
                }
            }
            Op::RemoveTag { target, tag } => {
                self.edit_meta(&target, |m| {
                    m.tags.remove(&tag);
                });
            }
            Op::SetFavorite { target, favorite } => {
                self.edit_meta(&target, |m| m.favorite = favorite);
            }
            Op::RemoveFolder { path, exclude } => {
                self.folders.retain(|f| *f != path);
                self.models.retain(|m| !m.path.starts_with(&path));
                self.reindex();
                self.folder_meta.retain(|p, _| !p.starts_with(&path));
                self.excluded.retain(|e| !e.starts_with(&path));
                // A removed root is simply gone; a removed subfolder must stay out of scans.
                let inside_root = self.folders.iter().any(|f| path.starts_with(f));
                if exclude && inside_root && !self.excluded.contains(&path) {
                    self.excluded.push(path);
                }
            }
        }
    }

    fn next_category_id(&self) -> CategoryId {
        self.categories
            .iter()
            .map(|c| c.id)
            .max()
            .map_or(1, |m| m.saturating_add(1))
    }

    /// Edits the organising data of `target`; a folder entry that ends up empty is dropped.
    fn edit_meta(&mut self, target: &Target, edit: impl FnOnce(&mut Meta)) {
        match target {
            Target::Model(id) => {
                if let Some(m) = self.get_mut(*id) {
                    edit(&mut m.meta);
                }
            }
            Target::Folder(path) => {
                let meta = self.folder_meta.entry(path.clone()).or_default();
                edit(meta);
                if meta.is_empty() {
                    self.folder_meta.remove(path);
                }
            }
        }
    }

    fn reindex(&mut self) {
        self.index = self
            .models
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
    }

    fn get_mut(&mut self, id: ModelId) -> Option<&mut Model> {
        let i = *self.index.get(&id)?;
        self.models.get_mut(i)
    }

    pub fn get(&self, id: ModelId) -> Option<&Model> {
        self.models.get(*self.index.get(&id)?)
    }

    pub fn models(&self) -> &[Model] {
        &self.models
    }

    pub fn folders(&self) -> &[PathBuf] {
        &self.folders
    }

    pub fn categories(&self) -> &[Category] {
        &self.categories
    }

    /// A free id for a new category.
    pub fn new_category_id(&self) -> CategoryId {
        self.next_category_id()
    }

    pub fn folder_meta(&self, path: &Path) -> Option<&Meta> {
        self.folder_meta.get(path)
    }

    /// The organising data of a model or folder (empty if it has none).
    pub fn meta_of(&self, target: &Target) -> Meta {
        match target {
            Target::Model(id) => self.get(*id).map(|m| m.meta.clone()).unwrap_or_default(),
            Target::Folder(p) => self.folder_meta.get(p).cloned().unwrap_or_default(),
        }
    }

    /// The category of a model, or else of the nearest folder above it that has one.
    pub fn effective_category(&self, model: &Model) -> Option<CategoryId> {
        model
            .meta
            .category
            .or_else(|| self.folder_category(&model.path))
    }

    /// The category of the nearest folder above `path` that has one.
    pub fn folder_category(&self, path: &Path) -> Option<CategoryId> {
        path.ancestors()
            .skip(1)
            .find_map(|a| self.folder_meta.get(a)?.category)
    }

    /// Whether a model, or a folder above it, is a favorite.
    pub fn effective_favorite(&self, model: &Model) -> bool {
        model.meta.favorite
            || model
                .path
                .ancestors()
                .skip(1)
                .any(|a| self.folder_meta.get(a).is_some_and(|m| m.favorite))
    }

    /// A model's own tags plus those of the folders above it.
    pub fn effective_tags(&self, model: &Model) -> BTreeSet<String> {
        let mut tags = model.meta.tags.clone();
        for a in model.path.ancestors().skip(1) {
            if let Some(m) = self.folder_meta.get(a) {
                tags.extend(m.tags.iter().cloned());
            }
        }
        tags
    }

    /// Every tag in use, with the number of models and folders carrying it.
    pub fn all_tags(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        let metas = self
            .models
            .iter()
            .map(|m| &m.meta)
            .chain(self.folder_meta.values());
        for meta in metas {
            for t in &meta.tags {
                *out.entry(t.clone()).or_insert(0) += 1;
            }
        }
        out
    }

    pub fn is_excluded(&self, path: &Path) -> bool {
        self.excluded.iter().any(|e| path.starts_with(e))
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// Like [`Catalog::version`], but unchanged by edits that don't alter which models exist.
    pub fn layout_version(&self) -> u64 {
        self.layout_version
    }

    /// Like [`Catalog::version`], but only changed by categories, tags and favorites.
    pub fn meta_version(&self) -> u64 {
        self.meta_version
    }

    /// The import batch shown as "Previous Import" (0 if nothing was imported yet).
    pub fn last_import(&self) -> u32 {
        self.last_import
    }

    /// Adds scanned files from `folder` as a new import batch, skipping paths already in the
    /// catalog. Returns how many models were added.
    pub fn import(
        &mut self,
        folder: &Path,
        files: Vec<ScannedFile>,
    ) -> Result<usize, CatalogError> {
        let known: std::collections::HashSet<&Path> =
            self.models.iter().map(|m| m.path.as_path()).collect();
        let new: Vec<ScannedFile> = files
            .into_iter()
            .filter(|f| !known.contains(f.path.as_path()) && !self.is_excluded(&f.path))
            .collect();
        if !self.folders.iter().any(|f| f == folder) {
            self.apply(Op::AddFolder {
                path: folder.to_owned(),
            })?;
        }
        if new.is_empty() {
            return Ok(0);
        }
        let batch = self.last_import + 1;
        let now = now_secs();
        let count = new.len();
        for f in new {
            let model = Model {
                id: self.next_id,
                path: f.path,
                folder: folder.to_owned(),
                format: f.format,
                size: f.size,
                modified: f.modified,
                added: now,
                import: batch,
                rating: 0,
                flag: Flag::None,
                label: None,
                stats: None,
                meta: Meta::default(),
                error: None,
            };
            self.apply(Op::AddModel {
                model: Box::new(model),
            })?;
        }
        Ok(count)
    }

    /// Brings the models under `scope` in line with `files`, a fresh scan of `scope`. `scope`
    /// is the imported folder `root` or a folder inside it. Ratings, flags and labels of
    /// unchanged and changed files are kept; deleted files are dropped; new files are added.
    pub fn sync(
        &mut self,
        root: &Path,
        scope: &Path,
        files: Vec<ScannedFile>,
    ) -> Result<SyncReport, CatalogError> {
        let on_disk: HashMap<&Path, &ScannedFile> =
            files.iter().map(|f| (f.path.as_path(), f)).collect();
        let mut report = SyncReport::default();
        let mut updates = Vec::new();
        for m in self.models.iter().filter(|m| m.path.starts_with(scope)) {
            match on_disk.get(m.path.as_path()) {
                None => report.removed.push(m.id),
                Some(f) if f.size != m.size || f.modified != m.modified => {
                    updates.push(Op::UpdateFile {
                        id: m.id,
                        size: f.size,
                        modified: f.modified,
                    });
                    report.updated.push(m.id);
                }
                Some(_) => {}
            }
        }
        for &id in &report.removed {
            self.apply(Op::Remove { id })?;
        }
        for op in updates {
            self.apply(op)?;
        }
        // `import` skips files that are already in the catalog.
        report.added = self.import(root, files)?;
        Ok(report)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> ScannedFile {
        ScannedFile {
            path: PathBuf::from("C:/models").join(name),
            format: Format::Stl,
            size: 1234,
            modified: Some(1),
        }
    }

    #[test]
    fn import_skips_known_paths_and_batches() {
        let mut c = Catalog::in_memory();
        let folder = Path::new("C:/models");
        assert_eq!(
            c.import(folder, vec![file("a.stl"), file("b.stl")])
                .unwrap(),
            2
        );
        assert_eq!(
            c.import(folder, vec![file("a.stl"), file("c.stl")])
                .unwrap(),
            1
        );
        assert_eq!(c.models().len(), 3);
        assert_eq!(c.folders().len(), 1);
        assert_eq!(c.last_import(), 2);
        let ids: Vec<_> = c.models().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn edits_survive_reopen_via_log_and_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut c = Catalog::open(dir.path()).unwrap();
            c.import(Path::new("C:/models"), vec![file("a.stl"), file("b.stl")])
                .unwrap();
            c.apply(Op::SetRating { id: 1, rating: 4 }).unwrap();
            c.apply(Op::SetFlag {
                id: 0,
                flag: Flag::Pick,
            })
            .unwrap();
        }
        let mut c = Catalog::open(dir.path()).unwrap();
        assert_eq!(c.get(1).unwrap().rating, 4);
        assert_eq!(c.get(0).unwrap().flag, Flag::Pick);

        c.compact().unwrap();
        c.apply(Op::SetLabel {
            id: 1,
            label: Some(ColorLabel::Green),
        })
        .unwrap();
        drop(c);
        let c = Catalog::open(dir.path()).unwrap();
        assert_eq!(c.get(1).unwrap().rating, 4);
        assert_eq!(c.get(1).unwrap().label, Some(ColorLabel::Green));
        // New ids continue after reopening.
        assert_eq!(c.next_id, 2);
    }

    #[test]
    fn corrupt_log_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut c = Catalog::open(dir.path()).unwrap();
            c.import(Path::new("C:/models"), vec![file("a.stl")])
                .unwrap();
        }
        let mut log = OpenOptions::new()
            .append(true)
            .open(dir.path().join(LOG))
            .unwrap();
        writeln!(log, "{{garbage").unwrap();
        writeln!(log, r#"{{"op":"set_rating","id":0,"rating":3}}"#).unwrap();
        write!(log, r#"{{"op":"set_fl"#).unwrap(); // torn final write
        drop(log);
        let c = Catalog::open(dir.path()).unwrap();
        assert_eq!(c.get(0).unwrap().rating, 3);
    }

    #[test]
    fn batched_ops_persist_and_edits_leave_the_layout_version_alone() {
        let dir = tempfile::tempdir().unwrap();
        let stats = polyroom_mesh::test_cube().stats().unwrap();
        let layout = {
            let mut c = Catalog::open(dir.path()).unwrap();
            c.import(Path::new("C:/models"), vec![file("a.stl"), file("b.stl")])
                .unwrap();
            let (layout, version) = (c.layout_version(), c.version());
            c.apply_all(vec![
                Op::SetStats { id: 0, stats },
                Op::SetStats { id: 1, stats },
                Op::SetRating { id: 1, rating: 2 },
            ])
            .unwrap();
            assert_eq!(
                c.layout_version(),
                layout,
                "stats and ratings keep the layout"
            );
            assert_eq!(c.version(), version + 3);
            layout
        };
        let c = Catalog::open(dir.path()).unwrap();
        assert!(c.get(0).unwrap().stats.is_some() && c.get(1).unwrap().stats.is_some());
        assert_eq!(c.get(1).unwrap().rating, 2);
        let mut c = c;
        c.apply(Op::Remove { id: 0 }).unwrap();
        assert!(c.layout_version() > layout);
    }

    #[test]
    fn corrupt_snapshot_does_not_prevent_opening() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SNAPSHOT), b"not json").unwrap();
        let c = Catalog::open(dir.path()).unwrap();
        assert!(c.models().is_empty());
        assert!(dir.path().join("snapshot.corrupt.json").exists());
    }

    #[test]
    fn sync_adds_removes_and_updates_within_scope_only() {
        let mut c = Catalog::in_memory();
        let root = Path::new("C:/models");
        let in_sub = |n: &str| ScannedFile {
            path: root.join("sub").join(n),
            ..file(n)
        };
        c.import(
            root,
            vec![file("top.stl"), in_sub("a.stl"), in_sub("b.stl")],
        )
        .unwrap();
        c.apply(Op::SetRating { id: 1, rating: 5 }).unwrap();
        c.apply(Op::SetStats {
            id: 1,
            stats: polyroom_mesh::test_cube().stats().unwrap(),
        })
        .unwrap();

        // On disk now: a.stl changed, b.stl deleted, c.stl new. top.stl is outside the scope.
        let changed = ScannedFile {
            size: 999,
            ..in_sub("a.stl")
        };
        let report = c
            .sync(root, &root.join("sub"), vec![changed, in_sub("c.stl")])
            .unwrap();
        assert_eq!(report.added, 1);
        assert_eq!(report.removed, vec![2]);
        assert_eq!(report.updated, vec![1]);

        let a = c.get(1).unwrap();
        assert_eq!((a.size, a.rating, a.stats), (999, 5, None)); // rating kept, stats stale
        assert!(c.get(2).is_none());
        assert!(c.get(0).is_some(), "files outside the scope are untouched");
        assert_eq!(c.models().len(), 3);

        // Nothing changed: an empty report.
        let again = vec![
            ScannedFile {
                size: 999,
                ..in_sub("a.stl")
            },
            in_sub("c.stl"),
        ];
        assert_eq!(
            c.sync(root, &root.join("sub"), again).unwrap(),
            SyncReport::default()
        );
    }

    #[test]
    fn rating_is_clamped() {
        let mut c = Catalog::in_memory();
        c.import(Path::new("C:/m"), vec![file("a.stl")]).unwrap();
        c.apply(Op::SetRating { id: 0, rating: 200 }).unwrap();
        assert_eq!(c.get(0).unwrap().rating, 5);
    }

    fn cat(id: CategoryId, name: &str) -> Category {
        Category {
            id,
            name: name.to_owned(),
            color: [1, 2, 3],
        }
    }

    #[test]
    fn fresh_catalogs_get_default_categories_once() {
        let dir = tempfile::tempdir().unwrap();
        let names = |c: &Catalog| {
            c.categories()
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
        };
        {
            let mut c = Catalog::open(dir.path()).unwrap();
            assert_eq!(names(&c), ["Miniatures", "Functional", "Art", "Terrain"]);
            let id = c.categories()[0].id;
            c.apply(Op::RemoveCategory { id }).unwrap();
        }
        let c = Catalog::open(dir.path()).unwrap();
        assert_eq!(names(&c), ["Functional", "Art", "Terrain"], "not reseeded");
    }

    #[test]
    fn organising_edits_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let folder = PathBuf::from("C:/models");
        {
            let mut c = Catalog::open(dir.path()).unwrap();
            c.import(&folder, vec![file("a.stl")]).unwrap();
            let art = c.categories()[2].id;
            c.apply(Op::SetCategory {
                target: Target::Model(0),
                category: Some(art),
            })
            .unwrap();
            c.apply(Op::AddTag {
                target: Target::Model(0),
                tag: "  Dragon ".into(),
            })
            .unwrap();
            c.apply(Op::SetFavorite {
                target: Target::Folder(folder.clone()),
                favorite: true,
            })
            .unwrap();
            c.compact().unwrap();
            c.apply(Op::AddTag {
                target: Target::Folder(folder.clone()),
                tag: "Orc".into(),
            })
            .unwrap();
        }
        let c = Catalog::open(dir.path()).unwrap();
        let m = c.get(0).unwrap();
        assert_eq!(m.meta.category, Some(c.categories()[2].id));
        assert_eq!(m.meta.tags.iter().collect::<Vec<_>>(), ["dragon"]);
        let f = c.folder_meta(&folder).unwrap();
        assert!(f.favorite);
        assert!(f.tags.contains("orc"));
        let tags = c.all_tags();
        assert_eq!(tags.get("dragon"), Some(&1));
        assert_eq!(
            c.effective_tags(m).into_iter().collect::<Vec<_>>(),
            ["dragon", "orc"]
        );
    }

    #[test]
    fn models_inherit_the_category_of_their_folder() {
        let mut c = Catalog::in_memory();
        let root = PathBuf::from("C:/models");
        let sub = root.join("sub");
        let nested = ScannedFile {
            path: sub.join("a.stl"),
            ..file("a.stl")
        };
        c.import(&root, vec![nested, file("top.stl")]).unwrap();
        c.apply(Op::AddCategory {
            category: cat(7, "Art"),
        })
        .unwrap();
        c.apply(Op::SetCategory {
            target: Target::Folder(sub),
            category: Some(7),
        })
        .unwrap();
        fn by_name<'a>(c: &'a Catalog, n: &str) -> &'a Model {
            c.models().iter().find(|m| m.name() == n).unwrap()
        }
        assert_eq!(c.effective_category(by_name(&c, "a.stl")), Some(7));
        assert_eq!(c.effective_category(by_name(&c, "top.stl")), None);

        c.apply(Op::SetCategory {
            target: Target::Model(0),
            category: None,
        })
        .unwrap();
        c.apply(Op::RemoveCategory { id: 7 }).unwrap();
        assert_eq!(c.effective_category(by_name(&c, "a.stl")), None);
        assert!(
            c.folder_meta(&root.join("sub")).is_none(),
            "empty meta dropped"
        );
    }

    #[test]
    fn a_model_in_a_favorite_folder_is_a_favorite() {
        let mut c = Catalog::in_memory();
        let root = PathBuf::from("C:/models");
        let nested = ScannedFile {
            path: root.join("sub").join("a.stl"),
            ..file("a.stl")
        };
        c.import(&root, vec![nested, file("top.stl")]).unwrap();
        c.apply(Op::SetFavorite {
            target: Target::Folder(root.join("sub")),
            favorite: true,
        })
        .unwrap();
        let favorite = |c: &Catalog, name: &str| {
            let m = c.models().iter().find(|m| m.name() == name).unwrap();
            c.effective_favorite(m)
        };
        assert!(favorite(&c, "a.stl"));
        assert!(!favorite(&c, "top.stl"));
    }

    #[test]
    fn unknown_categories_and_blank_tags_are_ignored() {
        let mut c = Catalog::in_memory();
        c.import(Path::new("C:/m"), vec![file("a.stl")]).unwrap();
        c.apply(Op::SetCategory {
            target: Target::Model(0),
            category: Some(99),
        })
        .unwrap();
        c.apply(Op::AddTag {
            target: Target::Model(0),
            tag: "   ".into(),
        })
        .unwrap();
        assert!(c.get(0).unwrap().meta.is_empty());
    }

    #[test]
    fn removing_a_root_drops_its_models_and_metadata() {
        let mut c = Catalog::in_memory();
        let (a, b) = (PathBuf::from("C:/a"), PathBuf::from("C:/b"));
        let in_a = ScannedFile {
            path: a.join("x.stl"),
            ..file("x.stl")
        };
        let in_b = ScannedFile {
            path: b.join("y.stl"),
            ..file("y.stl")
        };
        c.import(&a, vec![in_a]).unwrap();
        c.import(&b, vec![in_b]).unwrap();
        c.apply(Op::SetFavorite {
            target: Target::Folder(a.clone()),
            favorite: true,
        })
        .unwrap();
        let layout = c.layout_version();
        c.apply(Op::RemoveFolder {
            path: a.clone(),
            exclude: true,
        })
        .unwrap();
        assert!(c.layout_version() > layout);
        assert_eq!(c.folders(), [b]);
        assert_eq!(c.models().len(), 1);
        assert!(c.get(1).is_some() && c.get(0).is_none());
        assert!(c.folder_meta(&a).is_none());
        assert!(!c.is_excluded(&a), "a removed root may be imported again");
    }

    #[test]
    fn a_removed_subfolder_stays_out_after_sync_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let root = PathBuf::from("C:/models");
        let sub = root.join("sub");
        let in_sub = ScannedFile {
            path: sub.join("a.stl"),
            ..file("a.stl")
        };
        {
            let mut c = Catalog::open(dir.path()).unwrap();
            c.import(&root, vec![file("top.stl"), in_sub.clone()])
                .unwrap();
            c.apply(Op::RemoveFolder {
                path: sub.clone(),
                exclude: true,
            })
            .unwrap();
            assert_eq!(c.models().len(), 1);
            let report = c
                .sync(&root, &root, vec![file("top.stl"), in_sub.clone()])
                .unwrap();
            assert_eq!(report, SyncReport::default());
        }
        let mut c = Catalog::open(dir.path()).unwrap();
        assert!(c.is_excluded(&sub.join("deeper")));
        c.sync(&root, &root, vec![file("top.stl"), in_sub]).unwrap();
        assert_eq!(c.models().len(), 1);
    }

    #[test]
    fn snapshots_from_before_organising_data_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let old = r#"{"folders":["C:/m"],"models":[{"id":0,"path":"C:/m/a.stl","folder":"C:/m",
            "format":"Stl","size":5,"modified":null,"added":1,"import":1,"rating":3}]}"#;
        std::fs::write(dir.path().join(SNAPSHOT), old).unwrap();
        let c = Catalog::open(dir.path()).unwrap();
        assert_eq!(c.get(0).unwrap().rating, 3);
        assert!(c.get(0).unwrap().meta.is_empty());
        assert_eq!(c.categories().len(), 4);
    }
}
