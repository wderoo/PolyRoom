//! The model library.
//!
//! Persistence follows lightcraft's idea: a JSON snapshot plus an append-only log of
//! operations (`ops.jsonl`). Every change is one appended line, so a crash loses at most the
//! line being written; corrupt lines are skipped on load. The log is folded into the snapshot
//! when it grows large.

#![forbid(unsafe_code)]

mod scan;

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use polyroom_formats::Format;
use polyroom_mesh::MeshStats;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
}

#[derive(Debug, Default)]
pub struct Catalog {
    dir: Option<PathBuf>,
    log: Option<File>,
    ops_in_log: usize,
    folders: Vec<PathBuf>,
    models: Vec<Model>,
    index: HashMap<ModelId, usize>,
    next_id: ModelId,
    last_import: u32,
    /// Bumped on every change; lets views cache derived data.
    version: u64,
    /// Bumped only when models or folders are added, removed or replaced, not on edits like
    /// ratings or stats; lets views cache lists and trees.
    layout_version: u64,
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
            Op::AddFolder { .. } | Op::AddModel { .. } | Op::UpdateFile { .. } | Op::Remove { .. }
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

    pub fn version(&self) -> u64 {
        self.version
    }

    /// Like [`Catalog::version`], but unchanged by edits that don't alter which models exist.
    pub fn layout_version(&self) -> u64 {
        self.layout_version
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
            .filter(|f| !known.contains(f.path.as_path()))
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
}
