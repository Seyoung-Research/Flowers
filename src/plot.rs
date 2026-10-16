//! Rendering helpers for flow cytometry plots.
//!
//! This module has two halves:
//!
//! * **SVG export** — [`dot_plot_svg`] and [`histogram_svg`] produce standalone
//!   `.svg` files without any external graphics dependency, so `flowers` can
//!   run headless (CLI and "layout editor" exports).
//! * **Analysis helpers** — [`density_grid`], [`point_densities`], and
//!   [`contour_segments`] compute 2D kernel/histogram densities, per-point
//!   densities (for gradient point colouring) and topographic contour lines.
//!   These are shared by the GUI and the SVG renderer.

use serde::Serialize;

/// How a plot axis maps values to positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum AxisScale {
    Linear,
    Log,
}

impl Default for AxisScale {
    fn default() -> Self {
        AxisScale::Linear
    }
}

impl AxisScale {
    /// Forward transform: data value -> plot position (base-10 log for `Log`).
    /// Returns `None` for non-positive values on a log axis.
    pub fn transform(self, v: f64) -> Option<f64> {
        match self {
            AxisScale::Linear => Some(v),
            AxisScale::Log => {
                if v > 0.0 {
                    Some(v.log10())
                } else {
                    None
                }
            }
        }
    }

    /// Inverse transform: plot position -> data value.
    pub fn inverse(self, t: f64) -> f64 {
        match self {
            AxisScale::Linear => t,
            AxisScale::Log => 10f64.powf(t),
        }
    }

    /// Human-readable label for this scale.
    pub fn label(self) -> &'static str {
        match self {
            AxisScale::Linear => "Linear",
            AxisScale::Log => "Log",
        }
    }
}

/// Styling for a rendered plot.
#[derive(Debug, Clone, Serialize)]
pub struct PlotStyle {
    pub width: u32,
    pub height: u32,
    pub point_radius: f32,
    pub point_opacity: f32,
    pub point_color: String,
    pub line_color: String,
    pub axis_color: String,
    pub text_color: String,
    pub background: String,
    /// Scale of the X axis.
    pub x_scale: AxisScale,
    /// Scale of the Y axis.
    pub y_scale: AxisScale,
    /// Colour events by local density (gradient layering).
    pub gradient: bool,
    /// Render a topographic contour view instead of individual points.
    pub contour: bool,
    /// Number of contour levels used when `contour` is enabled.
    pub contour_levels: usize,
    /// Optional plot title.
    pub title: String,
    /// Font size (in SVG units) of the X-axis label.
    pub x_label_size: f32,
    /// Font size (in SVG units) of the Y-axis label.
    pub y_label_size: f32,
    /// Rotation of the X-axis label, in degrees (clockwise).
    pub x_label_rotation: f32,
    /// Rotation of the Y-axis label, in degrees (clockwise).
    pub y_label_rotation: f32,
    /// CSS font family used for the plot title.
    pub label_font: String,
    /// CSS font family used for the X-axis label.
    pub x_label_font: String,
    /// CSS font family used for the Y-axis label.
    pub y_label_font: String,
    /// Font size (in SVG units) of the plot title.
    pub title_size: f32,
    /// When false, the `(linear)` / `(log10)` suffix is omitted from axis labels.
    pub show_scale_labels: bool,
}

impl Default for PlotStyle {
    fn default() -> Self {
        Self {
            width: 500,
            height: 450,
            point_radius: 2.0,
            point_opacity: 0.6,
            point_color: "#1f77b4".to_string(),
            line_color: "#2ca02c".to_string(),
            axis_color: "#444444".to_string(),
            text_color: "#222222".to_string(),
            background: "#ffffff".to_string(),
            x_scale: AxisScale::Linear,
            y_scale: AxisScale::Linear,
            gradient: false,
            contour: false,
            contour_levels: 8,
            title: String::new(),
            x_label_size: 12.0,
            y_label_size: 12.0,
            x_label_rotation: 0.0,
            y_label_rotation: -90.0,
            label_font: "sans-serif".to_string(),
            x_label_font: "sans-serif".to_string(),
            y_label_font: "sans-serif".to_string(),
            title_size: 14.0,
            show_scale_labels: true,
        }
    }
}

