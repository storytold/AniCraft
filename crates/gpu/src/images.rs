//! Bitmaps on the GPU (ADR 0032): each image is uploaded once, as a straight RGBA8 texture, and
//! kept for later frames under a memory budget (least recently used first out).
//!
//! The key is the image itself (the `Arc<Pixels>` the display list holds, compared by pointer,
//! and kept alive by the cache so its address can't be reused): a replaced or re-imported
//! bitmap is a new image and never shows a stale texture, and two open documents with the same
//! bitmap id never share one. Textures are immutable once uploaded, so an entry evicted while a
//! submitted frame still samples it stays valid for that frame (wgpu keeps it alive).

use crate::{GpuError, describe};
use anicraft_doc::Pixels;
use anicraft_render::NeedsFull;
use std::collections::HashMap;
use std::sync::Arc;

/// Graphics memory the cached bitmaps may hold (4 bytes a pixel). One frame may need more (its
/// own images are never evicted while it binds them); the rest go first.
pub const IMAGE_BUDGET: u64 = 256 << 20;

struct Entry {
    /// Kept so the pointer stays this image's for as long as the entry lives.
    pixels: Arc<Pixels>,
    view: wgpu::TextureView,
    bytes: u64,
    used: u64,
}

#[derive(Default)]
pub(crate) struct Images {
    map: HashMap<usize, Entry>,
    bytes: u64,
    clock: u64,
}

impl Images {
    /// Images and bytes held.
    pub(crate) fn held(&self) -> (usize, u64) {
        (self.map.len(), self.bytes)
    }

    /// Texture views for `want` (in order), uploading what isn't cached yet.
    pub(crate) fn views(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, max_edge: u32, want: &[Arc<Pixels>]) -> Result<Vec<wgpu::TextureView>, GpuError> {
        self.clock += 1;
        let now = self.clock;
        let mut out = Vec::with_capacity(want.len());
        for px in want {
            let key = Arc::as_ptr(px) as usize;
            if let Some(e) = self.map.get_mut(&key).filter(|e| Arc::ptr_eq(&e.pixels, px)) {
                e.used = now;
                out.push(e.view.clone());
                continue;
            }
            let (view, bytes) = upload(device, queue, max_edge, px)?;
            self.bytes += bytes;
            out.push(view.clone());
            self.map.insert(key, Entry { pixels: px.clone(), view, bytes, used: now });
        }
        // Over budget: the least recently used first, never what this frame binds.
        while self.bytes > IMAGE_BUDGET {
            let Some((&key, _)) = self.map.iter().filter(|(_, e)| e.used != now).min_by_key(|(_, e)| e.used) else { break };
            if let Some(e) = self.map.remove(&key) {
                self.bytes = self.bytes.saturating_sub(e.bytes);
            }
        }
        Ok(out)
    }
}

/// One image as a texture, in its own error scopes: a failed upload is never cached.
fn upload(device: &wgpu::Device, queue: &wgpu::Queue, max_edge: u32, px: &Pixels) -> Result<(wgpu::TextureView, u64), GpuError> {
    let (w, h) = (px.width, px.height);
    if w == 0 || h == 0 || w > max_edge || h > max_edge {
        return Err(GpuError::NeedsFull(NeedsFull::Bitmaps));
    }
    let len = w as usize * h as usize * 4;
    // A short buffer reads as transparent past its end, as `Pixels::at` does.
    let padded;
    let bytes: &[u8] = match px.rgba.get(..len) {
        Some(b) => b,
        None => {
            let mut v = px.rgba.clone();
            v.resize(len, 0);
            padded = v;
            &padded
        }
    };
    let scopes = crate::Scopes::push(device);
    let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("anicraft-bitmap"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let (oom, other) = scopes.pop_split();
    if let Some(e) = other {
        return Err(GpuError::Device(describe(&e)));
    }
    if let Some(e) = oom {
        return Err(GpuError::TooLarge(format!("no room on the GPU for a {w}×{h} bitmap: {e}")));
    }
    Ok((view, len as u64))
}
