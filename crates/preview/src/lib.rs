//! Background work for previews: parsing models, computing stats and rendering thumbnails.
//!
//! A few CPU workers parse files; one GPU thread renders thumbnails with its own wgpu device.
//! Rendered thumbnails are cached as PNGs keyed by path, size, modification time and renderer
//! version, so they survive restarts. Results come back as [`Event`]s, polled by the engine.

#![forbid(unsafe_code)]

mod cache;
mod picture;

use std::collections::{HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;

use polyroom_formats::Format;
use polyroom_formats::archive::{self, ArchiveEntry};
use polyroom_mesh::{Mesh, MeshStats};
use polyroom_render::{Headless, Image, Style};

pub use cache::{decode_png, encode_png};

/// Default thumbnail edge length in pixels.
pub const THUMB_PX: u32 = 256;
/// Largest image inside a ZIP we decode for its cover.
const MAX_ZIP_IMAGE: u64 = 64 << 20;
/// Largest model inside a ZIP we render for its cover.
const MAX_ZIP_MESH: u64 = 256 << 20;
/// Models inside a ZIP tried for its cover before giving up.
const ZIP_MESH_TRIES: usize = 3;
/// Bump to invalidate cached thumbnails after changing how they look.
const RENDER_VERSION: u32 = 2;

pub type JobId = u64;

/// What identifies a model file for caching purposes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Source {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<u64>,
}

/// How thumbnails look. Each style has its own cache files, so changing it never serves stale
/// images, and changing it back is instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThumbStyle {
    /// Edge length in pixels.
    pub size: u32,
    /// sRGB background colour; `None` is transparent.
    pub background: Option<[u8; 3]>,
    /// sRGB model colour.
    pub model_color: [u8; 3],
    /// Bumped to throw away every cached thumbnail.
    pub generation: u32,
}

impl Default for ThumbStyle {
    fn default() -> Self {
        Self {
            size: THUMB_PX,
            background: None,
            model_color: [207, 203, 198],
            generation: 0,
        }
    }
}

impl ThumbStyle {
    fn render_style(&self) -> Style {
        let channel = |c: u8| f32::from(c) / 255.0;
        let linear = |c: u8| {
            let c = channel(c);
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b] = self.model_color;
        Style {
            background: self.background.map_or([0.0; 4], |[r, g, b]| {
                [channel(r), channel(g), channel(b), 1.0]
            }),
            base_color: [linear(r), linear(g), linear(b)],
            show_grid: false,
        }
    }

    /// Cached files of this generation start with this.
    fn file_prefix(&self) -> String {
        format!("g{}-", self.generation)
    }
}

impl Source {
    fn cache_key(&self, style: &ThumbStyle) -> String {
        let mut h = DefaultHasher::new();
        (self, RENDER_VERSION, style).hash(&mut h);
        format!("{}{:016x}", style.file_prefix(), h.finish())
    }
}

#[derive(Debug)]
pub enum Event {
    /// Stats from a freshly parsed model.
    Stats { id: JobId, stats: MeshStats },
    Thumbnail {
        id: JobId,
        image: Image,
        /// The look the image was made in; images of an older style are dropped.
        style: ThumbStyle,
    },
    /// A full mesh for the loupe.
    Mesh { id: JobId, mesh: Arc<Mesh> },
    /// A preview image at loupe size.
    Picture { id: JobId, image: Arc<Image> },
    /// The files inside a ZIP archive.
    Archive {
        id: JobId,
        entries: Vec<ArchiveEntry>,
    },
    /// Loading or rendering failed; `error` is shown to the user. `bad_file` is false when the
    /// file itself is fine (e.g. no GPU for thumbnails).
    Failed {
        id: JobId,
        error: String,
        bad_file: bool,
    },
}

#[derive(Debug, Clone)]
enum Job {
    Thumbnail {
        id: JobId,
        source: Source,
        /// Parse the model even when the thumbnail is cached, to get its stats.
        need_stats: bool,
        style: ThumbStyle,
    },
    Mesh {
        id: JobId,
        source: Source,
    },
    Picture {
        id: JobId,
        source: Source,
        max_px: u32,
    },
    Archive {
        id: JobId,
        source: Source,
    },
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    shutdown: bool,
}

