//! Background rendering of stage tiles and frames, with a memory-bounded cache.
//!
//! The stage is drawn from **tiles** (ADR 0007): fixed-size pieces of the stage at a zoom level,
//! each a [`FrameJob`]. The UI asks for the tiles it needs as one **batch**. The worker takes the
//! whole batch, evaluates the scene **once per (state, frame, onion)** (the job's `scene`), and
//! renders the tiles in parallel. Newer batches replace older pending ones (latest wins), so a
//! drag or a fast pan never builds a backlog. Jobs carry a `Document` clone, which is O(layers)
//! thanks to structural sharing. Keys hash the document *state id*, so undoing back to a state
//! already rendered is a cache hit.
//!
//! The CPU oracle draws by default. A [`Rasterizer`] (the GPU stage, ADR 0019) can take over;
//! whatever it hands back (content it doesn't draw yet, or a failed device) the oracle draws, so
//! the picture is always complete. Keys include which rasterizer drew them. A rasterizer may
//! keep its pictures on the GPU ([`Raster::Native`], ADR 0029): the cache then holds them, under
//! a budget of their own, and only the shell knows what they are.

use crate::editing::Front;
use crate::history::StateId;
use crate::onion::OnionSkin;
use anicraft_doc::{DisplayList, Document, evaluate_scene, evaluate_scene_camera};
use anicraft_render::{RenderStats, View, render};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use web_time::Instant;

/// One image to render: a whole frame or one stage tile.
#[derive(Clone)]
pub struct FrameJob {
    pub key: u64,
    /// Which rasterizer the key was made for (0: the CPU oracle); see [`RenderService::job`].
    pub raster: u64,
    /// Identity of the evaluated scene (`scene_key`): tiles of one frame share it.
    pub scene: u64,
    pub doc: Document,
    pub frame: u32,
    pub view: View,
    /// Onion skin or Edit Multiple Frames (ADR 0047); `None`: the frame alone.
    pub onion: Option<OnionSkin>,
    /// Seen through the scene's camera (View › Preview Through Camera, ADR 0027): what the
    /// movie shows, rather than the stage as edited.
    pub camera: bool,
    /// A symbol open in place, in front of the picture (ADR 0045): onion skin composes its own
    /// frames (ADR 0051). Set with [`FrameJob::inside`].
    pub front: Option<Arc<Front>>,
}

impl FrameJob {
    /// The same job seen through the scene's camera (`on`), keyed apart from the plain one.
    pub fn through_camera(mut self, on: bool) -> FrameJob {
        if on && !self.camera {
            let mix = |k: u64| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (k, "camera").hash(&mut h);
                h.finish()
            };
            self.key = mix(self.key);
            self.scene = mix(self.scene);
            self.camera = true;
        }
        self
    }

    /// The same job for a stage picture whose top layer is the open symbol `front` (from
    /// [`crate::editing::StagePicture::front`]): onion skin and Edit Multiple Frames then compose
    /// the symbol's frames, not the picture's one frame. Keyed apart from the plain job.
    pub fn inside(mut self, front: Option<Arc<Front>>) -> FrameJob {
        if let Some(f) = front {
            if self.onion.is_some() {
                let mix = |k: u64| {
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    (k, "front", f.layer.0).hash(&mut h);
                    h.finish()
                };
                self.key = mix(self.key);
                self.scene = mix(self.scene);
            }
            self.front = Some(f);
        }
        self
    }

    /// What the job draws: its frame with what its onion settings add (ADR 0047), on the open
    /// symbol's own timeline when there is one (ADR 0051).
    pub fn display_list(&self) -> DisplayList {
        let onion = self.onion.as_ref().filter(|o| o.shows());
        match (&self.front, onion) {
            (Some(front), Some(skin)) => display_list_inside(&self.doc, self.frame, front, skin),
            _ => display_list_for(&self.doc, self.frame, self.onion.as_ref(), self.camera),
        }
    }
}