/// A pre-computed overlay (gate outline) to draw on top of an SVG plot.
#[derive(Debug, Clone)]
pub struct PlotOverlay {
    /// Vertices in data coordinates.
    pub points: Vec<[f64; 2]>,
    /// Stroke colour (CSS colour string).
    pub stroke: String,
    /// Optional fill colour (CSS colour string).
    pub fill: Option<String>,
    /// Whether the path should be closed.
    pub closed: bool,
}

/// Escapes a string for safe inclusion in an SVG text node.
fn svg_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Builds an axis label, optionally appending the `(linear)` / `(log10)` suffix.
fn label_with_scale(label: &str, scale: AxisScale, show_scale: bool) -> String {
    if show_scale {
        format!("{} ({})", label, scale.label())
    } else {
        label.to_string()
    }
}

/// An SVG `transform="rotate(...)"` attribute about `(x, y)`, or an empty string
/// when the rotation is effectively zero.
fn rotation_attr(deg: f32, (x, y): (f64, f64)) -> String {
    if deg.abs() < 1e-6 {
        String::new()
    } else {
        format!(" transform=\"rotate({deg} {x},{y})\"")
    }
}

/// Formats an axis tick value for display given an axis scale.
///
/// Large values are rendered in scientific notation (e.g. `7e4` for 70000) so
/// that tick labels do not collide with the axis title.
pub fn format_tick(v: f64, scale: AxisScale) -> String {
    let value = match scale {
        AxisScale::Linear => v,
        AxisScale::Log => 10f64.powf(v),
    };
    if !value.is_finite() {
        return "".to_string();
    }
    let abs = value.abs();
    if abs >= 1000.0 || (abs > 0.0 && abs < 0.01) {
        // Scientific notation: one significant digit, strip trailing ".0".
        let mut s = format!("{value:.1e}");
        // "7.0e4" -> "7e4", "7.5e4" stays "7.5e4". Replace the whole ".0e"
        // run so the exponent marker is not duplicated.
        if let Some(pos) = s.find(".0e") {
            s.replace_range(pos..pos + 3, "e");
        }
        // Normalise exponent: "7e+04" -> "7e4".
        if let Some(e_pos) = s.find('e') {
            let rest = &s[e_pos + 1..];
            let normalized = rest.trim_start_matches('+').trim_start_matches('0');
            let normalized = if normalized.is_empty() { "0" } else { normalized };
            s = format!("{}e{}", &s[..e_pos], normalized);
        }
        s
    } else if abs >= 100.0 {
        format!("{value:.0}")
    } else if abs >= 10.0 {
        format!("{value:.1}")
    } else if abs >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}

