//! GPU textures for thumbnails, evicting the least recently drawn ones so large libraries
//! don't fill video memory. Evicted thumbnails reload quickly from the on-disk cache.

use std::collections::HashMap;

use egui::{ColorImage, TextureHandle, TextureOptions};
use polyroom_engine::{Image, ModelId};

/// Textures kept alive at most (~256 KB each).
const MAX_TEXTURES: usize = 800;

#[derive(Default)]
pub struct ThumbTextures {
    textures: HashMap<ModelId, (TextureHandle, u64)>,
    frame: u64,
}

impl ThumbTextures {
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    pub fn insert(&mut self, ctx: &egui::Context, id: ModelId, image: &Image) {
        let size = [image.width as usize, image.height as usize];
        if size[0] * size[1] * 4 != image.rgba.len() {
            return;
        }
        // Thumbnails are rendered with premultiplied alpha.
        let color = ColorImage::from_rgba_premultiplied(size, &image.rgba);
        let options = TextureOptions {
            mipmap_mode: Some(egui::TextureFilter::Linear),
            ..TextureOptions::LINEAR
        };
        let handle = ctx.load_texture(format!("thumb-{id}"), color, options);
        self.textures.insert(id, (handle, self.frame));
        self.evict();
    }

    /// Drops the texture for `id` so it is reloaded (its file changed).
    pub fn remove(&mut self, id: ModelId) {
        self.textures.remove(&id);
    }

    /// The texture for `id`, marking it as recently used.
    pub fn get(&mut self, id: ModelId) -> Option<egui::TextureId> {
        let frame = self.frame;
        self.textures.get_mut(&id).map(|(h, used)| {
            *used = frame;
            h.id()
        })
    }

    fn evict(&mut self) {
        if self.textures.len() <= MAX_TEXTURES {
            return;
        }
        let mut by_age: Vec<(u64, ModelId)> = self
            .textures
            .iter()
            .map(|(id, (_, used))| (*used, *id))
            .collect();
        by_age.sort_unstable();
        let excess = self.textures.len() - MAX_TEXTURES * 3 / 4;
        for (used, id) in by_age.into_iter().take(excess) {
            // Never drop something drawn this frame or the previous one.
            if used + 1 >= self.frame {
                break;
            }
            self.textures.remove(&id);
        }
    }
}
