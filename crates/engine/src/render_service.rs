//! Background rendering of stage tiles and frames, with a memory-bounded cache.
//!
//! The stage is drawn from **tiles** (ADR 0007): fixed-size pieces of the stage at a zoom level,
//! each a [`FrameJob`]. The UI asks for the tiles it needs as one **batch**. The worker takes the
//! whole batch, evaluates the scene **once per (state, frame, onion)** (the job's `scene`), and
//! renders the tiles in parallel. Newer batches replace older pending ones (latest wins), so a
//! drag or a fast pan never builds a backlog. Jobs carry a `Document` clone, which is O(layers)
//! thanks to structural sharing. Keys hash the document *state id*, so undoing back to a state
//! already rendered is a cache hit.

use crate::history::StateId;
use anicraft_doc::{ColorEffect, DisplayList, Document, Rgba, evaluate};
use anicraft_render::{RenderStats, View, render};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

/// One image to render: a whole frame or one stage tile.
#[derive(Clone)]
pub struct FrameJob {
    pub key: u64,
    /// Identity of the evaluated scene (`scene_key`): tiles of one frame share it.
    pub scene: u64,
    pub doc: Document,
    pub frame: u32,
    pub view: View,
    pub onion: bool,
}

/// A finished image: premultiplied RGBA8, ready for a texture upload.
pub struct FrameImage {
    pub key: u64,
    pub frame: u32,
    pub width: u32,
    pub height: u32,
    pub rgba_premul: Vec<u8>,
    pub ms: f32,
    pub stats: RenderStats,
    pub error: Option<String>,
}

/// Identity of an evaluated scene: the display list depends on these and nothing else.
pub fn scene_key(state: StateId, frame: u32, onion: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (state, frame, onion).hash(&mut h);
    h.finish()
}

/// Cache key for a scene seen through a view (a frame or a tile).
pub fn frame_key(state: StateId, frame: u32, view: &View, onion: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    state.hash(&mut h);
    frame.hash(&mut h);
    for v in view.transform.m {
        v.to_bits().hash(&mut h);
    }
    view.width.hash(&mut h);
    view.height.hash(&mut h);
    if let Some(bg) = view.background {
        [bg.r, bg.g, bg.b, bg.a].map(f32::to_bits).hash(&mut h);
    }
    onion.hash(&mut h);
    h.finish()
}

/// A job whose `scene` is derived from the same inputs as its key.
pub fn make_job(state: StateId, doc: &Document, frame: u32, view: View, onion: bool) -> FrameJob {
    FrameJob { key: frame_key(state, frame, &view, onion), scene: scene_key(state, frame, onion), doc: doc.clone(), frame, view, onion }
}

/// The frame's display list, with ±2 onion-skin frames (tinted red before, green after) underneath.
pub fn display_list_with_onion(doc: &Document, frame: u32, onion: bool) -> DisplayList {
    let mut dl = DisplayList::default();
    if onion {
        let n = i64::from(doc.frame_count());
        for (off, tint, alpha) in [(-2i64, 0xff5a5a, 0.12f32), (2, 0x4cd07d, 0.12), (-1, 0xff5a5a, 0.25), (1, 0x4cd07d, 0.25)] {
            let f = i64::from(frame) + off;
            if f < 0 || f >= n {
                continue;
            }
            let mut ghost = evaluate(doc, f as u32);
            for item in &mut ghost.items {
                item.color = item.color.concat(&ColorEffect { alpha, tint: Rgba::hex(tint), tint_amount: 1.0 });
            }
            dl.append(ghost);
        }
    }
    dl.append(evaluate(doc, frame));
    dl
}

/// Renders a job against an already evaluated scene.
pub fn render_job_with(job: &FrameJob, dl: &DisplayList) -> FrameImage {
    let t0 = Instant::now();
    // Last-resort guard: a renderer bug must not take the editor down with it.
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(dl, &job.view)));
    let (rgba, stats, error) = match out {
        Ok(Ok((pm, stats))) => (pm.to_premul_rgba8(), stats, None),
        Ok(Err(e)) => (Vec::new(), RenderStats::default(), Some(e.to_string())),
        Err(_) => (Vec::new(), RenderStats::default(), Some("the renderer failed on this frame".into())),
    };
    let (width, height) = if error.is_none() { (job.view.width, job.view.height) } else { (0, 0) };
    FrameImage { key: job.key, frame: job.frame, width, height, rgba_premul: rgba, ms: t0.elapsed().as_secs_f32() * 1000.0, stats, error }
}