/// Renders a 2D dot plot of two parameters for a set of events.
///
/// `data` has shape `(events, parameters)`. `x_idx` / `y_idx` select the
/// columns. `indices` selects which events to draw (if empty, all events).
/// `x_range` / `y_range` are the axis limits in **data** coordinates; if `None`
/// they are computed from the selected data. `overlays` are gate outlines drawn
/// on top of the events.
#[allow(clippy::too_many_arguments)]
pub fn dot_plot_svg(
    data: &ndarray::Array2<f32>,
    x_idx: usize,
    y_idx: usize,
    x_label: &str,
    y_label: &str,
    indices: &[usize],
    x_range: Option<(f32, f32)>,
    y_range: Option<(f32, f32)>,
    overlays: &[PlotOverlay],
    style: &PlotStyle,
) -> String {
    let (_, n_params) = data.dim();
    assert!(x_idx < n_params && y_idx < n_params);

    let sel: Vec<usize> = if indices.is_empty() {
        (0..data.nrows()).collect()
    } else {
        indices.to_vec()
    };

    let raw_x: Vec<f64> = sel.iter().map(|&i| data[[i, x_idx]] as f64).collect();
    let raw_y: Vec<f64> = sel.iter().map(|&i| data[[i, y_idx]] as f64).collect();

    // Resolve the data-space ranges.
    let (dx0, dx1) = x_range.unwrap_or_else(|| {
        let (a, b) = compute_range(&raw_x, style.x_scale);
        (a as f32, b as f32)
    });
    let (dy0, dy1) = y_range.unwrap_or_else(|| {
        let (a, b) = compute_range(&raw_y, style.y_scale);
        (a as f32, b as f32)
    });
    let (dx0, dx1) = (dx0 as f64, dx1 as f64);
    let (dy0, dy1) = (dy0 as f64, dy1 as f64);

    // Transform ranges into plot space.
    let x0t = style.x_scale.transform(dx0).unwrap_or(0.0);
    let x1t = style.x_scale.transform(dx1).unwrap_or(1.0);
    let y0t = style.y_scale.transform(dy0).unwrap_or(0.0);
    let y1t = style.y_scale.transform(dy1).unwrap_or(1.0);
    let (min_x, max_x) = pad_span(x0t, x1t);
    let (min_y, max_y) = pad_span(y0t, y1t);

    // Transform points and keep only those inside the range.
    let mut pts: Vec<[f64; 2]> = Vec::with_capacity(sel.len());
    for i in 0..raw_x.len() {
        let (Some(px), Some(py)) = (style.x_scale.transform(raw_x[i]), style.y_scale.transform(raw_y[i]))
        else {
            continue;
        };
        if px < min_x || px > max_x || py < min_y || py > max_y {
            continue;
        }
        pts.push([px, py]);
    }

    let grid = if style.gradient || style.contour {
        Some(density_grid(&pts, min_x, max_x, min_y, max_y, 128, 128))
    } else {
        None
    };
    let densities = grid
        .as_ref()
        .map(|g| (g.max(), point_densities(&pts, g)));

    let plot_w = style.width as f64 - 80.0;
    let plot_h = style.height as f64 - 80.0;
    let origin_x = 60.0;
    let origin_y = style.height as f64 - 40.0;

    let sx = move |v: f64| origin_x + (v - min_x) / (max_x - min_x) * plot_w;
    let sy = move |v: f64| origin_y - (v - min_y) / (max_y - min_y) * plot_h;

    let mut svg = String::new();
    svg.push_str(&svg_header(style.width, style.height, &style.background));
    svg.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"{}\" />\n",
        style.width, style.height, style.background
    ));

    // Axes.
    svg.push_str(&format!(
        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\" />\n",
        origin_x,
        origin_y,
        origin_x + plot_w,
        origin_y,
        style.axis_color
    ));
    svg.push_str(&format!(
        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\" />\n",
        origin_x,
        origin_y,
        origin_x,
        origin_y - plot_h,
        style.axis_color
    ));

    // Title.
    if !style.title.is_empty() {
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"22\" font-family=\"{}\" font-size=\"{}\" font-weight=\"bold\" fill=\"{}\" text-anchor=\"middle\">{}</text>\n",
            style.width as f64 / 2.0,
            svg_escape(&style.label_font),
            style.title_size,
            style.text_color,
            svg_escape(&style.title)
        ));
    }

    let y_center = origin_y - plot_h / 2.0;
    // Axis labels.
    let x_anchor = (origin_x + plot_w / 2.0, style.height as f64 - 10.0);
    let y_anchor = (15.0_f64, y_center);
    let x_text = label_with_scale(x_label, style.x_scale, style.show_scale_labels);
    let y_text = label_with_scale(y_label, style.y_scale, style.show_scale_labels);
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\" text-anchor=\"middle\"{}>{}</text>\n",
        x_anchor.0,
        x_anchor.1,
        svg_escape(&style.x_label_font),
        style.x_label_size,
        style.text_color,
        rotation_attr(style.x_label_rotation, x_anchor),
        svg_escape(&x_text)
    ));
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\" text-anchor=\"middle\"{}>{}</text>\n",
        y_anchor.0,
        y_anchor.1,
        svg_escape(&style.y_label_font),
        style.y_label_size,
        style.text_color,
        rotation_attr(style.y_label_rotation, y_anchor),
        svg_escape(&y_text)
    ));

    // Tick labels.
    let xticks = 5;
    let yticks = 5;
    for t in 0..=xticks {
        let v = min_x + (max_x - min_x) * (t as f64 / xticks as f64);
        let px = sx(v);
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"10\" fill=\"{}\" text-anchor=\"middle\">{}</text>\n",
            px,
            origin_y + 15.0,
            style.text_color,
            svg_escape(&format_tick(v, style.x_scale))
        ));
        svg.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"0.5\" />\n",
            px, origin_y, px, origin_y - 5.0, style.axis_color
        ));
    }
    for t in 0..=yticks {
        let v = min_y + (max_y - min_y) * (t as f64 / yticks as f64);
        let py = sy(v);
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"10\" fill=\"{}\" text-anchor=\"end\">{}</text>\n",
            origin_x - 8.0,
            py + 4.0,
            style.text_color,
            svg_escape(&format_tick(v, style.y_scale))
        ));
        svg.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"0.5\" />\n",
            origin_x - 5.0, py, origin_x, py, style.axis_color
        ));
    }

    // Events.
    if style.contour {
        if let Some(grid) = grid.as_ref() {
            let max_density = grid.max();
            for level in contour_levels(max_density, style.contour_levels) {
                for seg in contour_segments(grid, level) {
                    let a = seg[0];
                    let b = seg[1];
                    svg.push_str(&format!(
                        "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{}\" stroke-width=\"1.2\" stroke-opacity=\"0.85\" />\n",
                        sx(a[0]),
                        sy(a[1]),
                        sx(b[0]),
                        sy(b[1]),
                        style.line_color
                    ));
                }
            }
        }
    } else {
        for (i, p) in pts.iter().enumerate() {
            let px = sx(p[0]);
            let py = sy(p[1]);
            let fill = if style.gradient {
                if let Some((max_density, dens)) = &densities {
                    if *max_density > 0.0 {
                        let t = (dens[i] / max_density).clamp(0.0, 1.0);
                        density_hex(t)
                    } else {
                        style.point_color.clone()
                    }
                } else {
                    style.point_color.clone()
                }
            } else {
                style.point_color.clone()
            };
            svg.push_str(&format!(
                "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"{}\" fill=\"{}\" fill-opacity=\"{}\" />\n",
                px, py, style.point_radius, fill, style.point_opacity
            ));
        }
    }

    // Gate overlays.
    for overlay in overlays {
        if overlay.points.len() < 2 {
            continue;
        }
        let mut path = String::new();
        for (k, p) in overlay.points.iter().enumerate() {
            let (Some(px), Some(py)) = (style.x_scale.transform(p[0]), style.y_scale.transform(p[1]))
            else {
                continue;
            };
            if k == 0 {
                path.push_str(&format!("M {:.2} {:.2}", sx(px), sy(py)));
            } else {
                path.push_str(&format!(" L {:.2} {:.2}", sx(px), sy(py)));
            }
        }
        if path.is_empty() {
            continue;
        }
        if overlay.closed {
            path.push_str(" Z");
        }
        let fill = match &overlay.fill {
            Some(f) => format!(" fill=\"{f}\" fill-opacity=\"0.2\""),
            None => " fill=\"none\"".to_string(),
        };
        svg.push_str(&format!(
            "<path d=\"{path}\" stroke=\"{}\" stroke-width=\"2\"{fill} />\n",
            overlay.stroke
        ));
    }

    svg.push_str("</svg>\n");
    svg
}