type SharedQueue = Arc<(Mutex<Queue>, Condvar)>;

pub struct PreviewService {
    cache_dir: PathBuf,
    style: ThumbStyle,
    queue: SharedQueue,
    events: Receiver<Event>,
    pending_thumbs: HashSet<JobId>,
    pending_mesh: Option<JobId>,
    /// A picture or archive listing for the loupe.
    pending_detail: Option<JobId>,
}

impl PreviewService {
    /// Starts the workers. Thumbnails are cached under `cache_dir`.
    pub fn start(cache_dir: PathBuf) -> Self {
        let queue: SharedQueue = Arc::default();
        let (event_tx, events) = mpsc::channel();
        // Bounded so parsers can't run far ahead of the GPU and pile up meshes in memory.
        let (render_tx, render_rx) = mpsc::sync_channel::<RenderJob>(2);

        let gpu_events = event_tx.clone();
        let spawned = thread::Builder::new()
            .name("polyroom-gpu".into())
            .spawn(move || gpu_thread(&render_rx, &gpu_events));
        if let Err(e) = spawned {
            log::error!("could not start thumbnail renderer: {e}");
        }

        let workers = thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4));
        for i in 0..workers {
            let queue = Arc::clone(&queue);
            let events = event_tx.clone();
            let render = render_tx.clone();
            let cache_dir = cache_dir.clone();
            let spawned = thread::Builder::new()
                .name(format!("polyroom-loader-{i}"))
                .spawn(move || worker(&queue, &events, &render, &cache_dir));
            if let Err(e) = spawned {
                log::error!("could not start loader thread: {e}");
            }
        }

        Self {
            cache_dir,
            style: ThumbStyle::default(),
            queue,
            events,
            pending_thumbs: HashSet::new(),
            pending_mesh: None,
            pending_detail: None,
        }
    }

    fn push(&self, job: Job, front: bool) {
        let (lock, cvar) = &*self.queue;
        let mut q = lock.lock().unwrap_or_else(PoisonError::into_inner);
        if front {
            q.jobs.push_front(job);
        } else {
            q.jobs.push_back(job);
        }
        cvar.notify_one();
    }

    pub fn style(&self) -> ThumbStyle {
        self.style
    }

    /// Changes how new thumbnails look. Queued thumbnails are dropped so that they are made
    /// again in the new style; thumbnails already made are the caller's to refresh.
    pub fn set_style(&mut self, style: ThumbStyle) {
        if style == self.style {
            return;
        }
        self.style = style;
        self.clear_queued_thumbnails();
    }

    /// Queues a thumbnail unless one is already pending for `id`.
    pub fn request_thumbnail(&mut self, id: JobId, source: Source, need_stats: bool) {
        if self.pending_thumbs.insert(id) {
            let style = self.style;
            self.push(
                Job::Thumbnail {
                    id,
                    source,
                    need_stats,
                    style,
                },
                false,
            );
        }
    }

    /// Deletes the cached thumbnail of `source`, so the next request renders it again.
    pub fn forget_cached(&self, source: &Source) {
        let path = self
            .cache_dir
            .join(format!("{}.png", source.cache_key(&self.style)));
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("could not delete {}: {e}", path.display()),
        }
    }

    /// Deletes, in the background, every cached thumbnail that is not of the current
    /// generation (including files from before generations existed).
    pub fn purge_old_generations(&self) {
        let (dir, keep) = (self.cache_dir.clone(), self.style.file_prefix());
        let spawned = thread::Builder::new()
            .name("polyroom-cache-purge".into())
            .spawn(move || {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    return;
                };
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.ends_with(".png") && !name.starts_with(&keep) {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("could not start cache cleanup: {e}");
        }
    }

    /// Queues a full mesh load ahead of all thumbnails, dropping older queued mesh loads.
    pub fn request_mesh(&mut self, id: JobId, source: Source) {
        if self.pending_mesh == Some(id) {
            return;
        }
        {
            let (lock, _) = &*self.queue;
            let mut q = lock.lock().unwrap_or_else(PoisonError::into_inner);
            q.jobs.retain(|j| !matches!(j, Job::Mesh { .. }));
        }
        self.pending_mesh = Some(id);
        self.push(Job::Mesh { id, source }, true);
    }

    /// Queues a loupe-sized picture of an image file, ahead of thumbnails, replacing any
    /// queued picture or archive listing.
    pub fn request_picture(&mut self, id: JobId, source: Source, max_px: u32) {
        self.request_detail(id, Job::Picture { id, source, max_px });
    }

    /// Queues listing the files of a ZIP archive, like [`PreviewService::request_picture`].
    pub fn request_archive(&mut self, id: JobId, source: Source) {
        self.request_detail(id, Job::Archive { id, source });
    }

    fn request_detail(&mut self, id: JobId, job: Job) {
        if self.pending_detail == Some(id) {
            return;
        }
        {
            let (lock, _) = &*self.queue;
            let mut q = lock.lock().unwrap_or_else(PoisonError::into_inner);
            q.jobs
                .retain(|j| !matches!(j, Job::Picture { .. } | Job::Archive { .. }));
        }
        self.pending_detail = Some(id);
        self.push(job, true);
    }

    /// Drops queued thumbnail jobs that haven't started (e.g. after scrolling far away).
    pub fn clear_queued_thumbnails(&mut self) {
        let (lock, _) = &*self.queue;
        let mut q = lock.lock().unwrap_or_else(PoisonError::into_inner);
        q.jobs.retain(|j| {
            let keep = !matches!(j, Job::Thumbnail { .. });
            if let Job::Thumbnail { id, .. } = j {
                self.pending_thumbs.remove(id);
            }
            keep
        });
    }

    pub fn is_thumbnail_pending(&self, id: JobId) -> bool {
        self.pending_thumbs.contains(&id)
    }

    /// How many thumbnails are queued or being made.
    pub fn pending_thumbnails(&self) -> usize {
        self.pending_thumbs.len()
    }

    pub fn is_busy(&self) -> bool {
        !self.pending_thumbs.is_empty()
            || self.pending_mesh.is_some()
            || self.pending_detail.is_some()
    }

    /// Takes all finished results.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events: Vec<Event> = self.events.try_iter().collect();
        for e in &events {
            match e {
                Event::Thumbnail { id, .. } => {
                    self.pending_thumbs.remove(id);
                }
                Event::Mesh { id, .. } => {
                    if self.pending_mesh == Some(*id) {
                        self.pending_mesh = None;
                    }
                }
                Event::Picture { id, .. } | Event::Archive { id, .. } => {
                    if self.pending_detail == Some(*id) {
                        self.pending_detail = None;
                    }
                }
                Event::Failed { id, .. } => {
                    self.pending_thumbs.remove(id);
                    if self.pending_mesh == Some(*id) {
                        self.pending_mesh = None;
                    }
                    if self.pending_detail == Some(*id) {
                        self.pending_detail = None;
                    }
                }
                Event::Stats { .. } => {}
            }
        }
        // A thumbnail made before the style changed would sit next to new ones.
        events.retain(|e| !matches!(e, Event::Thumbnail { style, .. } if *style != self.style));
        events
    }
}