/// A stage picture's frame with the open symbol in front composed by `skin`: the timelines around
/// it as they are, then the symbol's frames, each layer's onion frames under its artwork. With
/// onion skin on (not Edit Multiple Frames), the timeline the symbol was opened from shows its
/// own onion frames too, dimmed with it (ADR 0061).
pub fn display_list_inside(doc: &Document, frame: u32, front: &Front, skin: &OnionSkin) -> DisplayList {
    let (scene, local) = doc.locate(frame);
    let tl = &front.timeline;
    let parent = front.parent.as_ref().filter(|_| skin.ghosts && !skin.edit_multiple);
    let mut dl = DisplayList::default();
    // Back to front: the picture's layers from the bottom up, each its own timeline.
    for l in doc.timeline(scene).layers.iter().rev() {
        if l.id == front.layer {
            dl.append(crate::onion::compose_timeline(tl, front.frame, Some(skin), |f| anicraft_doc::evaluate_timeline_at(doc, tl, f, front.at)));
        } else if let Some(p) = parent.filter(|p| p.layer == l.id) {
            dl.append(parent_onion(doc, p, front, skin));
        } else {
            dl.append(anicraft_doc::evaluate_timeline(doc, &anicraft_doc::Timeline { layers: vec![l.clone()] }, local));
        }
    }
    dl
}

/// The timeline around an open symbol with its onion frames, all at its dim opacity. Markers that
/// follow the playhead reach as many frames from the parent's frame; anchored ones (which name
/// the symbol's frames) as far as they reach from the symbol's playhead.
fn parent_onion(doc: &Document, p: &crate::editing::Around, front: &Front, skin: &OnionSkin) -> DisplayList {
    let mut own = *skin;
    if skin.anchored() {
        let (first, last) = skin.range(front.frame, front.timeline.frame_count());
        own.markers = crate::onion::Markers::Relative { before: front.frame.saturating_sub(first), after: last.saturating_sub(front.frame) };
    }
    let composed = crate::onion::compose_timeline(&p.timeline, p.frame, Some(&own), |f| anicraft_doc::evaluate_timeline_at(doc, &p.timeline, f, p.at));
    // It fades as one, as the dimmed instance it replaces does: everything in a group at `dim`.
    let alpha = if p.dim.is_finite() { p.dim.clamp(0.0, 1.0) } else { 1.0 };
    let mut dl = DisplayList::default();
    dl.groups.push(anicraft_doc::Group { parent: None, alpha, blend: Default::default(), filters: Arc::from(Vec::new()), clip: None });
    dl.append(composed);
    for g in dl.groups.iter_mut().skip(1) {
        g.parent = Some(g.parent.unwrap_or(0));
    }
    for it in dl.items.iter_mut().filter(|i| i.masks.is_none()) {
        it.group = Some(it.group.unwrap_or(0));
    }
    dl
}

/// A finished image: premultiplied RGBA8, ready for a texture upload, or (`native`) an image
/// already on the GPU.
pub struct FrameImage {
    pub key: u64,
    pub frame: u32,
    pub width: u32,
    pub height: u32,
    /// Empty when the picture is [`FrameImage::native`].
    pub rgba_premul: Vec<u8>,
    /// The rasterizer's own GPU image (ADR 0029), shown as it is; `None` for bytes.
    pub native: Option<NativeImage>,
    pub ms: f32,
    pub stats: RenderStats,
    pub error: Option<String>,
    /// Drawn by the rasterizer rather than the CPU oracle.
    pub gpu: bool,
    /// Why the oracle drew it although a rasterizer is set (for example "masks").
    pub fallback: Option<String>,
}

/// A picture a rasterizer keeps on the GPU. The engine stays GPU-free: only the shell, which
/// set the rasterizer, knows the concrete type (and how to show it).
pub type NativeImage = Arc<dyn std::any::Any + Send + Sync>;

/// What a [`Rasterizer`] drew.
pub enum Raster {
    /// Premultiplied RGBA8 bytes, `width × height × 4` of them.
    Premul8(Vec<u8>),
    /// A `width × height` picture on the GPU (no bytes on the CPU).
    Native { image: NativeImage, width: u32, height: u32 },
}

/// Draws a display list in place of the CPU oracle, or says why it can't (then the oracle
/// draws it).
pub type Rasterizer = Arc<dyn Fn(&DisplayList, &View) -> Result<(Raster, RenderStats), String> + Send + Sync>;

/// What onion settings add to a key: nothing when they show nothing.
fn onion_key(onion: Option<&OnionSkin>) -> u64 {
    onion.filter(|o| o.shows()).map_or(0, OnionSkin::key)
}

