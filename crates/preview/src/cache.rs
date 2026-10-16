//! PNG encoding for the thumbnail cache.

use polyroom_render::Image;

pub fn encode_png(image: &Image) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&image.rgba).ok()?;
    }
    Some(out)
}

/// Decodes an 8-bit RGBA PNG; anything else (or a corrupt file) yields `None`.
pub fn decode_png(bytes: &[u8]) -> Option<Image> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some(Image {
        width: info.width,
        height: info.height,
        rgba: buf,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip() {
        let image = Image {
            width: 3,
            height: 2,
            rgba: (0..24).collect(),
        };
        assert_eq!(decode_png(&encode_png(&image).unwrap()), Some(image));
        assert_eq!(decode_png(b"garbage"), None);
    }
}
