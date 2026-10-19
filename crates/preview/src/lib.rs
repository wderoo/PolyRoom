//! Background work for previews: parsing models, computing stats and rendering thumbnails.
//!
//! A few CPU workers parse files; one GPU thread renders thumbnails with its own wgpu device.
//! Rendered thumbnails are cached as PNGs keyed by path, size, modification time and renderer
//! version, so they survive restarts. Results come back as [`Event`]s, polled by the engine.

#![forbid(unsafe_code)]

mod cache;

use std::collections::{HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;

use polyroom_mesh::{Mesh, MeshStats};
use polyroom_render::{Headless, Image};

pub use cache::{decode_png, encode_png};

/// Thumbnail edge length in pixels.
pub const THUMB_PX: u32 = 256;
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

impl Source {
    fn cache_key(&self) -> String {
        let mut h = DefaultHasher::new();
        (self, RENDER_VERSION, THUMB_PX).hash(&mut h);
        format!("{:016x}", h.finish())
    }
}

#[derive(Debug)]
pub enum Event {
    /// Stats from a freshly parsed model.
    Stats {
        id: JobId,
        stats: MeshStats,
    },
    Thumbnail {
        id: JobId,
        image: Image,
    },
    /// A full mesh for the loupe.
    Mesh {
        id: JobId,
        mesh: Arc<Mesh>,
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
    },
    Mesh {
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
    queue: SharedQueue,
    events: Receiver<Event>,
    pending_thumbs: HashSet<JobId>,
    pending_mesh: Option<JobId>,
}

impl PreviewService {
    /// Starts the workers. Thumbnails are cached under `cache_dir`.
    pub fn start(cache_dir: PathBuf) -> Self {
        let queue: SharedQueue = Arc::default();
        let (event_tx, events) = mpsc::channel();
        // Bounded so parsers can't run far ahead of the GPU and pile up meshes in memory.
        let (render_tx, render_rx) = mpsc::sync_channel::<(JobId, Mesh, PathBuf)>(2);

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
            queue,
            events,
            pending_thumbs: HashSet::new(),
            pending_mesh: None,
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

    /// Queues a thumbnail unless one is already pending for `id`.
    pub fn request_thumbnail(&mut self, id: JobId, source: Source, need_stats: bool) {
        if self.pending_thumbs.insert(id) {
            self.push(
                Job::Thumbnail {
                    id,
                    source,
                    need_stats,
                },
                false,
            );
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
        !self.pending_thumbs.is_empty() || self.pending_mesh.is_some()
    }

    /// Takes all finished results.
    pub fn poll(&mut self) -> Vec<Event> {
        let events: Vec<Event> = self.events.try_iter().collect();
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
                Event::Failed { id, .. } => {
                    self.pending_thumbs.remove(id);
                    if self.pending_mesh == Some(*id) {
                        self.pending_mesh = None;
                    }
                }
                Event::Stats { .. } => {}
            }
        }
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
    render: &SyncSender<(JobId, Mesh, PathBuf)>,
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
            Job::Thumbnail {
                id,
                source,
                need_stats,
            } => {
                let cache_path = cache_dir.join(format!("{}.png", source.cache_key()));
                let cached = std::fs::read(&cache_path).ok().and_then(|b| decode_png(&b));
                if let (Some(image), false) = (&cached, need_stats) {
                    events.send(Event::Thumbnail {
                        id,
                        image: image.clone(),
                    })
                } else {
                    match load_guarded(&source.path) {
                        Ok(mesh) => {
                            if let Some(stats) = mesh.stats() {
                                let _ = events.send(Event::Stats { id, stats });
                            }
                            match cached {
                                Some(image) => events.send(Event::Thumbnail { id, image }),
                                None => {
                                    if render.send((id, mesh, cache_path)).is_err() {
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

fn gpu_thread(jobs: &Receiver<(JobId, Mesh, PathBuf)>, events: &Sender<Event>) {
    let gpu = match Headless::new() {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            log::error!("thumbnails disabled: {e}");
            None
        }
    };
    for (id, mesh, cache_path) in jobs {
        let result = match &gpu {
            Some(gpu) => guarded(|| gpu.thumbnail(&mesh, THUMB_PX).map_err(|e| e.to_string())),
            None => Err("no GPU available for thumbnails".to_owned()),
        };
        drop(mesh);
        let event = match result {
            Ok(image) => {
                if let Some(parent) = cache_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match encode_png(&image) {
                    Some(png) => {
                        if let Err(e) = std::fs::write(&cache_path, png) {
                            log::warn!("could not cache thumbnail {}: {e}", cache_path.display());
                        }
                    }
                    None => log::warn!("could not encode thumbnail"),
                }
                Event::Thumbnail { id, image }
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
}