/// Identity of an evaluated scene: the display list depends on these and nothing else.
pub fn scene_key(state: StateId, frame: u32, onion: Option<&OnionSkin>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (state, frame, onion_key(onion)).hash(&mut h);
    h.finish()
}

/// Cache key for a scene seen through a view (a frame or a tile).
pub fn frame_key(state: StateId, frame: u32, view: &View, onion: Option<&OnionSkin>) -> u64 {
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
    onion_key(onion).hash(&mut h);
    h.finish()
}

/// A job whose `scene` is derived from the same inputs as its key.
pub fn make_job(state: StateId, doc: &Document, frame: u32, view: View, onion: Option<&OnionSkin>) -> FrameJob {
    let onion = onion.filter(|o| o.shows()).copied();
    FrameJob { key: frame_key(state, frame, &view, onion.as_ref()), raster: 0, scene: scene_key(state, frame, onion.as_ref()), doc: doc.clone(), frame, view, onion, camera: false, front: None }
}

/// A job for rasterizer `raster` (from [`RenderService::raster_id`]; 0 is the CPU oracle, and
/// gives [`make_job`]'s key), keyed apart from other rasterizers' pictures.
pub fn make_job_for(raster: u64, state: StateId, doc: &Document, frame: u32, view: View, onion: Option<&OnionSkin>) -> FrameJob {
    let mut j = make_job(state, doc, frame, view, onion);
    if raster != 0 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (j.key, raster).hash(&mut h);
        j.key = h.finish();
        j.raster = raster;
    }
    j
}

/// Movie frame `frame`'s display list, with what `onion` adds underneath (onion frames or Edit
/// Multiple Frames' keyframes, ADR 0047), taken from the same scene only.
pub fn display_list_with_onion(doc: &Document, frame: u32, onion: Option<&OnionSkin>) -> DisplayList {
    display_list_for(doc, frame, onion, false)
}

/// [`display_list_with_onion`], seen through the scene's camera when `camera` (ADR 0027).
pub fn display_list_for(doc: &Document, frame: u32, onion: Option<&OnionSkin>, camera: bool) -> DisplayList {
    let (scene, local) = doc.locate(frame);
    crate::onion::compose(doc, scene, local, onion, |scene, f| if camera { evaluate_scene_camera(doc, scene, f) } else { evaluate_scene(doc, scene, f) })
}

/// Renders a job against an already evaluated scene, on the CPU oracle.
pub fn render_job_with(job: &FrameJob, dl: &DisplayList) -> FrameImage {
    render_job_by(job, dl, None)
}

/// Renders a job with `raster` when given (falling back to the oracle for what it hands back).
fn render_job_by(job: &FrameJob, dl: &DisplayList, raster: Option<&Rasterizer>) -> FrameImage {
    let t0 = Instant::now();
    let mut fallback = None;
    if let Some(r) = raster {
        // Last-resort guard: a renderer bug must not take the editor down with it.
        let (w, h) = (job.view.width, job.view.height);
        let done = |rgba_premul: Vec<u8>, native: Option<NativeImage>, stats: RenderStats| {
            let ms = t0.elapsed().as_secs_f32() * 1000.0;
            FrameImage { key: job.key, frame: job.frame, width: w, height: h, rgba_premul, native, ms, stats, error: None, gpu: true, fallback: None }
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r(dl, &job.view))) {
            Ok(Ok((Raster::Premul8(rgba), stats))) if rgba.len() == w as usize * h as usize * 4 => return done(rgba, None, stats),
            Ok(Ok((Raster::Native { image, width, height }, stats))) if (width, height) == (w, h) => return done(Vec::new(), Some(image), stats),
            Ok(Ok(_)) => fallback = Some("the GPU returned a wrong-sized image".to_string()),
            Ok(Err(why)) => fallback = Some(why),
            Err(_) => fallback = Some("the GPU renderer failed".to_string()),
        }
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(dl, &job.view)));
    let (rgba, stats, error) = match out {
        Ok(Ok((pm, stats))) => (pm.to_premul_rgba8(), stats, None),
        Ok(Err(e)) => (Vec::new(), RenderStats::default(), Some(e.to_string())),
        Err(_) => (Vec::new(), RenderStats::default(), Some("the renderer failed on this frame".into())),
    };
    let (width, height) = if error.is_none() { (job.view.width, job.view.height) } else { (0, 0) };
    FrameImage { key: job.key, frame: job.frame, width, height, rgba_premul: rgba, native: None, ms: t0.elapsed().as_secs_f32() * 1000.0, stats, error, gpu: false, fallback }
}

