use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use polyroom_catalog::{Catalog, CatalogError, Model, ModelId, Op, ScannedFile, SyncReport};
use polyroom_formats::Format;
use polyroom_formats::archive::ArchiveEntry;
use polyroom_mesh::Mesh;
use polyroom_preview::{Event, PreviewService};
use polyroom_render::Image;
use serde::{Deserialize, Serialize};

use crate::attributes::AttributeFilter;
use crate::folders::{self, FolderNode};
use crate::progress::{Progress, Tracker};
use crate::settings::{self, Settings};
use crate::stacks::Stacks;

/// Finished thumbnails handed to the UI per frame.
const THUMBNAILS_PER_FRAME: usize = 8;

/// Longest side of an image shown in the loupe, in pixels.
const LOUPE_PICTURE_PX: u32 = 2048;

/// Preview jobs an import keeps queued at once; the rest wait in the engine so that cells the
/// user scrolls to are served first.
const IMPORT_IN_FLIGHT: usize = 16;

/// What the visible list was computed from: the catalog layout, edit counter (only tracked
/// while an attribute filter is active), collection, name filter and attribute filter.
type VisibleKey = (u64, u64, Collection, String, AttributeFilter);

/// How long a changed setting waits before it is written, so dragging a slider saves once.
const SETTINGS_DEBOUNCE: Duration = Duration::from_millis(400);

/// The top-level module, like Lightroom's Library/Develop switcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Module {
    #[default]
    Library,
    Viewer,
}

/// How the centre of the Library module shows models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ViewMode {
    #[default]
    Grid,
    Loupe,
}

/// Visibility of the panels around the centre view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Panels {
    pub left: bool,
    pub right: bool,
    pub filmstrip: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            left: true,
            right: true,
            filmstrip: true,
        }
    }
}

/// Something only the front-end can do; queued by commands and drained by the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Show a folder picker and import the chosen folder.
    PickImportFolder,
    /// Show the keyboard shortcut overview.
    ShowShortcuts,
    /// Open a folder in the system file manager.
    ShowInExplorer(PathBuf),
    /// Close the application.
    Quit,
}

/// Which models the grid shows, like Lightroom's Catalog/Folders panel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Collection {
    #[default]
    All,
    PreviousImport,
    Folder(PathBuf),
}

#[derive(Debug, Clone)]
enum ScanKind {
    Import,
    Refresh { root: PathBuf },
}

/// A folder scan running on a background thread.
struct Scan {
    folder: PathBuf,
    kind: ScanKind,
    result: Receiver<Vec<ScannedFile>>,
}

/// Where the catalog and caches live on disk.
#[derive(Debug, Clone)]
pub struct Paths {
    pub catalog_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    /// Per-user locations: `%APPDATA%\PolyRoom` and `%LOCALAPPDATA%\PolyRoom` on Windows.
    /// Setting `POLYROOM_HOME` puts everything under that folder instead (for testing).
    pub fn for_user() -> Option<Self> {
        if let Some(home) = std::env::var_os("POLYROOM_HOME") {
            let home = PathBuf::from(home);
            return Some(Self {
                catalog_dir: home.join("catalog"),
                cache_dir: home.join("thumbnails"),
            });
        }
        let dirs = directories::ProjectDirs::from("", "", "PolyRoom")?;
        Some(Self {
            catalog_dir: dirs.data_dir().join("catalog"),
            cache_dir: dirs.cache_dir().join("thumbnails"),
        })
    }
}

/// The file shown in the loupe: a model's mesh, an image, or a ZIP's contents.
#[derive(Debug, Default)]
pub struct Loupe {
    /// File whose content is loaded (or loading).
    pub id: Option<ModelId>,
    pub mesh: Option<Arc<Mesh>>,
    /// The image shown: the file itself if it is an image, else the stacked `photo`.
    pub picture: Option<Arc<Image>>,
    /// The image `picture` is (or will be) of.
    pub picture_of: Option<ModelId>,
    /// The files inside a ZIP.
    pub archive: Option<Vec<ArchiveEntry>>,
    /// A photo from the model's stack shown instead of the 3D view.
    pub photo: Option<ModelId>,
    pub loading: bool,
    pub error: Option<String>,
}