impl Drop for PreviewService {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.queue;
        lock.lock().unwrap_or_else(PoisonError::into_inner).shutdown = true;
        cvar.notify_all();
    }
}

fn next_job(queue: &SharedQueue) -> Option<Job> {
    let (lock, cvar) = &**queue;
    let mut q = lock.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if q.shutdown {
            return None;
        }
        if let Some(job) = q.jobs.pop_front() {
            return Some(job);
        }
        q = cvar.wait(q).unwrap_or_else(PoisonError::into_inner);
    }
}

fn worker(
    queue: &SharedQueue,
    events: &Sender<Event>,
    render: &SyncSender<RenderJob>,
    cache_dir: &Path,
) {
    while let Some(job) = next_job(queue) {
        let sent = match job {
            Job::Mesh { id, source } => match load_guarded(&source.path) {
                Ok(mesh) => events.send(Event::Mesh {
                    id,
                    mesh: Arc::new(mesh),
                }),
                Err(error) => events.send(Event::Failed {
                    id,
                    error,
                    bad_file: true,
                }),
            },
            Job::Picture { id, source, max_px } => {
                let picture = std::fs::read(&source.path)
                    .map_err(|e| format!("could not read file: {e}"))
                    .and_then(|bytes| guarded(|| picture::fit(&bytes, max_px)));
                match picture {
                    Ok(image) => events.send(Event::Picture {
                        id,
                        image: Arc::new(image),
                    }),
                    Err(error) => events.send(Event::Failed {
                        id,
                        error,
                        bad_file: true,
                    }),
                }
            }
            Job::Archive { id, source } => {
                match guarded(|| archive::list(&source.path).map_err(|e| e.to_string())) {
                    Ok(entries) => events.send(Event::Archive { id, entries }),
                    Err(error) => events.send(Event::Failed {
                        id,
                        error,
                        bad_file: true,
                    }),
                }
            }
            Job::Thumbnail {
                id,
                source,
                need_stats,
                style,
            } => {
                let cache_path = cache_dir.join(format!("{}.png", source.cache_key(&style)));
                let cached = std::fs::read(&cache_path).ok().and_then(|b| decode_png(&b));
                let format = Format::from_path(&source.path);
                if let (Some(image), false) = (&cached, need_stats) {
                    events.send(Event::Thumbnail {
                        id,
                        image: image.clone(),
                        style,
                    })
                } else if format == Some(Format::Image) {
                    let thumb = std::fs::read(&source.path)
                        .map_err(|e| format!("could not read file: {e}"))
                        .and_then(|bytes| guarded(|| picture::thumbnail(&bytes, style.size)));
                    match thumb {
                        Ok(image) => {
                            write_cache(&cache_path, &image);
                            events.send(Event::Thumbnail { id, image, style })
                        }
                        Err(error) => events.send(Event::Failed {
                            id,
                            error,
                            bad_file: true,
                        }),
                    }
                } else if format == Some(Format::Zip) {
                    match guarded(|| zip_cover(&source.path, style.size)) {
                        Ok(Cover::Image(image)) => {
                            write_cache(&cache_path, &image);
                            events.send(Event::Thumbnail { id, image, style })
                        }
                        Ok(Cover::Mesh(mesh)) => {
                            if render.send((id, mesh, cache_path, style)).is_err() {
                                let _ = events.send(Event::Failed {
                                    id,
                                    error: "thumbnail renderer is not running".into(),
                                    bad_file: false,
                                });
                            }
                            Ok(())
                        }
                        Ok(Cover::NotDownloaded) => events.send(Event::Failed {
                            id,
                            error: "this ZIP is online-only; download it to see a preview".into(),
                            bad_file: false,
                        }),
                        // The archive is fine, it just holds nothing to show.
                        Ok(Cover::Nothing) => events.send(Event::Failed {
                            id,
                            error: "nothing to preview in this archive".into(),
                            bad_file: false,
                        }),
                        Err(error) => events.send(Event::Failed {
                            id,
                            error,
                            bad_file: true,
                        }),
                    }
                } else {
                    match load_guarded(&source.path) {
                        Ok(mesh) => {
                            if let Some(stats) = mesh.stats() {
                                let _ = events.send(Event::Stats { id, stats });
                            }
                            match cached {
                                Some(image) => events.send(Event::Thumbnail { id, image, style }),
                                None => {
                                    if render.send((id, mesh, cache_path, style)).is_err() {
                                        // GPU thread is gone; nothing more we can do.
                                        let _ = events.send(Event::Failed {
                                            id,
                                            error: "thumbnail renderer is not running".into(),
                                            bad_file: false,
                                        });
                                    }
                                    Ok(())
                                }
                            }
                        }
                        Err(error) => events.send(Event::Failed {
                            id,
                            error,
                            bad_file: true,
                        }),
                    }
                }
            }
        };
        if sent.is_err() {
            return; // The service was dropped.
        }
    }
}