/// Renders a job synchronously, evaluating its scene.
pub fn render_job(job: &FrameJob) -> FrameImage {
    render_job_with(job, &job.display_list())
}

/// Renders a batch: one scene evaluation per distinct `scene`, then all images in parallel.
pub fn render_batch(jobs: &[FrameJob]) -> Vec<FrameImage> {
    render_batch_by(jobs, &RasterSlot::default())
}

/// [`render_batch`] with the service's rasterizer for the jobs made for it.
fn render_batch_by(jobs: &[FrameJob], slot: &RasterSlot) -> Vec<FrameImage> {
    let mut scenes: HashMap<u64, Arc<DisplayList>> = HashMap::new();
    for j in jobs {
        scenes.entry(j.scene).or_insert_with(|| Arc::new(j.display_list()));
    }
    jobs.par_iter()
        .map(|j| {
            // A job made for another rasterizer (queued before a switch) is drawn by the oracle.
            let raster = slot.raster.as_ref().filter(|_| j.raster != 0 && j.raster == slot.id);
            match scenes.get(&j.scene) {
                Some(dl) => render_job_by(j, dl, raster),
                None => render_job_by(j, &j.display_list(), raster),
            }
        })
        .collect()
}

/// The rasterizer in use and its id (0 and `None`: the oracle).
#[derive(Clone, Default)]
struct RasterSlot {
    id: u64,
    raster: Option<Rasterizer>,
}

/// LRU cache of finished images, bounded by bytes: those in memory, and (apart) those a
/// rasterizer keeps on the GPU, counted at 4 bytes a pixel.
pub struct FrameCache {
    map: HashMap<u64, (Arc<FrameImage>, u64)>,
    tick: u64,
    bytes: usize,
    native_bytes: usize,
    pub budget_bytes: usize,
    pub native_budget_bytes: usize,
    pub hits: u64,
    pub misses: u64,
}

/// `(bytes in memory, bytes on the GPU)` an image holds.
fn cost(img: &FrameImage) -> (usize, usize) {
    let gpu = if img.native.is_some() { img.width as usize * img.height as usize * 4 } else { 0 };
    (img.rgba_premul.len() + std::mem::size_of::<FrameImage>(), gpu)
}

impl FrameCache {
    pub fn new(budget_bytes: usize) -> Self {
        FrameCache { map: HashMap::new(), tick: 0, bytes: 0, native_bytes: 0, budget_bytes, native_budget_bytes: DEFAULT_NATIVE_CACHE_BYTES, hits: 0, misses: 0 }
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
        let (cpu, gpu) = cost(&img);
        if let Some((old, _)) = self.map.insert(img.key, (img, self.tick)) {
            self.forget(&old);
        }
        self.bytes += cpu;
        self.native_bytes += gpu;
        loop {
            // Evict the least recently used image of the kind that is over its budget.
            let over_gpu = self.native_bytes > self.native_budget_bytes;
            if !(over_gpu || self.bytes > self.budget_bytes) || self.map.len() <= 1 {
                break;
            }
            let victim = self.map.iter().filter(|(_, (i, _))| !over_gpu || i.native.is_some()).min_by_key(|(_, (_, t))| *t).map(|(&k, _)| k);
            let Some(victim) = victim else { break };
            if let Some((old, _)) = self.map.remove(&victim) {
                self.forget(&old);
            }
        }
    }