/// All state of one running PolyRoom instance.
#[derive(Default)]
pub struct Session {
    pub module: Module,
    pub view: ViewMode,
    pub panels: Panels,
    /// Grid thumbnail edge length in logical pixels.
    pub thumb_size: f32,
    /// Whether the 3D view draws the build-plate grid.
    pub show_bed_grid: bool,
    /// Whether models with stacked photos show their first photo in the grid instead of a render.
    pub photo_covers: bool,
    /// Columns in the grid as last laid out by the UI; used for Up/Down navigation.
    pub grid_columns: usize,

    pub(crate) catalog: Catalog,
    preview: Option<PreviewService>,
    pub(crate) collection: Collection,
    pub(crate) filter: String,
    pub(crate) attributes: AttributeFilter,
    visible: Vec<ModelId>,
    /// Subfolders shown as cells before the models when browsing a folder.
    visible_folders: Vec<PathBuf>,
    visible_for: Option<VisibleKey>,
    folder_tree: Vec<FolderNode>,
    stacks: Stacks,
    pub(crate) current: Option<ModelId>,
    pub(crate) selection: HashSet<ModelId>,
    pub(crate) anchor: Option<ModelId>,
    pub(crate) loupe: Loupe,
    fresh_thumbnails: Vec<(ModelId, Image)>,
    /// Thumbnails that failed this session; not retried until restart.
    thumbnail_failed: HashSet<ModelId>,
    scans: Vec<Scan>,
    import_progress: Tracker,
    stale_thumbnails: Vec<ModelId>,
    status: String,
    requests: Vec<Request>,
    settings_path: Option<PathBuf>,
    saved_settings: Settings,
    /// When the settings first differed from what is on disk; `None` if they match.
    settings_changed_at: Option<Instant>,
}

impl Session {
    pub const THUMB_MIN: f32 = 96.0;
    pub const THUMB_MAX: f32 = 384.0;

    /// A session with an in-memory catalog and no background workers (tests, headless use).
    pub fn new() -> Self {
        Self {
            thumb_size: settings::DEFAULT_THUMB_SIZE,
            show_bed_grid: true,
            photo_covers: true,
            grid_columns: 1,
            ..Self::default()
        }
    }

    /// Opens the on-disk catalog and starts the preview workers.
    pub fn open(paths: &Paths) -> Result<Self, CatalogError> {
        let mut s = Self::new();
        s.catalog = Catalog::open(&paths.catalog_dir)?;
        s.preview = Some(PreviewService::start(paths.cache_dir.clone()));
        s.status = format!("{} models in library", s.catalog.models().len());
        // Start in the first imported folder, browsing it like a file manager.
        if let Some(root) = s.catalog.folders().first() {
            s.collection = Collection::Folder(root.clone());
        }
        s.refresh_visible();
        let path = paths.catalog_dir.join(settings::FILE);
        s.apply_settings(settings::load(&path));
        s.settings_path = Some(path);
        s.saved_settings = s.settings();
        s.refresh_visible();
        s.current = s.visible.first().copied();
        s.selection = s.current.into_iter().collect();
        s.anchor = s.current;
        Ok(s)
    }

    /// The layout choices that are saved between runs.
    pub(crate) fn settings(&self) -> Settings {
        Settings {
            thumb_size: if self.thumb_size.is_finite() {
                self.thumb_size
            } else {
                settings::DEFAULT_THUMB_SIZE
            },
            show_bed_grid: self.show_bed_grid,
            photo_covers: self.photo_covers,
            panels: self.panels,
            view: self.view,
            module: self.module,
            collection: self.collection.clone(),
        }
    }

