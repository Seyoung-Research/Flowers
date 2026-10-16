//! Raster image export.
//!
//! The plotting code emits SVG. To support exporting to PNG / JPEG / TIFF we
//! rasterise that SVG with [`resvg`] (which handles the vector geometry and
//! text layout) and encode the resulting pixels with the [`image`] crate.

use std::path::Path;

/// A raster image format we can export to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Tiff,
}

impl ImageFormat {
    /// Infers a format from a file extension, if recognised.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "tif" | "tiff" => Some(Self::Tiff),
            _ => None,
        }
    }

    /// The canonical file extension for this format.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Tiff => "tiff",
        }
    }

    /// A short human-readable name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Tiff => "TIFF",
        }
    }

    /// The `(name, extensions)` pairs used by the native save dialog.
    pub fn dialog_filters() -> &'static [(&'static str, &'static [&'static str])] {
        &[
            ("PNG", &["png"]),
            ("JPEG", &["jpg", "jpeg"]),
            ("TIFF", &["tif", "tiff"]),
        ]
    }
}

/// Rasterises an SVG string into an RGBA image.
///
/// `scale` multiplies the logical `width`/`height` to control the output
/// resolution (e.g. `2.0` for a 2x supersampled export).
pub fn rasterize_svg(
    svg: &str,
    width: u32,
    height: u32,
    scale: f32,
) -> Result<image::RgbaImage, String> {
    use resvg::usvg::{TreeParsing, TreeTextToPath};

    let options = resvg::usvg::Options::default();
    let mut tree = resvg::usvg::Tree::from_str(svg, &options)
        .map_err(|e| format!("could not parse SVG: {e}"))?;

    // Convert <text> elements to paths so labels survive rasterisation.
    let mut fontdb = fontdb::Database::new();
    fontdb.load_system_fonts();
    tree.convert_text(&fontdb);

    let tree = resvg::Tree::from_usvg(&tree);

    let scale = scale.max(0.05);
    let out_w = ((width as f32) * scale).round().max(1.0) as u32;
    let out_h = ((height as f32) * scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(out_w, out_h)
        .ok_or_else(|| format!("invalid image size {out_w}x{out_h}"))?;

    tree.render(
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let mut data = pixmap.take();
    // tiny-skia stores premultiplied RGBA; `image` expects straight alpha.
    for px in data.chunks_exact_mut(4) {
        let a = px[3];
        if a != 0 && a != 255 {
            let inv = 255.0 / a as f32;
            for c in &mut px[..3] {
                *c = ((*c as f32) * inv).round().min(255.0) as u8;
            }
        }
    }

    image::RgbaImage::from_raw(out_w, out_h, data)
        .ok_or_else(|| "could not build image buffer".to_string())
}

/// Rasterises `svg` and writes it to `path`, inferring the format from the
/// file extension. Returns the format that was written.
pub fn save_svg_as_image(
    svg: &str,
    path: &Path,
    width: u32,
    height: u32,
    scale: f32,
) -> Result<ImageFormat, String> {
    use image::ImageEncoder;

    let format = ImageFormat::from_path(path)
        .ok_or_else(|| "unsupported image extension (use .png, .jpg or .tiff)".to_string())?;
    let rgba = rasterize_svg(svg, width, height, scale)?;
    let (w, h) = rgba.dimensions();

    let file = std::fs::File::create(path).map_err(|e| format!("could not create file: {e}"))?;
    let writer = std::io::BufWriter::new(file);

    match format {
        ImageFormat::Png => {
            image::codecs::png::PngEncoder::new(writer)
                .write_image(rgba.as_raw(), w, h, image::ColorType::Rgba8)
                .map_err(|e| format!("PNG encoding failed: {e}"))?;
        }
        ImageFormat::Jpeg => {
            let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(writer, 92)
                .encode(rgb.as_raw(), w, h, image::ColorType::Rgb8)
                .map_err(|e| format!("JPEG encoding failed: {e}"))?;
        }
        ImageFormat::Tiff => {
            image::codecs::tiff::TiffEncoder::new(writer)
                .write_image(rgba.as_raw(), w, h, image::ColorType::Rgba8)
                .map_err(|e| format!("TIFF encoding failed: {e}"))?;
        }
    }

    Ok(format)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_from_extension() {
        assert_eq!(ImageFormat::from_path(Path::new("a.png")), Some(ImageFormat::Png));
        assert_eq!(ImageFormat::from_path(Path::new("a.JPG")), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_path(Path::new("a.tif")), Some(ImageFormat::Tiff));
        assert_eq!(ImageFormat::from_path(Path::new("a.svg")), None);
    }

    #[test]
    fn rasterizes_shapes() {
        // A 10x10 SVG filled with solid red; the centre pixel should be red.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <rect x="0" y="0" width="10" height="10" fill="#ff0000"/></svg>"##;
        let img = rasterize_svg(svg, 10, 10, 1.0).expect("rasterise");
        assert_eq!(img.dimensions(), (10, 10));
        let px = img.get_pixel(5, 5);
        assert!(px[0] > 200 && px[1] < 40 && px[2] < 40, "got {px:?}");
    }

    #[test]
    fn scale_multiplies_size() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <rect x="0" y="0" width="10" height="10" fill="#00ff00"/></svg>"##;
        let img = rasterize_svg(svg, 10, 10, 3.0).expect("rasterise");
        assert_eq!(img.dimensions(), (30, 30));
    }

    #[test]
    fn text_does_not_panic_without_fonts() {
        // Text nodes are converted to paths; with no matching font they simply
        // contribute no geometry, but must not panic.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
            <text x="2" y="14" font-size="12">FSC-A</text></svg>"##;
        let img = rasterize_svg(svg, 40, 20, 1.0).expect("rasterise");
        assert_eq!(img.dimensions(), (40, 20));
    }

    #[test]
    fn saves_all_formats() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12">
            <rect x="0" y="0" width="12" height="12" fill="#3366cc"/></svg>"##;
        let dir = std::env::temp_dir().join("flowers_raster_test");
        std::fs::create_dir_all(&dir).unwrap();
        for (ext, expected) in [
            ("png", ImageFormat::Png),
            ("jpg", ImageFormat::Jpeg),
            ("tiff", ImageFormat::Tiff),
        ] {
            let path = dir.join(format!("out.{ext}"));
            let fmt = save_svg_as_image(svg, &path, 12, 12, 1.0).expect("save");
            assert_eq!(fmt, expected);
            assert!(path.exists());
            assert!(std::fs::metadata(&path).unwrap().len() > 0);
        }
    }
}