    fn forget(&mut self, img: &FrameImage) {
        let (cpu, gpu) = cost(img);
        self.bytes = self.bytes.saturating_sub(cpu);
        self.native_bytes = self.native_bytes.saturating_sub(gpu);
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
    /// Bytes of the images kept on the GPU.
    pub fn native_bytes(&self) -> usize {
        self.native_bytes
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
    raster: Mutex<RasterSlot>,
    wake: Condvar,
    cache: Mutex<FrameCache>,
    notify: Option<Box<dyn Fn() + Send + Sync>>,
}

/// Default image-cache budget.
pub const DEFAULT_CACHE_BYTES: usize = 768 * 1024 * 1024;
/// Default budget for pictures a rasterizer keeps on the GPU (graphics memory is scarcer).
pub const DEFAULT_NATIVE_CACHE_BYTES: usize = 384 * 1024 * 1024;
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
        let shared = Arc::new(Shared { queue: Mutex::new(Queue::default()), raster: Mutex::new(RasterSlot::default()), wake: Condvar::new(), cache: Mutex::new(FrameCache::new(cache_bytes)), notify });
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

    fn slot(&self) -> RasterSlot {
        self.shared.raster.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Draws with `raster` from now on (`None`: the CPU oracle). Pictures are keyed by who drew
    /// them, so the stage keeps showing the previous ones as placeholders until the new ones
    /// arrive, and switching back finds the oracle's pictures still cached.
    pub fn set_rasterizer(&self, raster: Option<Rasterizer>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut s = self.shared.raster.lock().unwrap_or_else(PoisonError::into_inner);
        s.id = if raster.is_some() { NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) } else { 0 };
        s.raster = raster;
    }

    /// Whether a rasterizer is set.
    pub fn has_rasterizer(&self) -> bool {
        self.slot().raster.is_some()
    }

    /// The rasterizer in use (0: the CPU oracle), for [`make_job_for`].
    pub fn raster_id(&self) -> u64 {
        self.slot().id
    }

    /// A job for the rasterizer in use ([`make_job_for`] with [`RenderService::raster_id`]).
    pub fn job(&self, state: StateId, doc: &Document, frame: u32, view: View, onion: Option<&OnionSkin>) -> FrameJob {
        make_job_for(self.raster_id(), state, doc, frame, view, onion)
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
            let imgs: Vec<Arc<FrameImage>> = render_batch_by(&missing, &self.slot()).into_iter().map(Arc::new).collect();
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
        let fresh: Vec<Arc<FrameImage>> = render_batch_by(&missing, &self.slot()).into_iter().map(Arc::new).collect();
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
            Arc::new(FrameImage { key, frame, width: 0, height: 0, rgba_premul: Vec::new(), native: None, ms: 0.0, stats: RenderStats::default(), error: Some("render failed".into()), gpu: false, fallback: None })
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
        let slot = s.raster.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let imgs = render_batch_by(&todo, &slot);
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
        make_job(state, doc, frame, view, None)
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
        let full = render_job(&make_job(1, &doc, 13, View { transform: Affine::scale(z, z), width: w, height: h, background: bg }, None));
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
                jobs.push((tx, ty, make_job(1, &doc, 13, view, None)));
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
    fn a_rasterizer_draws_what_it_can_and_the_oracle_the_rest() {
        let doc = sample::demo();
        let svc = RenderService::new(64 << 20, None);
        let view = View { transform: Affine::scale(0.25, 0.25), width: 320, height: 180, background: Some(doc.stage.background) };
        let cpu = svc.job(1, &doc, 9, view, None);
        assert_eq!(cpu.key, make_job(1, &doc, 9, view, None).key, "no rasterizer: the oracle's keys");
        // A stand-in GPU that paints everything one colour, and refuses onion-skinned frames.
        let plain = display_list_with_onion(&doc, 9, None).items.len();
        let flat: Rasterizer = Arc::new(move |dl: &DisplayList, v: &View| {
            if dl.items.len() > plain {
                return Err("too much".into());
            }
            Ok((Raster::Premul8([10u8, 20, 30, 255].repeat(v.width as usize * v.height as usize)), RenderStats::default()))
        });
        svc.set_rasterizer(Some(flat));
        assert!(svc.has_rasterizer());
        let gpu = svc.job(1, &doc, 9, view, None);
        assert_ne!(gpu.key, cpu.key, "pictures are keyed by who drew them");
        let img = svc.render_now(gpu.clone());
        assert!(img.gpu && img.fallback.is_none() && img.rgba_premul[..4] == [10, 20, 30, 255]);
        // What it refuses, the oracle draws, saying why.
        let onion = svc.job(1, &doc, 9, view, Some(&OnionSkin::default()));
        let img = svc.render_now(onion.clone());
        assert!(!img.gpu && img.fallback.as_deref() == Some("too much"));
        assert_eq!(img.rgba_premul, render_job(&make_job(1, &doc, 9, view, Some(&OnionSkin::default()))).rgba_premul);
        // Through the worker too; and a job made for an older rasterizer is drawn by the oracle.
        let ahead = svc.job(1, &doc, 10, view, None);
        svc.prefetch(vec![ahead.clone()]);
        assert!(svc.wait_for(ahead.key, 10_000).is_some_and(|i| i.gpu));
        svc.set_rasterizer(None);
        let stale = FrameJob { key: 77, ..ahead };
        assert!(!render_batch_by(&[stale], &svc.slot())[0].gpu);
        assert_eq!(svc.job(1, &doc, 9, view, None).key, cpu.key, "back to the oracle's keys");
    }

    #[test]
    fn cache_respects_budget_lru() {
        let mut c = FrameCache::new(3 * 1000);
        for k in 0..10u64 {
            c.insert(Arc::new(FrameImage { key: k, frame: 0, width: 0, height: 0, rgba_premul: vec![0; 800], native: None, ms: 0.0, stats: RenderStats::default(), error: None, gpu: false, fallback: None }));
        }
        assert!(c.bytes() <= 3000);
        assert!(c.contains(9) && !c.contains(0));
    }

    #[test]
    fn gpu_pictures_have_a_budget_of_their_own() {
        // 10 × 10 pictures on the GPU (400 bytes each) under a 1,000-byte GPU budget: the
        // oldest GPU pictures go, the ones in memory stay.
        let mut c = FrameCache::new(1 << 20);
        c.native_budget_bytes = 1000;
        let img = |k: u64, native: bool| {
            let n: Option<NativeImage> = native.then(|| Arc::new(k) as NativeImage);
            Arc::new(FrameImage { key: k, frame: 0, width: 10, height: 10, rgba_premul: if native { Vec::new() } else { vec![0; 400] }, native: n, ms: 0.0, stats: RenderStats::default(), error: None, gpu: native, fallback: None })
        };
        c.insert(img(100, false));
        for k in 0..5 {
            c.insert(img(k, true));
        }
        assert!(c.native_bytes() <= 1000 && c.contains(4) && c.contains(3) && !c.contains(0));
        assert!(c.contains(100), "pictures in memory are not evicted for GPU memory");
        // Replacing a picture does not count it twice.
        c.insert(img(4, true));
        assert_eq!(c.native_bytes(), 800);
    }

    #[test]
    fn a_rasterizer_may_keep_its_pictures_on_the_gpu() {
        let doc = sample::demo();
        let svc = RenderService::new(64 << 20, None);
        let view = View { transform: Affine::scale(0.25, 0.25), width: 320, height: 180, background: Some(doc.stage.background) };
        // A stand-in that keeps its picture as a number, and gets the size wrong once.
        let wrong = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let w2 = wrong.clone();
        let native: Rasterizer = Arc::new(move |_: &DisplayList, v: &View| {
            let bad = w2.load(std::sync::atomic::Ordering::Relaxed);
            Ok((Raster::Native { image: Arc::new(7u32), width: v.width + u32::from(bad), height: v.height }, RenderStats::default()))
        });
        svc.set_rasterizer(Some(native));
        let img = svc.render_now(svc.job(1, &doc, 3, view, None));
        assert!(img.gpu && img.rgba_premul.is_empty());
        assert_eq!(img.native.as_ref().and_then(|n| n.downcast_ref::<u32>()), Some(&7));
        wrong.store(true, std::sync::atomic::Ordering::Relaxed);
        let img = svc.render_now(svc.job(1, &doc, 4, view, None));
        assert!(!img.gpu && img.native.is_none() && img.fallback.as_deref() == Some("the GPU returned a wrong-sized image"));
        assert_eq!(img.rgba_premul.len(), 320 * 180 * 4, "the oracle drew it instead");
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
        let plain = display_list_with_onion(&doc, 10, None).items.len();
        let onion = display_list_with_onion(&doc, 10, Some(&OnionSkin::default())).items.len();
        // ±2 frames of every unlocked layer (Animate doesn't onion-skin locked ones).
        let tl = doc.timeline(0);
        let unlocked = display_list_with_onion(&doc, 10, None).items.iter().filter(|i| tl.layer(i.layer).is_some_and(|l| !l.locked)).count();
        assert!(unlocked < plain, "the sample has a locked layer");
        assert_eq!(onion, plain + 4 * unlocked);
    }
}