/// Renders a 1D histogram for a single parameter over a set of events.
///
/// `label` is the X-axis label; `y_label` is the Y-axis label (use `"Count"`
/// for the default). Axis-label font/rotation/size come from `style`.
#[allow(clippy::too_many_arguments)]
pub fn histogram_svg(
    data: &ndarray::Array2<f32>,
    param_idx: usize,
    label: &str,
    y_label: &str,
    indices: &[usize],
    value_range: Option<(f32, f32)>,
    bins: usize,
    style: &PlotStyle,
) -> String {
    let (_, n_params) = data.dim();
    assert!(param_idx < n_params);

    let sel: Vec<usize> = if indices.is_empty() {
        (0..data.nrows()).collect()
    } else {
        indices.to_vec()
    };

    let values: Vec<f32> = sel.iter().map(|&i| data[[i, param_idx]]).collect();
    let (min_v, max_v) = value_range.unwrap_or_else(|| {
        let (a, b) = compute_range(&values.iter().map(|v| *v as f64).collect::<Vec<_>>(), style.x_scale);
        (a as f32, b as f32)
    });
    let (min_v, max_v) = pad_range(min_v, max_v);
    let bin_count = bins.max(1);
    let bin_w = (max_v - min_v) / bin_count as f32;

    let mut counts = vec![0usize; bin_count];
    for &v in &values {
        if !v.is_finite() {
            continue;
        }
        let mut bi = ((v - min_v) / bin_w) as usize;
        if bi >= bin_count {
            bi = bin_count - 1;
        }
        counts[bi] += 1;
    }
    let max_count = counts.iter().max().copied().unwrap_or(1).max(1);

    let plot_w = style.width as f32 - 80.0;
    let plot_h = style.height as f32 - 80.0;
    let origin_x = 60.0;
    let origin_y = style.height as f32 - 40.0;

    let sx = |v: f32| origin_x + (v - min_v) / (max_v - min_v) * plot_w;
    let sy = |c: usize| origin_y - (c as f32 / max_count as f32) * plot_h;

    let mut svg = String::new();
    svg.push_str(&svg_header(style.width, style.height, &style.background));
    svg.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"{}\" />\n",
        style.width, style.height, style.background
    ));

    // Axes.
    svg.push_str(&format!(
        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\" />\n",
        origin_x, origin_y, origin_x + plot_w, origin_y, style.axis_color
    ));
    svg.push_str(&format!(
        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\" />\n",
        origin_x, origin_y, origin_x, origin_y - plot_h, style.axis_color
    ));

    // Bars.
    for (b, &c) in counts.iter().enumerate() {
        let x0 = sx(min_v + b as f32 * bin_w);
        let x1 = sx(min_v + (b + 1) as f32 * bin_w);
        let y_top = sy(c);
        svg.push_str(&format!(
            "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"{}\" />\n",
            x0,
            y_top,
            (x1 - x0).max(1.0),
            origin_y - y_top,
            style.point_color
        ));
    }

    let y_center = origin_y - plot_h / 2.0;
    let x_anchor = (origin_x + plot_w / 2.0, style.height as f32 - 10.0);
    let y_anchor = (15.0_f64, y_center as f64);
    let x_text = label_with_scale(label, style.x_scale, style.show_scale_labels);
    let y_text = if y_label.is_empty() { "Count" } else { y_label };
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\" text-anchor=\"middle\"{}>{}</text>\n",
        x_anchor.0,
        x_anchor.1,
        svg_escape(&style.x_label_font),
        style.x_label_size,
        style.text_color,
        rotation_attr(style.x_label_rotation, (x_anchor.0 as f64, x_anchor.1 as f64)),
        svg_escape(&x_text)
    ));
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\" text-anchor=\"middle\"{}>{}</text>\n",
        y_anchor.0,
        y_anchor.1,
        svg_escape(&style.y_label_font),
        style.y_label_size,
        style.text_color,
        rotation_attr(style.y_label_rotation, y_anchor),
        svg_escape(y_text)
    ));

    // Tick labels.
    let xticks = 5;
    for t in 0..=xticks {
        let v = min_v + (max_v - min_v) * (t as f32 / xticks as f32);
        let px = sx(v);
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"10\" fill=\"{}\" text-anchor=\"middle\">{:.1}</text>\n",
            px,
            origin_y + 15.0,
            style.text_color,
            v
        ));
    }

    svg.push_str("</svg>\n");
    svg
}