    /// Restores saved choices. A folder that is no longer in the library is ignored.
    pub(crate) fn apply_settings(&mut self, settings: Settings) {
        self.thumb_size = if settings.thumb_size.is_finite() {
            settings.thumb_size.clamp(Self::THUMB_MIN, Self::THUMB_MAX)
        } else {
            settings::DEFAULT_THUMB_SIZE
        };
        self.panels = settings.panels;
        self.show_bed_grid = settings.show_bed_grid;
        self.photo_covers = settings.photo_covers;
        self.view = settings.view;
        self.module = settings.module;
        let available = match &settings.collection {
            Collection::Folder(path) => self.folder(path).is_some(),
            Collection::All | Collection::PreviousImport => true,
        };
        if available {
            self.collection = settings.collection;
        }
    }

    /// Writes the settings now if they changed (the front-end calls this on exit).
    pub fn save_settings(&mut self) {
        let Some(path) = self.settings_path.clone() else {
            return;
        };
        let now = self.settings();
        self.settings_changed_at = None;
        if now == self.saved_settings {
            return;
        }
        if let Err(e) = settings::save(&path, &now) {
            log::error!("could not save settings: {e}");
            self.status = format!("Settings not saved: {e}");
        }
        // Don't retry a failing write every few hundred milliseconds.
        self.saved_settings = now;
    }

    /// Saves changed settings once they have been stable for a moment.
    fn sync_settings(&mut self) {
        if self.settings_path.is_none() {
            return;
        }
        if self.settings() == self.saved_settings {
            self.settings_changed_at = None;
        } else if self
            .settings_changed_at
            .get_or_insert_with(Instant::now)
            .elapsed()
            >= SETTINGS_DEBOUNCE
        {
            self.save_settings();
        }
    }

    pub fn request(&mut self, request: Request) {
        if !self.requests.contains(&request) {
            self.requests.push(request);
        }
    }

    /// Takes all pending front-end requests.
    pub fn drain_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    // ---- read access for the UI ----

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn model(&self, id: ModelId) -> Option<&Model> {
        self.catalog.get(id)
    }

    /// Models shown in the grid, in display order.
    pub fn visible(&self) -> &[ModelId] {
        &self.visible
    }

    /// Subfolders of the current folder, shown in the grid before its models.
    pub fn visible_folders(&self) -> &[PathBuf] {
        &self.visible_folders
    }

    /// The photos stacked on `id`, cover first (empty if none).
    pub fn stack(&self, id: ModelId) -> &[ModelId] {
        self.stacks.photos(id)
    }

    /// What the grid shows for `id`: its cover photo, if it has a stack and photo covers are
    /// on, else `id` itself.
    pub fn cover(&self, id: ModelId) -> ModelId {
        match self.stack(id).first() {
            Some(&photo) if self.photo_covers => photo,
            _ => id,
        }
    }

    /// One folder tree per imported folder.
    pub fn folder_tree(&self) -> &[FolderNode] {
        &self.folder_tree
    }

    /// The tree node for path, if it holds any models.
    pub fn folder(&self, path: &Path) -> Option<&FolderNode> {
        self.folder_tree.iter().find_map(|root| root.find(path))
    }

    /// The imported folder that path lies in.
    pub fn folder_root(&self, path: &Path) -> Option<&Path> {
        self.folder_tree
            .iter()
            .map(|root| root.path.as_path())
            .find(|root| path.starts_with(root))
    }