/// Renders a job synchronously, evaluating its scene.
pub fn render_job(job: &FrameJob) -> FrameImage {
    render_job_with(job, &display_list_with_onion(&job.doc, job.frame, job.onion))
}

/// Renders a batch: one scene evaluation per distinct `scene`, then all images in parallel.
pub fn render_batch(jobs: &[FrameJob]) -> Vec<FrameImage> {
    let mut scenes: HashMap<u64, Arc<DisplayList>> = HashMap::new();
    for j in jobs {
        scenes.entry(j.scene).or_insert_with(|| Arc::new(display_list_with_onion(&j.doc, j.frame, j.onion)));
    }
    jobs.par_iter()
        .map(|j| match scenes.get(&j.scene) {
            Some(dl) => render_job_with(j, dl),
            None => render_job(j),
        })
        .collect()
}

/// LRU cache of finished images, bounded by bytes.
pub struct FrameCache {
    map: HashMap<u64, (Arc<FrameImage>, u64)>,
    tick: u64,
    bytes: usize,
    pub budget_bytes: usize,
    pub hits: u64,
    pub misses: u64,
}

impl FrameCache {
    pub fn new(budget_bytes: usize) -> Self {
        FrameCache { map: HashMap::new(), tick: 0, bytes: 0, budget_bytes, hits: 0, misses: 0 }
    }
    pub fn get(&mut self, key: u64) -> Option<Arc<FrameImage>> {
        self.tick += 1;
        let tick = self.tick;
        match self.map.get_mut(&key) {
            Some((img, t)) => {
                *t = tick;
                self.hits += 1;
                Some(img.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }
    pub fn contains(&self, key: u64) -> bool {
        self.map.contains_key(&key)
    }
    pub fn insert(&mut self, img: Arc<FrameImage>) {
        self.tick += 1;
        let size = img.rgba_premul.len() + std::mem::size_of::<FrameImage>();
        if let Some((old, _)) = self.map.insert(img.key, (img, self.tick)) {
            self.bytes = self.bytes.saturating_sub(old.rgba_premul.len() + std::mem::size_of::<FrameImage>());
        }
        self.bytes += size;
        while self.bytes > self.budget_bytes && self.map.len() > 1 {
            let Some((&victim, _)) = self.map.iter().min_by_key(|(_, (_, t))| *t) else { break };
            if let Some((old, _)) = self.map.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(old.rgba_premul.len() + std::mem::size_of::<FrameImage>());
            }
        }
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[derive(Default)]
struct Queue {
    /// What the screen needs now (replaced by each new batch).
    urgent: VecDeque<FrameJob>,
    /// Look-ahead: margin tiles and upcoming playback frames (replaced wholesale).
    prefetch: VecDeque<FrameJob>,
    busy: HashSet<u64>,
    shutdown: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    cache: Mutex<FrameCache>,
    notify: Option<Box<dyn Fn() + Send + Sync>>,
}

/// Default image-cache budget.
pub const DEFAULT_CACHE_BYTES: usize = 768 * 1024 * 1024;
/// Most jobs the worker renders in one parallel pass.
const BATCH: usize = 32;

/// A render thread plus its cache.
pub struct RenderService {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl RenderService {
    /// Starts the worker. `notify` runs on the worker after each finished batch (the UI passes
    /// "request a repaint"). If no thread can be spawned, requests render synchronously.
    pub fn new(cache_bytes: usize, notify: Option<Box<dyn Fn() + Send + Sync>>) -> RenderService {
        let shared = Arc::new(Shared { queue: Mutex::new(Queue::default()), wake: Condvar::new(), cache: Mutex::new(FrameCache::new(cache_bytes)), notify });
        let s = shared.clone();
        let worker = std::thread::Builder::new().name("anicraft-render".into()).spawn(move || worker_loop(&s)).ok();
        RenderService { shared, worker }
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, FrameCache> {
        self.shared.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Images that are ready, in order; the missing ones become the new urgent batch
    /// (replacing any older pending batch).
    pub fn request_batch(&self, jobs: Vec<FrameJob>) -> Vec<Option<Arc<FrameImage>>> {
        let mut out = Vec::with_capacity(jobs.len());
        let mut missing = Vec::new();
        {
            let mut cache = self.cache();
            for j in jobs {
                match cache.get(j.key) {
                    Some(img) => out.push(Some(img)),
                    None => {
                        out.push(None);
                        missing.push(j);
                    }
                }
            }
        }
        if missing.is_empty() {
            return out;
        }
        if self.worker.is_none() {
            // No thread available: render here, so the caller still gets pictures.
            let imgs: Vec<Arc<FrameImage>> = render_batch(&missing).into_iter().map(Arc::new).collect();
            let mut cache = self.cache();
            for img in &imgs {
                cache.insert(img.clone());
            }
            let mut it = imgs.into_iter();
            for slot in out.iter_mut().filter(|s| s.is_none()) {
                *slot = it.next();
            }
            return out;
        }
        let mut q = self.queue();
        let busy = q.busy.clone();
        q.urgent = missing.into_iter().filter(|j| !busy.contains(&j.key)).collect();
        if !q.urgent.is_empty() {
            self.shared.wake.notify_one();
        }
        out
    }

    /// Single-image form of [`RenderService::request_batch`].
    pub fn request(&self, job: FrameJob) -> Option<Arc<FrameImage>> {
        self.request_batch(vec![job]).into_iter().next().flatten()
    }

    /// Renders on the calling thread (one scene evaluation, tiles in parallel) and caches the
    /// results: for edits, where showing the change in this frame matters most.
    pub fn render_now_batch(&self, jobs: Vec<FrameJob>) -> Vec<Arc<FrameImage>> {
        let (mut out, missing): (Vec<Option<Arc<FrameImage>>>, Vec<FrameJob>) = {
            let mut cache = self.cache();
            let mut missing = Vec::new();
            let out = jobs
                .into_iter()
                .map(|j| {
                    let hit = cache.get(j.key);
                    if hit.is_none() {
                        missing.push(j);
                    }
                    hit
                })
                .collect();
            (out, missing)
        };
        let fresh: Vec<Arc<FrameImage>> = render_batch(&missing).into_iter().map(Arc::new).collect();
        {
            let mut cache = self.cache();
            for img in &fresh {
                cache.insert(img.clone());
            }
        }
        let mut it = fresh.into_iter();
        for slot in out.iter_mut().filter(|s| s.is_none()) {
            *slot = it.next();
        }
        out.into_iter().flatten().collect()
    }

    /// Single-image form of [`RenderService::render_now_batch`].
    pub fn render_now(&self, job: FrameJob) -> Arc<FrameImage> {
        let key = job.key;
        let frame = job.frame;
        self.render_now_batch(vec![job]).into_iter().next().unwrap_or_else(|| {
            Arc::new(FrameImage { key, frame, width: 0, height: 0, rgba_premul: Vec::new(), ms: 0.0, stats: RenderStats::default(), error: Some("render failed".into()) })
        })
    }

    /// Replaces the look-ahead queue (images already cached are skipped).
    pub fn prefetch(&self, jobs: Vec<FrameJob>) {
        if self.worker.is_none() {
            return;
        }
        let jobs: VecDeque<FrameJob> = {
            let cache = self.cache();
            jobs.into_iter().filter(|j| !cache.contains(j.key)).collect()
        };
        let mut q = self.queue();
        q.prefetch = jobs;
        if !q.prefetch.is_empty() {
            self.shared.wake.notify_one();
        }
    }

    /// Whether the worker has queued or in-flight work.
    pub fn busy(&self) -> bool {
        let q = self.queue();
        !q.busy.is_empty() || !q.urgent.is_empty() || !q.prefetch.is_empty()
    }

    /// `(images, bytes, hits, misses)`.
    pub fn cache_stats(&self) -> (usize, usize, u64, u64) {
        let c = self.cache();
        (c.len(), c.bytes(), c.hits, c.misses)
    }

    /// Blocks until `key` is rendered (tests, headless tools). Times out after `ms`.
    pub fn wait_for(&self, key: u64, ms: u64) -> Option<Arc<FrameImage>> {
        let deadline = Instant::now() + std::time::Duration::from_millis(ms);
        while Instant::now() < deadline {
            if let Some(img) = self.cache().get(key) {
                return Some(img);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        None
    }
}

impl Drop for RenderService {
    fn drop(&mut self) {
        {
            let mut q = self.queue();
            q.shutdown = true;
            q.urgent.clear();
            q.prefetch.clear();
        }
        self.shared.wake.notify_all();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

fn worker_loop(s: &Shared) {
    loop {
        // Take a whole batch: urgent first, topped up with prefetch work.
        let jobs: Vec<FrameJob> = {
            let mut q = s.queue.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if q.shutdown {
                    return;
                }
                if !q.urgent.is_empty() || !q.prefetch.is_empty() {
                    let mut batch = Vec::new();
                    while batch.len() < BATCH {
                        let Some(j) = q.urgent.pop_front().or_else(|| q.prefetch.pop_front()) else { break };
                        batch.push(j);
                    }
                    for j in &batch {
                        q.busy.insert(j.key);
                    }
                    break batch;
                }
                q = s.wake.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let todo: Vec<FrameJob> = {
            let cache = s.cache.lock().unwrap_or_else(PoisonError::into_inner);
            jobs.iter().filter(|j| !cache.contains(j.key)).cloned().collect()
        };
        let imgs = render_batch(&todo);
        {
            let mut cache = s.cache.lock().unwrap_or_else(PoisonError::into_inner);
            for img in imgs {
                cache.insert(Arc::new(img));
            }
        }
        {
            let mut q = s.queue.lock().unwrap_or_else(PoisonError::into_inner);
            for j in &jobs {
                q.busy.remove(&j.key);
            }
        }
        if let Some(n) = &s.notify {
            n();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::sample;
    use anicraft_geom::Affine;

    fn job(doc: &Document, state: u64, frame: u32) -> FrameJob {
        let view = View { transform: Affine::scale(0.25, 0.25), width: 320, height: 180, background: Some(doc.stage.background) };
        make_job(state, doc, frame, view, false)
    }

    #[test]
    fn worker_matches_synchronous_render() {
        let doc = sample::demo();
        let svc = RenderService::new(64 << 20, None);
        let j = job(&doc, 1, 9);
        let direct = render_job(&j);
        assert!(svc.request(j.clone()).is_none(), "first request is queued");
        let img = svc.wait_for(j.key, 10_000).unwrap();
        assert_eq!(img.rgba_premul, direct.rgba_premul);
        // second request is a cache hit
        assert!(svc.request(j).is_some());
    }

    #[test]
    fn render_now_is_synchronous_and_cached() {
        let doc = sample::demo();
        let svc = RenderService::new(64 << 20, None);
        let j = job(&doc, 3, 4);
        let img = svc.render_now(j.clone());
        assert_eq!(img.rgba_premul, render_job(&j).rgba_premul);
        assert!(svc.request(j).is_some(), "available to later requests without re-rendering");
    }

    /// Tiles rendered separately and stitched match one full render to within 1/255 per
    /// channel (float rounding differs at anti-aliased edges when coordinates are offset), with
    /// differences rare: no visible seams. Each tile itself is deterministic.
    #[test]
    fn stitched_tiles_match_a_full_render() {
        let doc = sample::demo();
        let z = 0.37f32; // deliberately odd zoom
        let (w, h) = ((1280.0 * z).ceil() as u32, (720.0 * z).ceil() as u32);
        let bg = Some(doc.stage.background);
        let full = render_job(&make_job(1, &doc, 13, View { transform: Affine::scale(z, z), width: w, height: h, background: bg }, false));
        let t = 128u32;
        let mut jobs = Vec::new();
        for ty in 0..h.div_ceil(t) {
            for tx in 0..w.div_ceil(t) {
                let view = View {
                    transform: Affine::translate(-((tx * t) as f32), -((ty * t) as f32)).mul(&Affine::scale(z, z)),
                    width: t.min(w - tx * t),
                    height: t.min(h - ty * t),
                    background: bg,
                };
                jobs.push((tx, ty, make_job(1, &doc, 13, view, false)));
            }
        }
        let batch: Vec<FrameJob> = jobs.iter().map(|(_, _, j)| j.clone()).collect();
        let imgs = render_batch(&batch);
        assert!(jobs.windows(2).all(|p| p[0].2.scene == p[1].2.scene), "one scene for all tiles");
        let (mut differing, mut total) = (0usize, 0usize);
        for ((tx, ty, _), img) in jobs.iter().zip(&imgs) {
            for y in 0..img.height {
                for x in 0..img.width {
                    let (gx, gy) = (tx * t + x, ty * t + y);
                    let a = ((y * img.width + x) * 4) as usize;
                    let b = ((gy * w + gx) * 4) as usize;
                    let (p, q) = (&img.rgba_premul[a..a + 4], &full.rgba_premul[b..b + 4]);
                    let d = p.iter().zip(q).map(|(u, v)| u.abs_diff(*v)).max().unwrap_or(0);
                    assert!(d <= 1, "tile ({tx},{ty}) px ({x},{y}): {p:?} vs {q:?}");
                    differing += usize::from(d > 0);
                    total += 1;
                }
            }
        }
        assert!(differing * 200 < total, "{differing} of {total} pixels differ (by 1/255)");
        // determinism: the same tile twice gives the same bytes
        assert_eq!(render_batch(&batch[..1])[0].rgba_premul, imgs[0].rgba_premul);
    }

    #[test]
    fn batches_replace_pending_work_and_complete() {
        let doc = sample::demo();
        let svc = RenderService::new(64 << 20, None);
        let first: Vec<FrameJob> = (0..8).map(|f| job(&doc, 7, f)).collect();
        let second: Vec<FrameJob> = (20..24).map(|f| job(&doc, 7, f)).collect();
        let keys: Vec<u64> = second.iter().map(|j| j.key).collect();
        let _ = svc.request_batch(first);
        let r = svc.request_batch(second);
        assert!(r.iter().all(Option::is_none));
        for k in keys {
            assert!(svc.wait_for(k, 10_000).is_some(), "the latest batch is always rendered");
        }
    }

    #[test]
    fn prefetch_fills_cache() {
        let doc = sample::demo();
        let svc = RenderService::new(64 << 20, None);
        let jobs: Vec<FrameJob> = (0..6).map(|f| job(&doc, 2, f)).collect();
        let keys: Vec<u64> = jobs.iter().map(|j| j.key).collect();
        svc.prefetch(jobs);
        for k in keys {
            assert!(svc.wait_for(k, 10_000).is_some());
        }
    }

    #[test]
    fn cache_respects_budget_lru() {
        let mut c = FrameCache::new(3 * 1000);
        for k in 0..10u64 {
            c.insert(Arc::new(FrameImage { key: k, frame: 0, width: 0, height: 0, rgba_premul: vec![0; 800], ms: 0.0, stats: RenderStats::default(), error: None }));
        }
        assert!(c.bytes() <= 3000);
        assert!(c.contains(9) && !c.contains(0));
    }

    #[test]
    fn keys_distinguish_state_frame_and_view() {
        let doc = sample::demo();
        let a = job(&doc, 1, 0).key;
        assert_ne!(a, job(&doc, 2, 0).key);
        assert_ne!(a, job(&doc, 1, 1).key);
        assert_eq!(a, job(&doc, 1, 0).key);
    }

    #[test]
    fn onion_adds_neighbour_frames() {
        let doc = sample::demo();
        let plain = display_list_with_onion(&doc, 10, false).items.len();
        let onion = display_list_with_onion(&doc, 10, true).items.len();
        assert_eq!(onion, plain * 5);
    }
}