// ---------------------------------------------------------------------------
// Density / contour analysis
// ---------------------------------------------------------------------------

/// A regular 2D grid of event counts, used for density colouring and contours.
#[derive(Debug, Clone)]
pub struct DensityGrid {
    pub nx: usize,
    pub ny: usize,
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
    /// Row-major densities, index `j * nx + i` (i over x, j over y).
    pub values: Vec<f64>,
}

impl DensityGrid {
    /// Value at grid node `(i, j)`, clamped to the grid extents.
    pub fn at(&self, i: usize, j: usize) -> f64 {
        let i = i.min(self.nx.saturating_sub(1));
        let j = j.min(self.ny.saturating_sub(1));
        self.values[j * self.nx + i]
    }

    pub fn max(&self) -> f64 {
        self.values.iter().copied().fold(0.0, f64::max)
    }

    fn x_at(&self, i: usize) -> f64 {
        if self.nx <= 1 {
            self.x0
        } else {
            self.x0 + (self.x1 - self.x0) * i as f64 / (self.nx - 1) as f64
        }
    }

    fn y_at(&self, j: usize) -> f64 {
        if self.ny <= 1 {
            self.y0
        } else {
            self.y0 + (self.y1 - self.y0) * j as f64 / (self.ny - 1) as f64
        }
    }
}