/// Runs , turning a panic into an error so one bad model can't take down a worker thread.
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err("internal error while processing this model".to_owned()))
}

fn load_guarded(path: &Path) -> Result<Mesh, String> {
    guarded(|| polyroom_formats::load(path).map_err(|e| e.to_string()))
}

/// What a ZIP's grid cell shows.
enum Cover {
    Image(Image),
    /// A model inside, to be rendered.
    Mesh(Mesh),
    /// Only a cloud placeholder is on disk; reading it would download the whole archive.
    NotDownloaded,
    Nothing,
}

/// Windows attributes of a cloud file (OneDrive "online-only") whose data isn't on disk:
/// `FILE_ATTRIBUTE_OFFLINE`, `FILE_ATTRIBUTE_RECALL_ON_OPEN`, `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`.
const ONLINE_ONLY_ATTRIBUTES: u32 = 0x1000 | 0x4_0000 | 0x40_0000;

fn is_online_only_attributes(attributes: u32) -> bool {
    attributes & ONLINE_ONLY_ATTRIBUTES != 0
}

/// Whether `path` is a cloud placeholder that reading would first download.
fn is_online_only(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        std::fs::metadata(path).is_ok_and(|m| is_online_only_attributes(m.file_attributes()))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Picks a ZIP's cover: the first image inside, else the first model inside that loads.
/// Only an unreadable archive is an error; bad files inside it are skipped.
fn zip_cover(path: &Path, size: u32) -> Result<Cover, String> {
    if is_online_only(path) {
        return Ok(Cover::NotDownloaded);
    }
    let entries = archive::list(path).map_err(|e| e.to_string())?;
    let read = |e: &ArchiveEntry, max: u64| archive::read_entry(path, &e.name, max).ok();
    for e in entries
        .iter()
        .filter(|e| e.format == Some(Format::Image) && e.size <= MAX_ZIP_IMAGE)
    {
        if let Some(image) = read(e, MAX_ZIP_IMAGE).and_then(|b| picture::thumbnail(&b, size).ok())
        {
            return Ok(Cover::Image(image));
        }
    }
    for e in entries
        .iter()
        .filter(|e| e.format.is_some_and(Format::is_mesh) && e.size <= MAX_ZIP_MESH)
        .take(ZIP_MESH_TRIES)
    {
        let mesh = e
            .format
            .zip(read(e, MAX_ZIP_MESH))
            .and_then(|(format, bytes)| polyroom_formats::load_bytes(format, &bytes).ok());
        if let Some(mesh) = mesh {
            return Ok(Cover::Mesh(mesh));
        }
    }
    Ok(Cover::Nothing)
}

fn write_cache(cache_path: &Path, image: &Image) {
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match encode_png(image) {
        Some(png) => {
            if let Err(e) = std::fs::write(cache_path, png) {
                log::warn!("could not cache thumbnail {}: {e}", cache_path.display());
            }
        }
        None => log::warn!("could not encode thumbnail"),
    }
}

/// A mesh waiting for the GPU thread: its job, the cache file to write and the look to render.
type RenderJob = (JobId, Mesh, PathBuf, ThumbStyle);

fn gpu_thread(jobs: &Receiver<RenderJob>, events: &Sender<Event>) {
    let gpu = match Headless::new() {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            log::error!("thumbnails disabled: {e}");
            None
        }
    };
    for (id, mesh, cache_path, style) in jobs {
        let result = match &gpu {
            Some(gpu) => guarded(|| {
                gpu.thumbnail_styled(&mesh, style.size, &style.render_style())
                    .map_err(|e| e.to_string())
            }),
            None => Err("no GPU available for thumbnails".to_owned()),
        };
        drop(mesh);
        let event = match result {
            Ok(image) => {
                write_cache(&cache_path, &image);
                Event::Thumbnail { id, image, style }
            }
            Err(error) => Event::Failed {
                id,
                error,
                bad_file: false,
            },
        };
        if events.send(event).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn write_cube_stl(path: &Path) {
        let cube = polyroom_mesh::test_cube();
        let mut out = vec![0u8; 80];
        out.extend_from_slice(&(cube.triangle_count() as u32).to_le_bytes());
        for t in cube.triangles() {
            out.extend_from_slice(&[0u8; 12]);
            for v in t {
                for c in v.to_array() {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out.extend_from_slice(&[0, 0]);
        }
        std::fs::write(path, out).unwrap();
    }

    fn wait_for(service: &mut PreviewService, mut done: impl FnMut(&Event) -> bool) -> Vec<Event> {
        let start = Instant::now();
        let mut all = Vec::new();
        while start.elapsed() < Duration::from_secs(30) {
            for e in service.poll() {
                let finished = done(&e);
                all.push(e);
                if finished {
                    return all;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out; got {all:?}");
    }

    #[test]
    fn stats_mesh_and_failures_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let stl = dir.path().join("cube.stl");
        write_cube_stl(&stl);
        let bad = dir.path().join("bad.3mf");
        std::fs::write(&bad, b"not a zip").unwrap();

        let mut s = PreviewService::start(dir.path().join("cache"));
        let source = |p: &Path| Source {
            path: p.to_owned(),
            size: 1,
            modified: None,
        };

        s.request_mesh(7, source(&stl));
        let events = wait_for(&mut s, |e| matches!(e, Event::Mesh { id: 7, .. }));
        assert!(
            matches!(events.last(), Some(Event::Mesh { mesh, .. }) if mesh.triangle_count() == 12)
        );

        s.request_thumbnail(8, source(&bad), true);
        wait_for(&mut s, |e| matches!(e, Event::Failed { id: 8, .. }));
        assert!(!s.is_thumbnail_pending(8));

        // Stats arrive even if the GPU is unavailable on this machine.
        s.request_thumbnail(9, source(&stl), true);
        let events = wait_for(&mut s, |e| {
            matches!(
                e,
                Event::Thumbnail { id: 9, .. } | Event::Failed { id: 9, .. }
            )
        });
        assert!(events.iter().any(
            |e| matches!(e, Event::Stats { id: 9, stats } if (stats.volume - 1000.0).abs() < 1e-3)
        ));
    }

    fn zip_with(path: &Path, files: &[(&str, &[u8])]) {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    fn red_png(w: u32, h: u32) -> Vec<u8> {
        encode_png(&Image {
            width: w,
            height: h,
            rgba: [255, 0, 0, 255].repeat((w * h) as usize),
        })
        .unwrap()
    }

    #[test]
    fn online_only_cloud_files_are_recognised() {
        // An online-only OneDrive file reports RECALL_ON_DATA_ACCESS (plus ARCHIVE, 0x20).
        assert!(is_online_only_attributes(0x40_0020));
        assert!(is_online_only_attributes(0x1000));
        // "Always keep on this device" / locally available files don't.
        assert!(!is_online_only_attributes(0x20));
        assert!(!is_online_only_attributes(0x8_0020)); // PINNED
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("local.zip");
        std::fs::write(&local, b"x").unwrap();
        assert!(!is_online_only(&local));
    }

    #[test]
    fn images_and_archives_get_thumbnails_pictures_and_listings() {
        let dir = tempfile::tempdir().unwrap();
        let photo = dir.path().join("photo.png");
        std::fs::write(&photo, red_png(300, 200)).unwrap();
        let with_image = dir.path().join("with image.zip");
        zip_with(
            &with_image,
            &[("README.txt", b"hi"), ("images/a.png", &red_png(8, 8))],
        );
        let empty = dir.path().join("only text.zip");
        zip_with(&empty, &[("README.txt", b"hi")]);
        let broken = dir.path().join("broken.zip");
        std::fs::write(&broken, b"not a zip").unwrap();

        let mut s = PreviewService::start(dir.path().join("cache"));
        let source = |p: &Path| Source {
            path: p.to_owned(),
            size: 1,
            modified: None,
        };
        let finished = |id: JobId| move |e: &Event| matches!(e, Event::Thumbnail { id: i, .. } | Event::Failed { id: i, .. } if *i == id);

        s.request_thumbnail(1, source(&photo), false);
        let events = wait_for(&mut s, finished(1));
        assert!(
            matches!(events.last(), Some(Event::Thumbnail { image, .. }) if image.width == THUMB_PX && image.height == THUMB_PX),
            "{events:?}"
        );

        s.request_thumbnail(2, source(&with_image), false);
        let events = wait_for(&mut s, finished(2));
        assert!(
            matches!(events.last(), Some(Event::Thumbnail { .. })),
            "{events:?}"
        );

        s.request_thumbnail(3, source(&empty), false);
        let events = wait_for(&mut s, finished(3));
        assert!(
            matches!(
                events.last(),
                Some(Event::Failed {
                    bad_file: false,
                    ..
                })
            ),
            "{events:?}"
        );

        s.request_thumbnail(4, source(&broken), false);
        let events = wait_for(&mut s, finished(4));
        assert!(
            matches!(events.last(), Some(Event::Failed { bad_file: true, .. })),
            "{events:?}"
        );

        s.request_picture(5, source(&photo), 150);
        let events = wait_for(&mut s, |e| matches!(e, Event::Picture { id: 5, .. }));
        assert!(
            matches!(events.last(), Some(Event::Picture { image, .. }) if (image.width, image.height) == (150, 100))
        );

        s.request_archive(6, source(&with_image));
        let events = wait_for(&mut s, |e| matches!(e, Event::Archive { id: 6, .. }));
        assert!(
            matches!(events.last(), Some(Event::Archive { entries, .. }) if entries.len() == 2)
        );
        assert!(!s.is_busy());
    }

    fn some_source() -> Source {
        Source {
            path: PathBuf::from("C:/m/a.stl"),
            size: 1,
            modified: None,
        }
    }

    #[test]
    fn every_style_has_its_own_cache_file() {
        let source = some_source();
        let base = ThumbStyle::default();
        let key = |style: ThumbStyle| source.cache_key(&style);
        let keys = [
            key(base),
            key(ThumbStyle { size: 512, ..base }),
            key(ThumbStyle {
                background: Some([0, 0, 0]),
                ..base
            }),
            key(ThumbStyle {
                model_color: [255, 128, 0],
                ..base
            }),
            key(ThumbStyle {
                generation: 1,
                ..base
            }),
        ];
        for (i, a) in keys.iter().enumerate() {
            for b in keys.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
        assert_eq!(key(base), key(ThumbStyle::default()), "keys are stable");
    }

    #[test]
    fn the_render_style_follows_the_thumb_style() {
        let style = ThumbStyle {
            background: Some([255, 0, 0]),
            model_color: [255, 255, 255],
            ..ThumbStyle::default()
        };
        let render = style.render_style();
        assert_eq!(render.background, [1.0, 0.0, 0.0, 1.0]);
        assert!(render.base_color.iter().all(|c| (c - 1.0).abs() < 1e-3));
        assert_eq!(ThumbStyle::default().render_style().background, [0.0; 4]);
    }

    #[test]
    fn forgetting_a_thumbnail_deletes_only_its_cache_file() {
        let dir = tempfile::tempdir().unwrap();
        let s = PreviewService::start(dir.path().to_owned());
        let (mine, other) = (
            some_source(),
            Source {
                size: 2,
                ..some_source()
            },
        );
        let file = |src: &Source| {
            dir.path()
                .join(format!("{}.png", src.cache_key(&s.style())))
        };
        std::fs::write(file(&mine), b"x").unwrap();
        std::fs::write(file(&other), b"x").unwrap();
        s.forget_cached(&mine);
        assert!(!file(&mine).exists() && file(&other).exists());
        s.forget_cached(&mine); // already gone: no error
    }

    #[test]
    fn purging_removes_other_generations_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = PreviewService::start(dir.path().to_owned());
        s.set_style(ThumbStyle {
            generation: 3,
            ..ThumbStyle::default()
        });
        for name in ["old.png", "g2-abc.png", "g3-keep.png", "notes.txt"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        s.purge_old_generations();
        let deadline = Instant::now() + Duration::from_secs(5);
        while dir.path().join("g2-abc.png").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let left = |n: &str| dir.path().join(n).exists();
        assert!(!left("old.png") && !left("g2-abc.png"));
        assert!(left("g3-keep.png") && left("notes.txt"));
    }

    #[test]
    fn a_new_style_drops_queued_thumbnails_and_makes_new_ones() {
        let dir = tempfile::tempdir().unwrap();
        let photo = dir.path().join("a.png");
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, 8, 8);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[255u8; 8 * 8 * 4]).unwrap();
        }
        std::fs::write(&photo, &png).unwrap();
        let mut s = PreviewService::start(dir.path().join("cache"));
        s.set_style(ThumbStyle {
            size: 512,
            ..ThumbStyle::default()
        });
        let source = Source {
            path: photo,
            size: png.len() as u64,
            modified: None,
        };
        s.request_thumbnail(1, source, false);
        let events = wait_for(&mut s, |e| {
            matches!(e, Event::Thumbnail { .. } | Event::Failed { .. })
        });
        assert!(
            matches!(events.last(), Some(Event::Thumbnail { image, .. }) if image.width == 512),
            "{events:?}"
        );
    }
}
