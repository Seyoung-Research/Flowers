//! Minimal SVG rendering for flow cytometry plots.
//!
//! Produces standalone `.svg` files (dot plots and 1D histograms) without any
//! external graphics dependency, so `flowers` can run headless.


use serde::Serialize;

/// Styling for a rendered plot.
#[derive(Debug, Clone, Serialize)]
pub struct PlotStyle {
    pub width: u32,
    pub height: u32,
    pub point_radius: f32,
    pub point_opacity: f32,
    pub point_color: &'static str,
    pub line_color: &'static str,
    pub axis_color: &'static str,
    pub text_color: &'static str,
    pub background: &'static str,
}

impl Default for PlotStyle {
    fn default() -> Self {
        Self {
            width: 500,
            height: 450,
            point_radius: 2.0,
            point_opacity: 0.6,
            point_color: "#1f77b4",
            line_color: "#2ca02c",
            axis_color: "#444444",
            text_color: "#222222",
            background: "#ffffff",
        }
    }
}

/// Escapes a string for safe inclusion in an SVG text node.
fn svg_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Renders a 2D dot plot of two parameters for a set of events.
///
/// `data` has shape `(events, parameters)`. `x_idx` / `y_idx` select the
/// columns. `indices` selects which events to draw (if empty, all events).
/// `x_range` / `y_range` are the axis limits; if `None` they are computed from
/// the selected data.
pub fn dot_plot_svg(
    data: &ndarray::Array2<f32>,
    x_idx: usize,
    y_idx: usize,
    x_label: &str,
    y_label: &str,
    indices: &[usize],
    x_range: Option<(f32, f32)>,
    y_range: Option<(f32, f32)>,
    style: &PlotStyle,
) -> String {
    let (_, n_params) = data.dim();
    assert!(x_idx < n_params && y_idx < n_params);

    let sel: Vec<usize> = if indices.is_empty() {
        (0..data.nrows()).collect()
    } else {
        indices.to_vec()
    };

    let xs: Vec<f32> = sel.iter().map(|&i| data[[i, x_idx]]).collect();
    let ys: Vec<f32> = sel.iter().map(|&i| data[[i, y_idx]]).collect();

    let (min_x, max_x) = x_range.unwrap_or_else(|| compute_range(&xs));
    let (min_y, max_y) = y_range.unwrap_or_else(|| compute_range(&ys));

    let (min_x, max_x) = pad_range(min_x, max_x);
    let (min_y, max_y) = pad_range(min_y, max_y);

    let plot_w = style.width as f32 - 80.0;
    let plot_h = style.height as f32 - 80.0;
    let origin_x = 60.0;
    let origin_y = style.height as f32 - 40.0;

    let sx = |v: f32| origin_x + (v - min_x) / (max_x - min_x) * plot_w;
    let sy = |v: f32| origin_y - (v - min_y) / (max_y - min_y) * plot_h;

    let mut svg = String::new();
    svg.push_str(&svg_header(style.width, style.height, style.background));
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

    let y_center = origin_y - plot_h / 2.0;
    // Axis labels.
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-size=\"12\" fill=\"{}\" text-anchor=\"middle\">{}</text>\n",
        origin_x + plot_w / 2.0,
        style.height as f32 - 10.0,
        style.text_color,
        svg_escape(x_label)
    ));
    svg.push_str(&format!(
        "<text x=\"15\" y=\"{}\" font-size=\"12\" fill=\"{}\" text-anchor=\"middle\" transform=\"rotate(-90 15,{})\">{}</text>\n",
        y_center,
        style.text_color,
        y_center,
        svg_escape(y_label)
    ));

    // Tick labels.
    let xticks = 5;
    let yticks = 5;
    for t in 0..=xticks {
        let v = min_x + (max_x - min_x) * (t as f32 / xticks as f32);
        let px = sx(v);
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"10\" fill=\"{}\" text-anchor=\"middle\">{:.1}</text>\n",
            px,
            origin_y + 15.0,
            style.text_color,
            v
        ));
        svg.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"0.5\" />\n",
            px,
            origin_y,
            px,
            origin_y - 5.0,
            style.axis_color
        ));
    }
    for t in 0..=yticks {
        let v = min_y + (max_y - min_y) * (t as f32 / yticks as f32);
        let py = sy(v);
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"10\" fill=\"{}\" text-anchor=\"end\">{:.1}</text>\n",
            origin_x - 8.0,
            py + 4.0,
            style.text_color,
            v
        ));
        svg.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"0.5\" />\n",
            origin_x - 5.0,
            py,
            origin_x,
            py,
            style.axis_color
        ));
    }

    // Points.
    for (&x, &y) in xs.iter().zip(ys.iter()) {
        let px = sx(x);
        let py = sy(y);
        if px < 0.0 || px > style.width as f32 || py < 0.0 || py > style.height as f32 {
            continue;
        }
        svg.push_str(&format!(
            "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"{}\" fill=\"{}\" fill-opacity=\"{}\" />\n",
            px,
            py,
            style.point_radius,
            style.point_color,
            style.point_opacity
        ));
    }

    svg.push_str("</svg>\n");
    svg
}

/// Renders a 1D histogram for a single parameter over a set of events.
pub fn histogram_svg(
    data: &ndarray::Array2<f32>,
    param_idx: usize,
    label: &str,
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
    let (min_v, max_v) = value_range.unwrap_or_else(|| compute_range(&values));
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
    svg.push_str(&svg_header(style.width, style.height, style.background));
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
    // Axis labels.
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-size=\"12\" fill=\"{}\" text-anchor=\"middle\">{}</text>\n",
        origin_x + plot_w / 2.0,
        style.height as f32 - 10.0,
        style.text_color,
        svg_escape(label)
    ));
    svg.push_str(&format!(
        "<text x=\"15\" y=\"{}\" font-size=\"12\" fill=\"{}\" text-anchor=\"middle\" transform=\"rotate(-90 15,{})\">Count</text>\n",
        y_center,
        style.text_color,
        y_center
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

fn compute_range(values: &[f32]) -> (f32, f32) {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for &v in values {
        if v.is_finite() {
            min = min.min(v);
            max = max.max(v);
        }
    }
    if !min.is_finite() {
        (0.0, 1.0)
    } else if min == max {
        (min - 1.0, max + 1.0)
    } else {
        (min, max)
    }
}

fn pad_range(min: f32, max: f32) -> (f32, f32) {
    let span = max - min;
    let pad = span * 0.05;
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
            &PlotStyle::default(),
        );
        assert!(svg.contains("<svg"));
        assert!(svg.contains("FSC"));
        assert!(svg.contains("SSC"));
        assert!(svg.contains("<circle"));
    }

    #[test]
    fn histogram_svg_renders() {
        let data = array![[1.0], [2.0], [2.0], [3.0]];
        let svg = histogram_svg(&data, 0, "FSC", &[], None, 10, &PlotStyle::default());
        assert!(svg.contains("<svg"));
        assert!(svg.contains("FSC"));
        assert!(svg.contains("<rect"));
    }
}