/// Build a histogram density grid over the given rectangle.
pub fn density_grid(
    points: &[[f64; 2]],
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
    nx: usize,
    ny: usize,
) -> DensityGrid {
    let nx = nx.max(2);
    let ny = ny.max(2);
    let w = if (x1 - x0).abs() < f64::EPSILON {
        1.0
    } else {
        x1 - x0
    };
    let h = if (y1 - y0).abs() < f64::EPSILON {
        1.0
    } else {
        y1 - y0
    };
    let mut values = vec![0.0f64; nx * ny];
    for p in points {
        if !p[0].is_finite() || !p[1].is_finite() {
            continue;
        }
        let fx = (p[0] - x0) / w * (nx - 1) as f64;
        let fy = (p[1] - y0) / h * (ny - 1) as f64;
        let i = fx.round().clamp(0.0, (nx - 1) as f64) as usize;
        let j = fy.round().clamp(0.0, (ny - 1) as f64) as usize;
        values[j * nx + i] += 1.0;
    }
    DensityGrid {
        nx,
        ny,
        x0,
        x1: x0 + w,
        y0,
        y1: y0 + h,
        values,
    }
}

/// Bilinearly sample the density at each point (used for gradient colouring).
pub fn point_densities(points: &[[f64; 2]], grid: &DensityGrid) -> Vec<f64> {
    let w = if (grid.x1 - grid.x0).abs() < f64::EPSILON {
        1.0
    } else {
        grid.x1 - grid.x0
    };
    let h = if (grid.y1 - grid.y0).abs() < f64::EPSILON {
        1.0
    } else {
        grid.y1 - grid.y0
    };
    points
        .iter()
        .map(|p| {
            let fx = (p[0] - grid.x0) / w * (grid.nx - 1) as f64;
            let fy = (p[1] - grid.y0) / h * (grid.ny - 1) as f64;
            let i = fx.floor();
            let j = fy.floor();
            let tx = fx - i;
            let ty = fy - j;
            let i0 = i.clamp(0.0, (grid.nx - 2) as f64) as usize;
            let j0 = j.clamp(0.0, (grid.ny - 2) as f64) as usize;
            let v00 = grid.at(i0, j0);
            let v10 = grid.at(i0 + 1, j0);
            let v01 = grid.at(i0, j0 + 1);
            let v11 = grid.at(i0 + 1, j0 + 1);
            let a = v00 * (1.0 - tx) + v10 * tx;
            let b = v01 * (1.0 - tx) + v11 * tx;
            a * (1.0 - ty) + b * ty
        })
        .collect()
}

/// Choose `n` evenly spaced contour levels between 0 and `max`.
pub fn contour_levels(max: f64, n: usize) -> Vec<f64> {
    let n = n.max(1);
    (1..=n)
        .map(|k| max * k as f64 / (n + 1) as f64)
        .collect()
}

