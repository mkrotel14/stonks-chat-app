//! Decode bounded static/animated images off the UI thread. Preserve delays
//! and compositing; GPUI expects BGRA pixels and advances frames itself.
use image::{AnimationDecoder, ImageDecoder};
use std::io::Cursor;

pub const MAX_ANIMATION_BYTES: usize = 16 * 1024 * 1024;
const MAX_FRAMES: usize = 300;

pub struct Decoded {
    pub frames: Vec<image::Frame>,
    pub bytes: usize,
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    limits
}

pub fn decode(bytes: &[u8], edge: u32) -> Option<Decoded> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    match reader.format()? {
        image::ImageFormat::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).ok()?;
            decoder.set_limits(limits()).ok()?;
            frames(decoder.into_frames(), edge)
        }
        image::ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
            decoder.set_limits(limits()).ok()?;
            if decoder.has_animation() {
                frames(decoder.into_frames(), edge)
            } else {
                reader.limits(limits());
                single(reader.decode().ok()?, edge)
            }
        }
        image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Bmp => {
            reader.limits(limits());
            single(reader.decode().ok()?, edge)
        }
        _ => None,
    }
}

fn pixels(decoded: image::DynamicImage, edge: u32) -> image::RgbaImage {
    let edge = edge.max(1);
    let mut pixels = if decoded.width() > edge || decoded.height() > edge {
        decoded.thumbnail(edge, edge).into_rgba8()
    } else {
        decoded.into_rgba8()
    };
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    pixels
}

fn single(decoded: image::DynamicImage, edge: u32) -> Option<Decoded> {
    let pixels = pixels(decoded, edge);
    let bytes = pixels.as_raw().len();
    Some(Decoded {
        frames: vec![image::Frame::new(pixels)],
        bytes,
    })
}

fn frames(source: image::Frames<'_>, edge: u32) -> Option<Decoded> {
    let mut result = Decoded {
        frames: Vec::new(),
        bytes: 0,
    };
    for frame in source {
        let frame = frame.ok()?;
        let delay = frame.delay();
        let pixels = pixels(image::DynamicImage::ImageRgba8(frame.into_buffer()), edge);
        let bytes = pixels.as_raw().len();
        if result.frames.len() >= MAX_FRAMES
            || result.bytes.saturating_add(bytes) > MAX_ANIMATION_BYTES
        {
            // Keep a usable preview rather than a truncated animation loop.
            let first = result.frames.into_iter().next()?;
            let bytes = first.buffer().as_raw().len();
            return Some(Decoded {
                frames: vec![first],
                bytes,
            });
        }
        result.bytes += bytes;
        result
            .frames
            .push(image::Frame::from_parts(pixels, 0, 0, delay));
    }
    (!result.frames.is_empty()).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bmp_decodes_with_bgra_pixels_and_respects_dimension_limits() {
        let encode = |width, height| {
            let mut buffer = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                width,
                height,
                image::Rgb([255, 20, 10]),
            ))
            .write_to(&mut buffer, image::ImageFormat::Bmp)
            .unwrap();
            buffer.into_inner()
        };
        let decoded = decode(&encode(2, 1), 80).unwrap();
        assert_eq!(decoded.frames.len(), 1);
        assert_eq!(decoded.frames[0].buffer().dimensions(), (2, 1));
        assert_eq!(
            decoded.frames[0].buffer().get_pixel(0, 0).0,
            [10, 20, 255, 255]
        );
        assert!(decode(&encode(4097, 1), 80).is_none());
    }

    #[test]
    fn gif_keeps_frames_delays_colors_and_does_not_upscale() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(2, 1, image::Rgba(color)),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(100, 1),
                    ))
                    .unwrap();
            }
        }
        let decoded = decode(&bytes, 80).unwrap();
        assert_eq!(decoded.frames.len(), 2);
        assert_eq!(decoded.bytes, 16);
        assert_eq!(
            decoded.frames[0].buffer().get_pixel(0, 0).0,
            [0, 0, 255, 255]
        );
        assert_eq!(
            decoded.frames[1].buffer().get_pixel(0, 0).0,
            [255, 0, 0, 255]
        );
        assert_eq!(decoded.frames[0].delay().numer_denom_ms(), (100, 1));
    }
    #[test]
    fn animated_webp_keeps_both_frames_and_timing() {
        // Synthetic testsrc2 fixture, generated locally with ffmpeg's
        // libwebp_anim encoder; contains no account/user image data.
        let decoded = decode(include_bytes!("../../../tests/fixtures/animated.webp"), 80).unwrap();
        assert_eq!(decoded.frames.len(), 2);
        assert_eq!(decoded.bytes, 2 * 16 * 16 * 4);
        assert_ne!(decoded.frames[0].buffer(), decoded.frames[1].buffer());
        assert_eq!(decoded.frames[0].delay().numer_denom_ms(), (500, 1));
    }

    #[test]
    fn oversized_frame_count_falls_back_to_a_complete_static_preview() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for _ in 0..=MAX_FRAMES {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        1,
                        1,
                        image::Rgba([255, 0, 0, 255]),
                    )))
                    .unwrap();
            }
        }
        let decoded = decode(&bytes, 80).unwrap();
        assert_eq!(decoded.frames.len(), 1);
        assert_eq!(decoded.bytes, 4);
    }
}
