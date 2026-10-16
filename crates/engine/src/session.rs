use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use polyroom_catalog::{Catalog, CatalogError, Model, ModelId, Op, ScannedFile};
use polyroom_mesh::Mesh;
use polyroom_preview::{Event, PreviewService};
use polyroom_render::Image;

/// The top-level module, like Lightroom's Library/Develop switcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Module {
    #[default]
    Library,
    Viewer,
}

/// How the centre of the Library module shows models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Grid,
    Loupe,
}

/// Visibility of the panels around the centre view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Close the application.
    Quit,
}

/// Which models the grid shows, like Lightroom's Catalog/Folders panel.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Collection {
    #[default]
    All,
    PreviousImport,
    Folder(PathBuf),
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

/// The model shown in the loupe.
#[derive(Debug, Default)]
pub struct Loupe {
    /// Model whose mesh is loaded (or loading).
    pub id: Option<ModelId>,
    pub mesh: Option<Arc<Mesh>>,
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
    /// Columns in the grid as last laid out by the UI; used for Up/Down navigation.
    pub grid_columns: usize,

    pub(crate) catalog: Catalog,
    preview: Option<PreviewService>,
    pub(crate) collection: Collection,
    pub(crate) filter: String,
    visible: Vec<ModelId>,
    visible_for: Option<(u64, Collection, String)>,
    pub(crate) current: Option<ModelId>,
    pub(crate) selection: HashSet<ModelId>,
    pub(crate) anchor: Option<ModelId>,
    loupe: Loupe,
    fresh_thumbnails: Vec<(ModelId, Image)>,
    /// Thumbnails that failed this session; not retried until restart.
    thumbnail_failed: HashSet<ModelId>,
    scans: Vec<Receiver<(PathBuf, Vec<ScannedFile>)>>,
    status: String,
    requests: Vec<Request>,
}

impl Session {
    pub const THUMB_MIN: f32 = 96.0;
    pub const THUMB_MAX: f32 = 384.0;

    /// A session with an in-memory catalog and no background workers (tests, headless use).
    pub fn new() -> Self {
        Self {
            thumb_size: 192.0,
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
        s.refresh_visible();
        s.current = s.visible.first().copied();
        s.selection = s.current.into_iter().collect();
        s.anchor = s.current;
        Ok(s)
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

    pub fn collection(&self) -> &Collection {
        &self.collection
    }

    pub fn filter(&self) -> &str {
        &self.filter
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
        !self.scans.is_empty() || self.preview.as_ref().is_some_and(PreviewService::is_busy)
    }

    // ---- background work ----

    pub(crate) fn start_import(&mut self, folder: PathBuf) {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("polyroom-scan".into())
            .spawn(move || {
                let files = polyroom_catalog::scan_folder(&folder);
                let _ = tx.send((folder, files));
            });
        match spawned {
            Ok(_) => {
                self.scans.push(rx);
                self.status = "Scanning folder…".into();
            }
            Err(e) => self.status = format!("Could not start import: {e}"),
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
        let need_stats = model.stats.is_none();
        preview.request_thumbnail(id, source(model), need_stats);
    }

    /// Whether a thumbnail can't be shown (bad file or rendering failure).
    pub fn thumbnail_failed(&self, id: ModelId) -> bool {
        self.thumbnail_failed.contains(&id)
            || self.catalog.get(id).is_some_and(|m| m.error.is_some())
    }

    /// Thumbnails that finished since the last call.
    pub fn take_thumbnails(&mut self) -> Vec<(ModelId, Image)> {
        std::mem::take(&mut self.fresh_thumbnails)
    }

    /// Collects background results and keeps derived state up to date. Call once per frame.
    pub fn poll(&mut self) {
        let mut finished = Vec::new();
        self.scans.retain(|rx| match rx.try_recv() {
            Ok(result) => {
                finished.push(result);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        });
        for (folder, files) in finished {
            self.finish_import(&folder, files);
        }

        let events = self
            .preview
            .as_mut()
            .map(PreviewService::poll)
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Stats { id, stats } => self.apply(Op::SetStats { id, stats }),
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
                        self.apply(Op::SetError {
                            id,
                            error: Some(error),
                        });
                    } else {
                        log::warn!("thumbnail for model {id} failed: {error}");
                    }
                }
            }
        }

        self.refresh_visible();
        self.update_loupe();
    }

    fn finish_import(&mut self, folder: &Path, files: Vec<ScannedFile>) {
        let found = files.len();
        match self.catalog.import(folder, files) {
            Ok(added) => {
                self.status = match (found, added) {
                    (0, _) => format!("No STL, 3MF or OBJ files found in {}", folder.display()),
                    (_, 0) => format!(
                        "All {found} models in {} were already imported",
                        folder.display()
                    ),
                    _ => format!("Imported {added} models from {}", folder.display()),
                };
                if added > 0 {
                    self.collection = Collection::PreviousImport;
                    self.filter.clear();
                    self.refresh_visible();
                    self.current = self.visible.first().copied();
                    self.selection = self.current.into_iter().collect();
                    self.anchor = self.current;
                }
            }
            Err(e) => self.status = format!("Import failed: {e}"),
        }
    }

    /// Loads the current model's mesh when the loupe or viewer shows it.
    fn update_loupe(&mut self) {
        let showing = self.module == Module::Viewer || self.view == ViewMode::Loupe;
        let Some(id) = self.current.filter(|_| showing) else {
            return;
        };
        if self.loupe.id == Some(id) {
            return;
        }
        let Some(model) = self.catalog.get(id) else {
            return;
        };
        let source = source(model);
        self.loupe.id = Some(id);
        self.loupe.error = None;
        match &mut self.preview {
            Some(preview) => {
                self.loupe.loading = true;
                preview.request_mesh(id, source);
            }
            None => self.loupe.error = Some("previews are not available".into()),
        }
    }

    /// Recomputes the visible list if the catalog, collection or filter changed.
    pub(crate) fn refresh_visible(&mut self) {
        let key = (
            self.catalog.version(),
            self.collection.clone(),
            self.filter.clone(),
        );
        if self.visible_for.as_ref() == Some(&key) {
            return;
        }
        let words: Vec<String> = self
            .filter
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let last_import = self.catalog.last_import();
        self.visible = self
            .catalog
            .models()
            .iter()
            .filter(|m| match &self.collection {
                Collection::All => true,
                Collection::PreviousImport => m.import == last_import,
                Collection::Folder(f) => &m.folder == f,
            })
            .filter(|m| {
                let name = m.name().to_lowercase();
                words.iter().all(|w| name.contains(w.as_str()))
            })
            .map(|m| m.id)
            .collect();
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