/// Extract straight contour segments at `level` using marching squares.
///
/// Each returned segment is a pair of interpolated endpoints in the grid's
/// coordinate space.
pub fn contour_segments(grid: &DensityGrid, level: f64) -> Vec<[[f64; 2]; 2]> {
    let mut segments = Vec::new();
    for j in 0..grid.ny.saturating_sub(1) {
        for i in 0..grid.nx.saturating_sub(1) {
            let v00 = grid.at(i, j);
            let v10 = grid.at(i + 1, j);
            let v11 = grid.at(i + 1, j + 1);
            let v01 = grid.at(i, j + 1);

            let x0 = grid.x_at(i);
            let x1 = grid.x_at(i + 1);
            let y0 = grid.y_at(j);
            let y1 = grid.y_at(j + 1);

            let idx = (v00 > level) as u8
                | ((v10 > level) as u8) << 1
                | ((v11 > level) as u8) << 2
                | ((v01 > level) as u8) << 3;
            if idx == 0 || idx == 15 {
                continue;
            }

            let interp = |a: f64, b: f64| -> f64 {
                if (b - a).abs() < f64::EPSILON {
                    0.5
                } else {
                    ((level - a) / (b - a)).clamp(0.0, 1.0)
                }
            };
            // Edge midpoints (edge ids: 0 bottom, 1 right, 2 top, 3 left).
            let edge = |e: u8| -> [f64; 2] {
                match e {
                    0 => {
                        let t = interp(v00, v10);
                        [x0 + (x1 - x0) * t, y0]
                    }
                    1 => {
                        let t = interp(v10, v11);
                        [x1, y0 + (y1 - y0) * t]
                    }
                    2 => {
                        let t = interp(v01, v11);
                        [x0 + (x1 - x0) * t, y1]
                    }
                    _ => {
                        let t = interp(v00, v01);
                        [x0, y0 + (y1 - y0) * t]
                    }
                }
            };

            let pairs: &[(u8, u8)] = match idx {
                1 => &[(3, 0)],
                2 => &[(0, 1)],
                3 => &[(3, 1)],
                4 => &[(1, 2)],
                5 => &[(3, 0), (1, 2)],
                6 => &[(0, 2)],
                7 => &[(3, 2)],
                8 => &[(3, 2)],
                9 => &[(0, 2)],
                10 => &[(0, 1), (2, 3)],
                11 => &[(1, 2)],
                12 => &[(3, 1)],
                13 => &[(0, 1)],
                14 => &[(3, 0)],
                _ => &[],
            };
            for (a, b) in pairs {
                segments.push([edge(*a), edge(*b)]);
            }
        }
    }
    segments
}

/// Maps a normalized density `t` in `[0, 1]` to an RGB triple (blue -> cyan ->
/// yellow -> red). Shared by the GUI and SVG renderers so both look identical.
pub fn density_rgb(t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    // Piecewise-linear colour ramp.
    let stops: [(f64, (f64, f64, f64)); 4] = [
        (0.0, (0.0, 0.0, 140.0)),
        (0.35, (0.0, 190.0, 220.0)),
        (0.7, (250.0, 220.0, 40.0)),
        (1.0, (220.0, 20.0, 20.0)),
    ];
    for w in stops.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t >= t0 && t <= t1 {
            let f = if (t1 - t0).abs() < f64::EPSILON {
                0.0
            } else {
                (t - t0) / (t1 - t0)
            };
            let r = c0.0 + (c1.0 - c0.0) * f;
            let g = c0.1 + (c1.1 - c0.1) * f;
            let b = c0.2 + (c1.2 - c0.2) * f;
            return (r.round() as u8, g.round() as u8, b.round() as u8);
        }
    }
    (220, 20, 20)
}

/// Converts a normalized density to a `#rrggbb` string.
pub fn density_hex(t: f64) -> String {
    let (r, g, b) = density_rgb(t);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Parses a `#rrggbb` (or `#rgb`) colour into an RGB triple.
pub fn parse_hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#');
    match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some((r, g, b))
        }
        3 => {
            let r = u8::from_str_radix(&s[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&s[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&s[2..3], 16).ok()? * 17;
            Some((r, g, b))
        }
        _ => None,
    }
}