    pub fn collection(&self) -> &Collection {
        &self.collection
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn attributes(&self) -> &AttributeFilter {
        &self.attributes
    }

    pub fn current(&self) -> Option<ModelId> {
        self.current
    }

    pub fn is_selected(&self, id: ModelId) -> bool {
        self.selection.contains(&id)
    }

    pub fn selection_len(&self) -> usize {
        self.selection.len()
    }

    pub fn loupe(&self) -> &Loupe {
        &self.loupe
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    /// True while background work is running; the UI keeps repainting to pick up results.
    pub fn is_busy(&self) -> bool {
        !self.scans.is_empty()
            || !self.fresh_thumbnails.is_empty()
            || self.settings_changed_at.is_some()
            || self.import_progress.is_active()
            || self.preview.as_ref().is_some_and(PreviewService::is_busy)
    }

    /// Overall progress of scanning and preview generation, if any is running.
    pub fn progress(&self) -> Option<Progress> {
        if !self.scans.is_empty() {
            return Some(Progress {
                text: "Scanning folders…".into(),
                fraction: None,
            });
        }
        self.import_progress.progress()
    }

    /// Requests previews for queued import models, a few at a time.
    fn feed_import_previews(&mut self) {
        while self
            .preview
            .as_ref()
            .is_some_and(|p| p.pending_thumbnails() < IMPORT_IN_FLIGHT)
        {
            let Some(id) = self.import_progress.next() else {
                break;
            };
            self.want_thumbnail(id);
            let queued = self
                .preview
                .as_ref()
                .is_some_and(|p| p.is_thumbnail_pending(id));
            if !queued {
                self.import_progress.finish(id);
            }
        }
    }

    // ---- background work ----

    pub(crate) fn start_import(&mut self, folder: PathBuf) {
        self.start_scan(folder, ScanKind::Import);
    }

    /// Rescans `folder` (an imported folder or one inside it) and syncs the catalog with it.
    pub(crate) fn start_refresh(&mut self, folder: PathBuf) -> Result<(), String> {
        let root = self
            .folder_root(&folder)
            .ok_or_else(|| format!("{} is not inside an imported folder", folder.display()))?
            .to_owned();
        if self.scans.iter().any(|s| s.folder == folder) {
            return Ok(()); // already refreshing
        }
        self.start_scan(folder, ScanKind::Refresh { root });
        Ok(())
    }

    fn start_scan(&mut self, folder: PathBuf, kind: ScanKind) {
        let (tx, rx) = mpsc::channel();
        let scan_dir = folder.clone();
        let spawned = std::thread::Builder::new()
            .name("polyroom-scan".into())
            .spawn(move || {
                let _ = tx.send(polyroom_catalog::scan_folder(&scan_dir));
            });
        match spawned {
            Ok(_) => {
                self.status = format!("Scanning {}…", folder.display());
                self.scans.push(Scan {
                    folder,
                    kind,
                    result: rx,
                });
            }
            Err(e) => self.status = format!("Could not start scanning: {e}"),
        }
    }

    fn finish_refresh(&mut self, root: &Path, folder: &Path, files: Vec<ScannedFile>) {
        let name = folder.file_name().map_or_else(
            || folder.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let report = match self.catalog.sync(root, folder, files) {
            Ok(report) => report,
            Err(e) => {
                self.status = format!("Refreshing {name} failed: {e}");
                return;
            }
        };
        // Changed files need new thumbnails and, if open, a fresh mesh in the loupe.
        for id in &report.updated {
            self.thumbnail_failed.remove(id);
            if self.loupe.id == Some(*id) {
                self.loupe = Loupe::default();
            }
        }
        self.stale_thumbnails.extend(&report.updated);
        self.status = if report == SyncReport::default() {
            format!("{name} is up to date")
        } else {
            format!(
                "Refreshed {name}: {} added, {} removed, {} changed",
                report.added,
                report.removed.len(),
                report.updated.len()
            )
        };
        self.refresh_visible();
        // If the folder being browsed is gone, go to the nearest folder that still exists.
        if let Collection::Folder(current) = &self.collection
            && self.folder(current).is_none()
        {
            let fallback = current
                .ancestors()
                .find(|p| self.folder(p).is_some())
                .unwrap_or(root)
                .to_owned();
            self.collection = Collection::Folder(fallback);
            self.refresh_visible();
        }
    }

    /// Models whose thumbnails went stale (file changed); the UI drops its cached images.
    pub fn take_stale_thumbnails(&mut self) -> Vec<ModelId> {
        std::mem::take(&mut self.stale_thumbnails)
    }

    /// Runs `ops` against the catalog with a single log write.
    fn apply_all(&mut self, ops: Vec<Op>) {
        if ops.is_empty() {
            return;
        }
        if let Err(e) = self.catalog.apply_all(ops) {
            log::error!("{e}");
            self.status = format!("Changes not saved: {e}");
        }
    }

    /// Runs `op` against the catalog, reporting a failed save in the status bar.
    pub(crate) fn apply(&mut self, op: Op) {
        if let Err(e) = self.catalog.apply(op) {
            log::error!("{e}");
            self.status = format!("Changes not saved: {e}");
        }
    }

    /// Asks for a thumbnail of `id` (the UI calls this for visible cells it has no image for).
    pub fn want_thumbnail(&mut self, id: ModelId) {
        let Some(preview) = &mut self.preview else {
            return;
        };
        let Some(model) = self.catalog.get(id) else {
            return;
        };
        if model.error.is_some()
            || self.thumbnail_failed.contains(&id)
            || preview.is_thumbnail_pending(id)
        {
            return;
        }
        let need_stats = model.format.is_mesh() && model.stats.is_none();
        preview.request_thumbnail(id, source(model), need_stats);
        self.import_progress.track(id);
    }

    /// Whether a thumbnail can't be shown (bad file or rendering failure).
    pub fn thumbnail_failed(&self, id: ModelId) -> bool {
        self.thumbnail_failed.contains(&id)
            || self.catalog.get(id).is_some_and(|m| m.error.is_some())
    }

    /// Thumbnails that finished since the last call.
    /// A few at a time, so uploading them to the GPU can't stall a frame.
    pub fn take_thumbnails(&mut self) -> Vec<(ModelId, Image)> {
        let n = self.fresh_thumbnails.len().min(THUMBNAILS_PER_FRAME);
        self.fresh_thumbnails.drain(..n).collect()
    }

    /// Collects background results and keeps derived state up to date. Call once per frame.
    pub fn poll(&mut self) {
        let mut finished = Vec::new();
        self.scans.retain(|scan| match scan.result.try_recv() {
            Ok(files) => {
                finished.push((scan.folder.clone(), scan.kind.clone(), files));
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        });
        for (folder, kind, files) in finished {
            match kind {
                ScanKind::Import => self.finish_import(&folder, files),
                ScanKind::Refresh { root } => self.finish_refresh(&root, &folder, files),
            }
        }

        let events = self
            .preview
            .as_mut()
            .map(PreviewService::poll)
            .unwrap_or_default();
        // Stats and errors are saved with one write per frame, not one per model.
        let mut ops = Vec::new();
        for event in events {
            match &event {
                Event::Thumbnail { id, .. } | Event::Failed { id, .. } => {
                    self.import_progress.finish(*id);
                }
                _ => {}
            }
            match event {
                Event::Stats { id, stats } => ops.push(Op::SetStats { id, stats }),
                Event::Thumbnail { id, image } => self.fresh_thumbnails.push((id, image)),
                Event::Mesh { id, mesh } => {
                    if self.loupe.id == Some(id) {
                        if let (Some(stats), true) = (
                            mesh.stats(),
                            self.catalog.get(id).is_some_and(|m| m.stats.is_none()),
                        ) {
                            self.apply(Op::SetStats { id, stats });
                        }
                        self.loupe.mesh = Some(mesh);
                        self.loupe.loading = false;
                        self.loupe.error = None;
                    }
                }
                Event::Picture { id, image } => {
                    if self.loupe.picture_of == Some(id) {
                        self.loupe.picture = Some(image);
                        if self.loupe.id == Some(id) {
                            self.loupe.loading = false;
                            self.loupe.error = None;
                        }
                    }
                }
                Event::Archive { id, entries } => {
                    if self.loupe.id == Some(id) {
                        self.loupe.archive = Some(entries);
                        self.loupe.loading = false;
                        self.loupe.error = None;
                    }
                }
                Event::Failed {
                    id,
                    error,
                    bad_file,
                } => {
                    if self.loupe.id == Some(id) && self.loupe.loading {
                        self.loupe.loading = false;
                        self.loupe.mesh = None;
                        self.loupe.error = Some(error.clone());
                    }
                    self.thumbnail_failed.insert(id);
                    if bad_file {
                        ops.push(Op::SetError {
                            id,
                            error: Some(error),
                        });
                    } else {
                        log::warn!("thumbnail for model {id} failed: {error}");
                    }
                }
            }
        }

        self.apply_all(ops);
        self.feed_import_previews();
        self.sync_settings();
        self.refresh_visible();
        self.update_loupe();
    }

    fn finish_import(&mut self, folder: &Path, files: Vec<ScannedFile>) {
        let found = files.len();
        match self.catalog.import(folder, files) {
            Ok(added) => {
                self.status = match (found, added) {
                    (0, _) => format!(
                        "No models, images or ZIP files found in {}",
                        folder.display()
                    ),
                    (_, 0) => format!(
                        "All {found} models in {} were already imported",
                        folder.display()
                    ),
                    _ => format!("Imported {added} models from {}", folder.display()),
                };
                if added > 0 {
                    if self.preview.is_some() {
                        let last_import = self.catalog.last_import();
                        let new_models = self
                            .catalog
                            .models()
                            .iter()
                            .filter(|m| m.import == last_import && m.error.is_none())
                            .map(|m| m.id);
                        self.import_progress.add(new_models);
                    }
                    self.collection = Collection::Folder(folder.to_owned());
                    self.filter.clear();
                    self.attributes.clear();
                    self.refresh_visible();
                    self.current = self.visible.first().copied();
                    self.selection = self.current.into_iter().collect();
                    self.anchor = self.current;
                }
            }
            Err(e) => self.status = format!("Import failed: {e}"),
        }
    }

    /// Loads what the loupe or viewer shows of the current file: a model's mesh, an image, a
    /// ZIP's contents, or a photo from the model's stack.
    pub(crate) fn update_loupe(&mut self) {
        let showing = self.module == Module::Viewer || self.view == ViewMode::Loupe;
        let Some(id) = self.current.filter(|_| showing) else {
            return;
        };
        if self.loupe.id != Some(id) {
            let Some(model) = self.catalog.get(id) else {
                return;
            };
            let (format, source) = (model.format, source(model));
            // Everything held belongs to the previous file; the 3D view files whatever mesh it
            // finds here under `loupe.id`.
            self.loupe = Loupe {
                id: Some(id),
                ..Loupe::default()
            };
            match &mut self.preview {
                Some(preview) => {
                    self.loupe.loading = true;
                    match format {
                        Format::Image => {
                            self.loupe.picture_of = Some(id);
                            preview.request_picture(id, source, LOUPE_PICTURE_PX);
                        }
                        Format::Zip => preview.request_archive(id, source),
                        _ => preview.request_mesh(id, source),
                    }
                }
                None => self.loupe.error = Some("previews are not available".into()),
            }
        }
        // A stacked photo picked in the loupe.
        if let Some(photo) = self.loupe.photo
            && self.loupe.picture_of != Some(photo)
        {
            self.loupe.picture = None;
            self.loupe.picture_of = Some(photo);
            if let (Some(preview), Some(model)) = (&mut self.preview, self.catalog.get(photo)) {
                preview.request_picture(photo, source(model), LOUPE_PICTURE_PX);
            }
        }
    }

    /// Recomputes the visible list if the catalog, collection or filter changed.
    pub(crate) fn refresh_visible(&mut self) {
        // Ratings, flags and labels don't change the layout, but they do change what an
        // attribute filter shows.
        let edits = if self.attributes.is_active() {
            self.catalog.version()
        } else {
            0
        };
        let key: VisibleKey = (
            self.catalog.layout_version(),
            edits,
            self.collection.clone(),
            self.filter.clone(),
            self.attributes.clone(),
        );
        if self.visible_for.as_ref() == Some(&key) {
            return;
        }
        let words: Vec<String> = self
            .filter
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        if self.visible_for.as_ref().map(|k| k.0) != Some(key.0) {
            self.stacks = Stacks::build(self.catalog.models());
            self.folder_tree =
                folders::build(self.catalog.folders(), self.catalog.models(), &self.stacks);
        }
        let last_import = self.catalog.last_import();
        // In a folder: only its own models, unless searching, which looks in subfolders too.
        let searching = !words.is_empty() || self.attributes.is_active();
        self.visible = self
            .catalog
            .models()
            .iter()
            .filter(|m| match &self.collection {
                Collection::All => true,
                Collection::PreviousImport => m.import == last_import,
                Collection::Folder(f) if searching => m.path.starts_with(f),
                Collection::Folder(f) => m.path.parent() == Some(f.as_path()),
            })
            .filter(|m| self.attributes.matches(m))
            .filter(|m| {
                let name = m.name().to_lowercase();
                words.iter().all(|w| name.contains(w.as_str()))
            })
            .map(|m| m.id)
            .collect();
        // A photo stacked on a model that is shown is shown through that model.
        let shown: HashSet<ModelId> = self.visible.iter().copied().collect();
        self.visible.retain(|id| {
            self.stacks
                .owner(*id)
                .is_none_or(|owner| !shown.contains(&owner))
        });
        self.visible_folders = match &self.collection {
            Collection::Folder(f) if !searching => self
                .folder(f)
                .map(|n| n.children.iter().map(|c| c.path.clone()).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        self.visible_for = Some(key);
        // Drop selection that is no longer visible.
        let visible: HashSet<ModelId> = self.visible.iter().copied().collect();
        self.selection.retain(|id| visible.contains(id));
        if self.current.is_some_and(|id| !visible.contains(&id)) {
            self.current = self.selection.iter().next().copied();
        }
    }

    /// The models an edit applies to: the selection, or else the current model.
    pub(crate) fn targets(&self) -> Vec<ModelId> {
        if self.selection.is_empty() {
            self.current.into_iter().collect()
        } else {
            // Keep display order so the op log reads naturally.
            self.visible
                .iter()
                .copied()
                .filter(|id| self.selection.contains(id))
                .collect()
        }
    }
}

fn source(model: &Model) -> polyroom_preview::Source {
    polyroom_preview::Source {
        path: model.path.clone(),
        size: model.size,
        modified: model.modified,
    }
}

#[cfg(test)]
mod tests {
    use polyroom_formats::Format;

    use super::*;
    use crate::default_registry;

    /// A session whose catalog holds files in two folders and which saves settings to `path`.
    fn session_saving_to(path: &Path) -> Session {
        let mut s = Session::new();
        for root in ["C:/a", "C:/b"] {
            let file = ScannedFile {
                path: Path::new(root).join("m.stl"),
                format: Format::Stl,
                size: 1,
                modified: None,
            };
            s.catalog.import(Path::new(root), vec![file]).unwrap();
        }
        s.refresh_visible();
        s.settings_path = Some(path.to_owned());
        s.saved_settings = s.settings();
        s
    }

    /// What `Session::open` does with the settings file, minus the catalog and GPU workers.
    fn reopened(path: &Path) -> Session {
        let mut s = session_saving_to(path);
        s.apply_settings(settings::load(path));
        s
    }

    #[test]
    fn layout_choices_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(settings::FILE);
        let r = default_registry();
        let mut s = session_saving_to(&path);
        r.run("view.thumb_size", &mut s, &json_size(300)).unwrap();
        r.run("view.toggle_left_panel", &mut s, &serde_json::Value::Null)
            .unwrap();
        r.run(
            "library.show_folder",
            &mut s,
            &serde_json::json!({"path": "C:/b"}),
        )
        .unwrap();
        s.view = ViewMode::Loupe;

        // Nothing is written until the change has settled...
        s.poll();
        assert!(!path.exists());
        assert!(s.is_busy(), "the UI must keep repainting to flush it");
        // ...then it is, without another change.
        std::thread::sleep(SETTINGS_DEBOUNCE + Duration::from_millis(50));
        s.poll();
        assert!(path.exists());
        assert!(!s.is_busy());

        let s2 = reopened(&path);
        assert_eq!(s2.thumb_size, 300.0);
        assert!(!s2.panels.left && s2.panels.right && s2.panels.filmstrip);
        assert_eq!(s2.view, ViewMode::Loupe);
        assert_eq!(s2.collection, Collection::Folder("C:/b".into()));
    }

    /// Regression: opening a model in the loupe after another one showed the *previous* mesh,
    /// because the loupe kept the old mesh while it switched to the new id, and the 3D view
    /// then filed that old mesh under the new id.
    #[test]
    fn switching_models_never_pairs_the_new_id_with_the_old_mesh() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session_saving_to(&dir.path().join(settings::FILE));
        let (a, b) = (s.catalog().models()[0].id, s.catalog().models()[1].id);
        let cube = Arc::new(polyroom_mesh::test_cube());

        // Model `a` is open in the loupe with its mesh loaded.
        s.view = ViewMode::Loupe;
        s.current = Some(a);
        s.update_loupe();
        s.loupe.mesh = Some(Arc::clone(&cube));
        s.loupe.loading = false;
        assert_eq!(s.loupe().id, Some(a));

        // Back to the grid, then double-click `b`.
        s.view = ViewMode::Grid;
        s.current = Some(b);
        s.view = ViewMode::Loupe;
        s.update_loupe();

        assert_eq!(s.loupe().id, Some(b));
        assert!(
            s.loupe().mesh.is_none(),
            "the loupe must not hold model {a}'s mesh while showing model {b}"
        );
    }

    #[test]
    fn the_bed_grid_toggle_is_a_command_and_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(settings::FILE);
        let r = default_registry();
        let mut s = session_saving_to(&path);
        assert!(s.show_bed_grid, "on by default");
        r.run("view.toggle_bed_grid", &mut s, &serde_json::Value::Null)
            .unwrap();
        assert!(!s.show_bed_grid);
        s.save_settings();
        assert!(!reopened(&path).show_bed_grid);
        r.run("view.toggle_bed_grid", &mut s, &serde_json::Value::Null)
            .unwrap();
        assert!(s.show_bed_grid);
    }

    #[test]
    fn save_settings_flushes_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(settings::FILE);
        let mut s = session_saving_to(&path);
        s.panels.right = false;
        s.save_settings();
        assert!(!reopened(&path).panels.right);
    }

    #[test]
    fn unusable_saved_values_are_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(settings::FILE);
        let saved = Settings {
            thumb_size: 10_000.0,
            collection: Collection::Folder("C:/gone".into()),
            ..Settings::default()
        };
        settings::save(&path, &saved).unwrap();
        let s = reopened(&path);
        assert_eq!(s.thumb_size, Session::THUMB_MAX);
        // The saved folder is no longer imported, so the collection stays as it was.
        assert_eq!(s.collection, Collection::All);
    }

    fn json_size(size: u32) -> serde_json::Value {
        serde_json::json!({ "size": size })
    }
}
