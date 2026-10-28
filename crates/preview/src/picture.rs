//! Decoding preview images (PNG, JPEG, WebP, GIF, BMP) into thumbnails and loupe pictures.

use std::io::Cursor;

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use polyroom_render::Image;

/// Largest image side we decode; bigger images are refused rather than eating memory.
const MAX_SIDE: u32 = 16_384;
const MAX_ALLOC: u64 = 512 << 20;

fn decode(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("could not read image: {e}"))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("unsupported image: {e}"))?;
    // Phone photos are often stored sideways with an EXIF hint to turn them.
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| format!("invalid image: {e}"))?;
    img.apply_orientation(orientation);
    Ok(img.into_rgba8())
}

/// Converts straight alpha to the premultiplied alpha the UI expects.
fn premultiply(rgba: &mut [u8]) {
    for [r, g, b, a] in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(*a);
        for c in [r, g, b] {
            *c = ((u16::from(*c) * alpha + 127) / 255) as u8;
        }
    }
}

/// Decodes an image and scales it down (never up) so its longer side is at most `max_px`.
pub fn fit(bytes: &[u8], max_px: u32) -> Result<Image, String> {
    let rgba = decode(bytes)?;
    let (w, h) = rgba.dimensions();
    let longest = w.max(h).max(1);
    let small = if longest > max_px {
        let scale = f64::from(max_px) / f64::from(longest);
        let tw = ((f64::from(w) * scale).round() as u32).clamp(1, max_px);
        let th = ((f64::from(h) * scale).round() as u32).clamp(1, max_px);
        image::imageops::thumbnail(&rgba, tw, th)
    } else {
        rgba
    };
    let (width, height) = small.dimensions();
    let mut data = small.into_raw();
    premultiply(&mut data);
    Ok(Image {
        width,
        height,
        rgba: data,
    })
}

/// A `px`×`px` thumbnail: the image fitted inside, centred on a transparent background.
pub fn thumbnail(bytes: &[u8], px: u32) -> Result<Image, String> {
    let fitted = fit(bytes, px)?;
    let side = px as usize;
    let (w, h) = (fitted.width as usize, fitted.height as usize);
    let (ox, oy) = ((side.saturating_sub(w)) / 2, (side.saturating_sub(h)) / 2);
    let mut out = vec![0u8; side * side * 4];
    for (y, row) in fitted.rgba.chunks_exact(w.max(1) * 4).enumerate() {
        let start = ((y + oy) * side + ox) * 4;
        if let Some(dst) = out.get_mut(start..start + row.len()) {
            dst.copy_from_slice(row);
        }
    }
    Ok(Image {
        width: px,
        height: px,
        rgba: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w`×`h` PNG, opaque red.
    pub(crate) fn red_png(w: u32, h: u32) -> Vec<u8> {
        crate::encode_png(&Image {
            width: w,
            height: h,
            rgba: [255, 0, 0, 255].repeat((w * h) as usize),
        })
        .unwrap()
    }

    #[test]
    fn a_wide_image_is_letterboxed_into_a_square() {
        let thumb = thumbnail(&red_png(400, 100), 64).unwrap();
        assert_eq!((thumb.width, thumb.height), (64, 64));
        let px = |x: usize, y: usize| &thumb.rgba[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert_eq!(px(32, 32), [255, 0, 0, 255], "the centre is the image");
        assert_eq!(px(32, 0), [0, 0, 0, 0], "above it is transparent");
    }

    #[test]
    fn small_images_are_not_enlarged() {
        let img = fit(&red_png(20, 10), 2048).unwrap();
        assert_eq!((img.width, img.height), (20, 10));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(fit(b"\x89PNG\r\n\x1a\nnope", 64).is_err());
        assert!(thumbnail(b"", 64).is_err());
    }
}