fn compute_range(values: &[f64], scale: AxisScale) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &v in values {
        let v = match scale {
            AxisScale::Linear => v,
            AxisScale::Log => {
                if v > 0.0 {
                    v
                } else {
                    continue;
                }
            }
        };
        if v.is_finite() {
            min = min.min(v);
            max = max.max(v);
        }
    }
    if !min.is_finite() {
        (1.0, 10.0)
    } else if (min - max).abs() < f64::EPSILON {
        if min.abs() > 1.0 {
            (min * 0.5, min * 1.5)
        } else {
            (min - 1.0, max + 1.0)
        }
    } else {
        (min, max)
    }
}

fn pad_span(min: f64, max: f64) -> (f64, f64) {
    if !min.is_finite() || !max.is_finite() || (max - min).abs() < f64::EPSILON {
        return (-1.0, 1.0);
    }
    let pad = (max - min) * 0.05;
    (min - pad, max + pad)
}

fn pad_range(min: f32, max: f32) -> (f32, f32) {
    let span = max - min;
    let pad = if span.abs() < f32::EPSILON { 1.0 } else { span * 0.05 };
    (min - pad, max + pad)
}

fn svg_header(width: u32, height: u32, background: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" style="background:{background}">"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn format_tick_uses_compact_scientific_notation() {
        // Regression: the exponent marker used to be duplicated ("7eee4").
        assert_eq!(format_tick(70000.0, AxisScale::Linear), "7e4");
        assert_eq!(format_tick(1_000_000.0, AxisScale::Linear), "1e6");
        assert_eq!(format_tick(1e7, AxisScale::Linear), "1e7");
        assert_eq!(format_tick(5_300_871.0, AxisScale::Linear), "5.3e6");
        assert_eq!(format_tick(-70000.0, AxisScale::Linear), "-7e4");
        assert_eq!(format_tick(1e-3, AxisScale::Linear), "1e-3");
        // Log scale receives the exponent and raises it back.
        assert_eq!(format_tick(6.0, AxisScale::Log), "1e6");
        // Small / mid values stay in plain notation.
        assert_eq!(format_tick(123.0, AxisScale::Linear), "123");
        assert_eq!(format_tick(1.5, AxisScale::Linear), "1.50");
    }

    #[test]
    fn dot_plot_svg_renders() {
        let data = array![[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]];
        let svg = dot_plot_svg(
            &data,
            0,
            1,
            "FSC",
            "SSC",
            &[],
            None,
            None,
            &[],
            &PlotStyle::default(),
        );
        assert!(svg.contains("<svg"));
        assert!(svg.contains("FSC"));
        assert!(svg.contains("SSC"));
        assert!(svg.contains("<circle"));
    }

    #[test]
    fn log_axis_dot_plot_renders() {
        let data = array![[1.0, 10.0], [100.0, 1000.0]];
        let mut style = PlotStyle::default();
        style.x_scale = AxisScale::Log;
        style.y_scale = AxisScale::Log;
        style.gradient = true;
        let svg = dot_plot_svg(&data, 0, 1, "A", "B", &[], None, None, &[], &style);
        assert!(svg.contains("Log"));
    }

    #[test]
    fn contour_segments_are_produced() {
        // A tight cluster should produce a closed contour.
        let mut pts = Vec::new();
        for _ in 0..1000 {
            pts.push([0.0, 0.0]);
        }
        let grid = density_grid(&pts, -1.0, 1.0, -1.0, 1.0, 32, 32);
        assert!(grid.max() > 0.0);
        let segs = contour_segments(&grid, grid.max() * 0.5);
        assert!(!segs.is_empty());
    }

    #[test]
    fn density_colour_ramp_is_monotonic_channels() {
        let (r0, _, b0) = density_rgb(0.0);
        let (r1, _, b1) = density_rgb(1.0);
        assert!(r1 > r0);
        assert!(b0 > b1);
    }

    #[test]
    fn histogram_svg_renders() {
        let data = array![[1.0], [2.0], [2.0], [3.0]];
        let svg = histogram_svg(&data, 0, "FSC", "Count", &[], None, 10, &PlotStyle::default());
        assert!(svg.contains("<svg"));
        assert!(svg.contains("FSC"));
        assert!(svg.contains("<rect"));
    }
}
