//! Layout editor: compose several rendered plots into one image/export.
//!
//! Each panel is a self-contained SVG document (as produced by
//! [`crate::plot::dot_plot_svg`]). [`render_layout`] arranges them in a grid and
//! nests each panel inside the composite SVG with an offset and a title, which
//! is how the GUI's "layout editor" exports multi-plot figures.

/// A single panel in a layout: a title and a pre-rendered SVG document.
#[derive(Debug, Clone, Default)]
pub struct LayoutPanel {
    pub title: String,
    pub svg: String,
}

/// Options controlling how a layout grid is rendered.
#[derive(Debug, Clone)]
pub struct LayoutOptions {
    pub columns: usize,
    pub cell_width: u32,
    pub cell_height: u32,
    /// Height reserved above each panel for its title.
    pub title_height: u32,
    /// Padding around the whole figure.
    pub padding: u32,
    pub background: String,
    pub text_color: String,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            columns: 2,
            cell_width: 500,
            cell_height: 450,
            title_height: 28,
            padding: 12,
            background: "#ffffff".to_string(),
            text_color: "#222222".to_string(),
        }
    }
}

fn svg_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Re-emits an SVG document positioned at `(x, y)` inside a parent SVG.
fn nest_svg(svg: &str, x: f64, y: f64) -> String {
    // The document always begins with an `<svg ` open tag.
    if let Some(idx) = svg.find("<svg ") {
        let mut out = String::with_capacity(svg.len() + 32);
        out.push_str(&svg[..idx]);
        out.push_str(&format!("<svg x=\"{x}\" y=\"{y}\" "));
        out.push_str(&svg[idx + 5..]);
        out
    } else {
        svg.to_string()
    }
}

/// Arranges panels in a grid and returns a composite SVG document.
pub fn render_layout(panels: &[LayoutPanel], opts: &LayoutOptions) -> String {
    let cols = opts.columns.max(1);
    let rows = if panels.is_empty() {
        1
    } else {
        panels.len().div_ceil(cols)
    };
    let cell_w = opts.cell_width + opts.padding;
    let cell_h = opts.cell_height + opts.title_height + opts.padding;
    let total_w = cell_w * cols as u32 + opts.padding;
    let total_h = cell_h * rows as u32 + opts.padding;

    let mut svg = String::new();
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}">"#
    ));
    svg.push('\n');
    svg.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{total_w}\" height=\"{total_h}\" fill=\"{}\" />\n",
        opts.background
    ));

    for (i, panel) in panels.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let cx = opts.padding as f64 + col as f64 * cell_w as f64;
        let cy = opts.padding as f64 + row as f64 * cell_h as f64;

        // Title.
        svg.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"13\" font-weight=\"bold\" fill=\"{}\" text-anchor=\"middle\">{}</text>\n",
            cx + opts.cell_width as f64 / 2.0,
            cy + 18.0,
            opts.text_color,
            svg_escape(&panel.title)
        ));

        // Nested panel, offset below the title.
        let nested = nest_svg(&panel.svg, cx, cy + opts.title_height as f64);
        svg.push_str(&nested);
        if !nested.ends_with('\n') {
            svg.push('\n');
        }
    }

    svg.push_str("</svg>\n");
    svg
}

/// A panel placed at an explicit position and size on a free-form canvas.
///
/// This backs the GUI layout editor, where panels can be dragged and resized
/// like objects in a drawing program before being exported.
#[derive(Debug, Clone, Default)]
pub struct LayoutPlacement {
    pub title: String,
    pub svg: String,
    /// The natural size of `svg` (before scaling to `width`/`height`).
    pub svg_width: u32,
    pub svg_height: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Composes freely-placed panels into a single SVG canvas.
///
/// Each panel's SVG is nested and scaled to fit its `width`/`height`, with its
/// title drawn above it.
pub fn render_layout_free(
    canvas_width: u32,
    canvas_height: u32,
    placements: &[LayoutPlacement],
    opts: &LayoutOptions,
) -> String {
    let canvas_width = canvas_width.max(1);
    let canvas_height = canvas_height.max(1);
    let mut svg = String::new();
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{canvas_width}" height="{canvas_height}" viewBox="0 0 {canvas_width} {canvas_height}">"#
    ));
    svg.push('\n');
    svg.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{canvas_width}\" height=\"{canvas_height}\" fill=\"{}\" />\n",
        opts.background
    ));

    for p in placements {
        // Title above the panel.
        svg.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"13\" font-weight=\"bold\" fill=\"{}\" text-anchor=\"middle\">{}</text>\n",
            p.x as f64 + p.width as f64 / 2.0,
            p.y as f64 + 12.0,
            opts.text_color,
            svg_escape(&p.title)
        ));

        let nat_w = p.svg_width.max(1) as f32;
        let nat_h = p.svg_height.max(1) as f32;
        let sx = p.width / nat_w;
        let sy = p.height / nat_h;
        svg.push_str(&format!(
            "<g transform=\"translate({:.2},{:.2}) scale({:.4},{:.4})\">\n",
            p.x,
            p.y + opts.title_height as f32,
            sx,
            sy
        ));
        let nested = nest_svg(&p.svg, 0.0, 0.0);
        svg.push_str(&nested);
        if !nested.ends_with('\n') {
            svg.push('\n');
        }
        svg.push_str("</g>\n");
    }

    svg.push_str("</svg>\n");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;
    use crate::plot::{dot_plot_svg, PlotStyle};

    #[test]
    fn render_layout_nests_panels() {
        let data = array![[1.0, 2.0], [3.0, 4.0]];
        let panel = dot_plot_svg(
            &data,
            0,
            1,
            "A",
            "B",
            &[],
            None,
            None,
            &[],
            &PlotStyle::default(),
        );
        let panels = vec![
            LayoutPanel {
                title: "Sample 1".into(),
                svg: panel.clone(),
            },
            LayoutPanel {
                title: "Sample 2".into(),
                svg: panel,
            },
        ];
        let out = render_layout(&panels, &LayoutOptions::default());
        assert!(out.starts_with("<svg"));
        assert!(out.contains("Sample 1"));
        assert!(out.contains("Sample 2"));
        assert!(out.matches("<svg").count() >= 3); // outer + 2 nested
    }

    #[test]
    fn render_empty_layout() {
        let out = render_layout(&[], &LayoutOptions::default());
        assert!(out.starts_with("<svg"));
        assert!(out.ends_with("</svg>\n"));
    }

    #[test]
    fn render_free_layout_scales_and_places() {
        let data = array![[1.0, 2.0], [3.0, 4.0]];
        let panel = dot_plot_svg(&data, 0, 1, "A", "B", &[], None, None, &[], &PlotStyle::default());
        let placements = vec![LayoutPlacement {
            title: "Sample 1".into(),
            svg: panel,
            svg_width: 500,
            svg_height: 450,
            x: 30.0,
            y: 40.0,
            width: 300.0,
            height: 260.0,
        }];
        let out = render_layout_free(800, 600, &placements, &LayoutOptions::default());
        assert!(out.starts_with("<svg"));
        assert!(out.contains("translate(30.00,68.00)"));
        assert!(out.contains("scale(0.6000,0.5778)"));
        assert!(out.contains("Sample 1"));
    }
}
