//! egui-based GUI for `flowers`.
//!
//! Provides a desktop application for loading FCS files, drawing gates on
//! scatter plots (rect / ellipse / quadrant / bilinear), organising gates as a
//! hierarchical tree (like FlowJo), applying those gates to every loaded
//! sample, exporting CSV reports, and arranging plots in a "layout editor".
//!
//! Feature map (see `FEATURES.md`):
//! * fixed-size plotting window with editable X/Y ranges
//! * per-axis "Auto" scaling button
//! * per-axis linear / logarithmic scale
//! * configurable dot colour
//! * density gradient layering (points recoloured where they overlap)
//! * detachable window that shows only a gated population
//! * dots vs. topographic (contour) view
//! * gates applied across all loaded FCS files
//! * per-file "apply existing gates?" prompt when a new file is loaded
//! * default X/Y axes picked as FSC-H / SSC-H when present
//! * hierarchical gate tree with counts and percentages
//! * CSV export of counts / percentages
//! * quadrant gate (draw or click)
//! * bilinear / line gate (the region under a drawn line)
//! * dark / light theme switch
//! * detached population window with the full plotting & gating toolkit
//! * interactive gate manipulation (move / rotate / enlarge / shrink)
//! * free-form visual layout editor (drag, resize, reorder, preview)
//! * editable axis labels in the layout editor (text, size, rotation, font, colour)
//! * figure export to SVG, PNG, JPEG and TIFF

use eframe::egui;
use egui_plot::{Bar, BarChart, Line, Plot, PlotBounds, Points, Polygon};

use crate::export::{write_populations_csv, PopulationRow};
use crate::fcs::{FcsFile, FcsParser};
use crate::gating::{
    EllipseGate, Gate as GatingGate, LineGate, Quadrant, QuadrantGate, RectangularGate,
};
use crate::layout::{
    render_layout_free, LayoutOptions, LayoutPlacement,
};
use crate::plot::{
    density_grid, density_rgb, dot_plot_svg, point_densities, AxisScale, PlotOverlay, PlotStyle,
};
use crate::raster::{save_svg_as_image, ImageFormat};
use crate::project::{
    GateShape, Project, ProjectGate, ProjectLabel, ProjectPanel, ProjectSample, ViewSettings,
};

// ---------------------------------------------------------------------------
// Gate model
// ---------------------------------------------------------------------------

/// View mode for a plot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ViewMode {
    #[default]
    Dots,
    Contour,
    Histogram,
}

impl ViewMode {
    fn label(self) -> &'static str {
        match self {
            ViewMode::Dots => "Dots",
            ViewMode::Contour => "Contour",
            ViewMode::Histogram => "Histogram",
        }
    }
}

/// Identifies a draggable manipulation handle on a gate.
#[derive(Debug, Clone, PartialEq)]
enum HandleId {
    /// Corner handle (0..4) for uniform scale about the centre.
    Corner(usize),
    /// Edge handle (0..3) for single-axis stretch of an ellipse.
    Edge(usize),
    /// Rotation handle.
    Rotate,
    /// Polygon vertex handle.
    Vertex(usize),
    /// Centre handle (move a split point / bilinear centre).
    Center,
}

/// A manipulation target chosen when a drag begins on a selected gate.
#[derive(Debug, Clone, PartialEq)]
enum ManipTarget {
    Move,
    Handle(HandleId),
}

/// The shape/geometry of a gate.
#[derive(Debug, Clone, PartialEq)]
enum GateKind {
    Rect {
        x_min: f64,
        x_max: f64,
        y_min: f64,
        y_max: f64,
        /// Rotation about the bounding-box centre, in radians.
        angle: f64,
    },
    Ellipse {
        x_min: f64,
        x_max: f64,
        y_min: f64,
        y_max: f64,
        /// Rotation about the bounding-box centre, in radians.
        angle: f64,
    },
    /// Free-form polygon (vertices in data coordinates).
    Polygon {
        vertices: Vec<[f64; 2]>,
    },
    /// One wedge of a 4-way quadrant split.
    Quadrant {
        x: f64,
        y: f64,
        quad: Quadrant,
    },
    /// A line gate: events on one side of a drawn line segment, restricted to
    /// the segment's horizontal extent (the "region under the line").
    Bilinear {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        /// When true, points below the line are selected; false selects above.
        below: bool,
    },
}

impl GateKind {
    fn label(&self) -> &'static str {
        match self {
            GateKind::Rect { .. } => "Rect",
            GateKind::Ellipse { .. } => "Ellipse",
            GateKind::Polygon { .. } => "Polygon",
            GateKind::Quadrant { .. } => "Quadrant",
            GateKind::Bilinear { .. } => "Bilinear",
        }
    }

    /// Build the trait object used for event filtering.
    fn to_gate(&self, x_idx: usize, y_idx: usize) -> Box<dyn GatingGate> {
        match self {
            GateKind::Rect {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
            } => Box::new(
                RectangularGate::new(
                    x_idx,
                    y_idx,
                    *x_min as f32,
                    *x_max as f32,
                    *y_min as f32,
                    *y_max as f32,
                )
                .with_angle(*angle as f32),
            ),
            GateKind::Ellipse {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
            } => Box::new(
                EllipseGate::new(
                    x_idx,
                    y_idx,
                    ((x_min + x_max) / 2.0) as f32,
                    ((y_min + y_max) / 2.0) as f32,
                    ((x_max - x_min) / 2.0) as f32,
                    ((y_max - y_min) / 2.0) as f32,
                )
                .with_angle(*angle as f32),
            ),
            GateKind::Polygon { vertices } => Box::new(crate::gating::PolygonGate::new(
                x_idx,
                y_idx,
                vertices.iter().map(|p| (p[0] as f32, p[1] as f32)).collect(),
            )),
            GateKind::Quadrant { x, y, quad } => {
                Box::new(QuadrantGate::new(x_idx, y_idx, *x as f32, *y as f32, quad.clone()))
            }
            GateKind::Bilinear {
                x1,
                y1,
                x2,
                y2,
                below,
            } => Box::new(LineGate::new(
                x_idx,
                y_idx,
                *x1 as f32,
                *y1 as f32,
                *x2 as f32,
                *y2 as f32,
                *below,
            )),
        }
    }

    /// Closed outline polygon in data coordinates (for rendering/filling).
    ///
    /// `x_range`/`y_range` are needed to clip the unbounded wedge gates.
    fn polygon(&self, x_range: (f64, f64), y_range: (f64, f64)) -> Vec<[f64; 2]> {
        let (x0, x1) = x_range;
        let (y0, y1) = y_range;
        match self {
            GateKind::Rect {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
            } => {
                let cx = (x_min + x_max) / 2.0;
                let cy = (y_min + y_max) / 2.0;
                let hw = (x_max - x_min) / 2.0;
                let hh = (y_max - y_min) / 2.0;
                let (s, c) = angle.sin_cos();
                let mut pts: Vec<[f64; 2]> = [
                    [-hw, -hh],
                    [hw, -hh],
                    [hw, hh],
                    [-hw, hh],
                ]
                .iter()
                .map(|[lx, ly]| [cx + lx * c - ly * s, cy + lx * s + ly * c])
                .collect();
                pts.push(pts[0]);
                pts
            }
            GateKind::Ellipse {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
            } => {
                let cx = (x_min + x_max) / 2.0;
                let cy = (y_min + y_max) / 2.0;
                let rx = (x_max - x_min) / 2.0;
                let ry = (y_max - y_min) / 2.0;
                let (s, c) = angle.sin_cos();
                let n = 48;
                (0..=n)
                    .map(|i| {
                        let a = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
                        let lx = rx * a.cos();
                        let ly = ry * a.sin();
                        [cx + lx * c - ly * s, cy + lx * s + ly * c]
                    })
                    .collect()
            }
            GateKind::Polygon { vertices } => {
                let mut pts = vertices.clone();
                if pts.len() >= 3 {
                    pts.push(pts[0]);
                }
                pts
            }
            GateKind::Quadrant { x, y, quad } => {
                let rect = vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
                let half = match quad {
                    Quadrant::UpperLeft => vec![[x0, *y], [*x, *y], [*x, y1], [x0, y1]],
                    Quadrant::UpperRight => vec![[*x, *y], [x1, *y], [x1, y1], [*x, y1]],
                    Quadrant::LowerLeft => vec![[x0, y0], [*x, y0], [*x, *y], [x0, *y]],
                    Quadrant::LowerRight => vec![[*x, y0], [x1, y0], [x1, *y], [*x, *y]],
                };
                let _ = rect;
                let mut p = half;
                if p.len() >= 3 {
                    p.push(p[0]);
                }
                p
            }
            GateKind::Bilinear {
                x1: lx1,
                y1: ly1,
                x2: lx2,
                y2: ly2,
                below,
            } => {
                let (xa, xb) = effective_x_band(*lx1, *lx2, x0, x1);
                let ya = line_y_at(*lx1, *ly1, *lx2, *ly2, xa).clamp(y0, y1);
                let yb = line_y_at(*lx1, *ly1, *lx2, *ly2, xb).clamp(y0, y1);
                let y_edge = if *below { y0 } else { y1 };
                vec![
                    [xa, ya],
                    [xb, yb],
                    [xb, y_edge],
                    [xa, y_edge],
                    [xa, ya],
                ]
            }
        }
    }

    /// Divider lines (quadrant crosshair, bilinear crossing lines).
    fn divider_lines(&self, x_range: (f64, f64), y_range: (f64, f64)) -> Vec<Vec<[f64; 2]>> {
        let (x0, x1) = x_range;
        let (y0, y1) = y_range;
        match self {
            GateKind::Quadrant { x, y, .. } => vec![
                vec![[*x, y0], [*x, y1]],
                vec![[x0, *y], [x1, *y]],
            ],
            GateKind::Bilinear {
                x1: lx1,
                y1: ly1,
                x2: lx2,
                y2: ly2,
                ..
            } => {
                vec![vec![[*lx1, *ly1], [*lx2, *ly2]]]
            }
            _ => Vec::new(),
        }
    }

    /// Short human-readable description for reports.
    fn summary(&self) -> String {
        match self {
            GateKind::Rect {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
            } => format!(
                "rect x[{x_min:.2},{x_max:.2}] y[{y_min:.2},{y_max:.2}] rot={:.1}deg",
                angle.to_degrees()
            ),
            GateKind::Ellipse {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
            } => format!(
                "ellipse x[{x_min:.2},{x_max:.2}] y[{y_min:.2},{y_max:.2}] rot={:.1}deg",
                angle.to_degrees()
            ),
            GateKind::Polygon { vertices } => {
                format!("polygon ({} vertices)", vertices.len())
            }
            GateKind::Quadrant { x, y, quad } => {
                format!("quadrant {quad:?} split x={x:.2} y={y:.2}")
            }
            GateKind::Bilinear {
                x1,
                y1,
                x2,
                y2,
                below,
            } => format!(
                "line ({x1:.2},{y1:.2})-({x2:.2},{y2:.2}) {}",
                if *below { "below" } else { "above" }
            ),
        }
    }

    /// Centre of the gate, used as the pivot for rotate / scale.
    fn center(&self) -> [f64; 2] {
        match self {
            GateKind::Rect {
                x_min,
                x_max,
                y_min,
                y_max,
                ..
            }
            | GateKind::Ellipse {
                x_min,
                x_max,
                y_min,
                y_max,
                ..
            } => [(x_min + x_max) / 2.0, (y_min + y_max) / 2.0],
            GateKind::Quadrant { x, y, .. } => [*x, *y],
            GateKind::Bilinear { x1, y1, x2, y2, .. } => [(x1 + x2) / 2.0, (y1 + y2) / 2.0],
            GateKind::Polygon { vertices, .. } => centroid(vertices),
        }
    }

    /// Translate the whole gate by `(dx, dy)`.
    fn translate(&mut self, dx: f64, dy: f64) {
        match self {
            GateKind::Rect {
                x_min,
                x_max,
                y_min,
                y_max,
                ..
            }
            | GateKind::Ellipse {
                x_min,
                x_max,
                y_min,
                y_max,
                ..
            } => {
                *x_min += dx;
                *x_max += dx;
                *y_min += dy;
                *y_max += dy;
            }
            GateKind::Quadrant { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            GateKind::Bilinear { x1, y1, x2, y2, .. } => {
                *x1 += dx;
                *x2 += dx;
                *y1 += dy;
                *y2 += dy;
            }
            GateKind::Polygon { vertices, .. } => {
                for v in vertices {
                    v[0] += dx;
                    v[1] += dy;
                }
            }
        }
    }

    /// Rotate the gate about `center` by `delta` radians.
    fn rotate_about(&mut self, center: [f64; 2], delta: f64) {
        match self {
            GateKind::Rect { angle, .. } | GateKind::Ellipse { angle, .. } => *angle += delta,
            // Quadrants are defined by axis-aligned splits; rotation does not apply.
            GateKind::Quadrant { .. } => {}
            GateKind::Bilinear {
                x1, y1, x2, y2, ..
            } => {
                let p1 = rotate_point([*x1, *y1], center, delta);
                let p2 = rotate_point([*x2, *y2], center, delta);
                *x1 = p1[0];
                *y1 = p1[1];
                *x2 = p2[0];
                *y2 = p2[1];
            }
            GateKind::Polygon { vertices, .. } => {
                for v in vertices {
                    *v = rotate_point(*v, center, delta);
                }
            }
        }
    }

    /// Scale the gate about `center` by `factor`.
    fn scale_about(&mut self, center: [f64; 2], factor: f64) {
        let f = factor.abs().max(1e-6);
        match self {
            GateKind::Rect {
                x_min,
                x_max,
                y_min,
                y_max,
                ..
            }
            | GateKind::Ellipse {
                x_min,
                x_max,
                y_min,
                y_max,
                ..
            } => {
                *x_min = center[0] + (*x_min - center[0]) * f;
                *x_max = center[0] + (*x_max - center[0]) * f;
                *y_min = center[1] + (*y_min - center[1]) * f;
                *y_max = center[1] + (*y_max - center[1]) * f;
            }
            GateKind::Quadrant { x, y, .. } => {
                *x = center[0] + (*x - center[0]) * f;
                *y = center[1] + (*y - center[1]) * f;
            }
            GateKind::Bilinear {
                x1, y1, x2, y2, ..
            } => {
                *x1 = center[0] + (*x1 - center[0]) * f;
                *y1 = center[1] + (*y1 - center[1]) * f;
                *x2 = center[0] + (*x2 - center[0]) * f;
                *y2 = center[1] + (*y2 - center[1]) * f;
            }
            GateKind::Polygon { vertices, .. } => {
                for v in vertices {
                    v[0] = center[0] + (v[0] - center[0]) * f;
                    v[1] = center[1] + (v[1] - center[1]) * f;
                }
            }
        }
    }

    /// Draggable handles for interactive manipulation, in data coordinates.
    fn handles(&self) -> Vec<([f64; 2], HandleId)> {
        match self {
            GateKind::Rect {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
            } => {
                let cx = (x_min + x_max) / 2.0;
                let cy = (y_min + y_max) / 2.0;
                let hw = (x_max - x_min) / 2.0;
                let hh = (y_max - y_min) / 2.0;
                let (s, c) = angle.sin_cos();
                let world = |lx: f64, ly: f64| [cx + lx * c - ly * s, cy + lx * s + ly * c];
                let margin = hh.abs().max(hw.abs()) * 0.15 + 1e-9;
                vec![
                    (world(-hw, -hh), HandleId::Corner(0)),
                    (world(hw, -hh), HandleId::Corner(1)),
                    (world(hw, hh), HandleId::Corner(2)),
                    (world(-hw, hh), HandleId::Corner(3)),
                    (world(0.0, hh + margin), HandleId::Rotate),
                ]
            }
            GateKind::Ellipse {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
            } => {
                let cx = (x_min + x_max) / 2.0;
                let cy = (y_min + y_max) / 2.0;
                let hw = (x_max - x_min) / 2.0;
                let hh = (y_max - y_min) / 2.0;
                let _diag = (hw * hw + hh * hh).sqrt().max(1e-9);
                let (s, c) = angle.sin_cos();
                let world = |lx: f64, ly: f64| [cx + lx * c - ly * s, cy + lx * s + ly * c];
                let margin = hh.abs().max(hw.abs()) * 0.15 + 1e-9;
                let inv_sqrt2 = 1.0 / 2.0f64.sqrt();
                // Corner handles sit ON the ellipse at 45-degree points (easy to grab).
                let mut out: Vec<([f64; 2], HandleId)> = vec![
                    (world(hw * inv_sqrt2, hh * inv_sqrt2), HandleId::Corner(0)),
                    (world(-hw * inv_sqrt2, hh * inv_sqrt2), HandleId::Corner(1)),
                    (world(-hw * inv_sqrt2, -hh * inv_sqrt2), HandleId::Corner(2)),
                    (world(hw * inv_sqrt2, -hh * inv_sqrt2), HandleId::Corner(3)),
                ];
                // Edge handles for single-axis stretch (left, top, right, bottom).
                out.push((world(hw, 0.0), HandleId::Edge(0)));
                out.push((world(0.0, hh), HandleId::Edge(1)));
                out.push((world(-hw, 0.0), HandleId::Edge(2)));
                out.push((world(0.0, -hh), HandleId::Edge(3)));
                // Rotation handle above the ellipse.
                out.push((world(0.0, hh + margin), HandleId::Rotate));
                out
            }
            GateKind::Polygon { vertices, .. } => {
                let c = centroid(vertices);
                let max_y = vertices.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
                let span = (max_y - c[1]).abs().max(1e-9);
                let mut out: Vec<([f64; 2], HandleId)> = vertices
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (*p, HandleId::Vertex(i)))
                    .collect();
                out.push(([c[0], max_y + span * 0.2], HandleId::Rotate));
                out
            }
            GateKind::Bilinear { x1, y1, x2, y2, .. } => vec![
                ([*x1, *y1], HandleId::Vertex(0)),
                ([*x2, *y2], HandleId::Vertex(1)),
                ([(x1 + x2) / 2.0, (y1 + y2) / 2.0], HandleId::Center),
            ],
            GateKind::Quadrant { x, y, .. } => vec![([*x, *y], HandleId::Center)],
        }
    }

    /// Point-in-gate test (used to decide "move" vs "create" on drag).
    fn contains_point(&self, p: [f64; 2]) -> bool {
        let (x, y) = (p[0], p[1]);
        match self {
            GateKind::Rect {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
            } => {
                let cx = (x_min + x_max) / 2.0;
                let cy = (y_min + y_max) / 2.0;
                let hw = (x_max - x_min) / 2.0;
                let hh = (y_max - y_min) / 2.0;
                let (s, c) = angle.sin_cos();
                let lx = (x - cx) * c + (y - cy) * s;
                let ly = -(x - cx) * s + (y - cy) * c;
                lx.abs() <= hw && ly.abs() <= hh
            }
            GateKind::Ellipse {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
            } => {
                let cx = (x_min + x_max) / 2.0;
                let cy = (y_min + y_max) / 2.0;
                let rx = (x_max - x_min) / 2.0;
                let ry = (y_max - y_min) / 2.0;
                let (s, c) = angle.sin_cos();
                let lx = (x - cx) * c + (y - cy) * s;
                let ly = -(x - cx) * s + (y - cy) * c;
                let ex = if rx.abs() < 1e-12 { f64::INFINITY } else { lx / rx };
                let ey = if ry.abs() < 1e-12 { f64::INFINITY } else { ly / ry };
                ex * ex + ey * ey <= 1.0
            }
            GateKind::Quadrant { x: sx, y: sy, quad } => match quad {
                Quadrant::UpperLeft => x <= *sx && y >= *sy,
                Quadrant::UpperRight => x >= *sx && y >= *sy,
                Quadrant::LowerLeft => x <= *sx && y <= *sy,
                Quadrant::LowerRight => x >= *sx && y <= *sy,
            },
            GateKind::Bilinear { x1, y1, x2, y2, .. } => {
                // Treat a click near the line as a grab on the gate body.
                let seg2 = ((x2 - x1).powi(2) + (y2 - y1).powi(2)).max(1e-12);
                let t = (((x - x1) * (x2 - x1) + (y - y1) * (y2 - y1)) / seg2)
                    .clamp(0.0, 1.0);
                let px = x1 + t * (x2 - x1);
                let py = y1 + t * (y2 - y1);
                let tol = (seg2.sqrt() * 0.04).max(1e-9);
                ((x - px).powi(2) + (y - py).powi(2)).sqrt() <= tol
            }
            GateKind::Polygon { vertices, .. } => point_in_polygon([x, y], vertices),
        }
    }
}

/// Centroid of a set of vertices (falls back to the first point).
fn centroid(vertices: &[[f64; 2]]) -> [f64; 2] {
    if vertices.is_empty() {
        return [0.0, 0.0];
    }
    let n = vertices.len() as f64;
    let sx: f64 = vertices.iter().map(|p| p[0]).sum();
    let sy: f64 = vertices.iter().map(|p| p[1]).sum();
    [sx / n, sy / n]
}

/// A regular `n`-gon inscribed in the ellipse `(rx, ry)` centred on `(cx, cy)`.
fn regular_polygon(cx: f64, cy: f64, rx: f64, ry: f64, n: usize) -> Vec<[f64; 2]> {
    let n = n.max(3);
    (0..n)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2
                + 2.0 * std::f64::consts::PI * i as f64 / n as f64;
            [cx + rx * a.cos(), cy + ry * a.sin()]
        })
        .collect()
}

fn rotate_point(p: [f64; 2], center: [f64; 2], delta: f64) -> [f64; 2] {
    let (s, c) = delta.sin_cos();
    let dx = p[0] - center[0];
    let dy = p[1] - center[1];
    [center[0] + dx * c - dy * s, center[1] + dx * s + dy * c]
}

/// Paints a single line of text centred on `center` and rotated by `angle`
/// radians clockwise. Used to preview the layout editor's axis-label styling.
fn paint_label(
    painter: &egui::Painter,
    center: egui::Pos2,
    text: &str,
    size: f32,
    color: egui::Color32,
    family: egui::FontFamily,
    angle: f32,
) {
    let font = egui::FontId::new(size.max(1.0), family);
    let galley = painter.layout_no_wrap(text.to_string(), font, color);
    let half = galley.size() / 2.0;
    // Offset of the galley centre from its top-left, rotated about the top-left
    // (which is the pivot egui rotates around).
    let (s, c) = angle.sin_cos();
    let off = egui::vec2(half.x * c - half.y * s, half.x * s + half.y * c);
    let shape = egui::epaint::TextShape::new(center - off, galley, color).with_angle(angle);
    painter.add(egui::Shape::Text(shape));
}

/// A single-line editor for a layout label's custom text. Blank means "use the
/// parameter name".
fn label_text_edit(ui: &mut egui::Ui, id: &str, style: &mut LabelStyle) {
    let mut text = style.text.clone().unwrap_or_default();
    if ui
        .add(
            egui::TextEdit::singleline(&mut text)
                .id_salt(id)
                .hint_text("(parameter)")
                .desired_width(110.0),
        )
        .changed()
    {
        style.text = if text.trim().is_empty() {
            None
        } else {
            Some(text)
        };
    }
}

/// Font-family picker for a layout label.
fn label_font_combo(ui: &mut egui::Ui, id: &str, font: &mut LabelFont) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(font.label())
        .width(72.0)
        .show_ui(ui, |ui| {
            for f in LabelFont::ALL {
                ui.selectable_value(font, f, f.label());
            }
        });
}

fn point_in_polygon(p: [f64; 2], verts: &[[f64; 2]]) -> bool {
    if verts.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = verts.len() - 1;
    for i in 0..verts.len() {
        let (xi, yi) = (verts[i][0], verts[i][1]);
        let (xj, yj) = (verts[j][0], verts[j][1]);
        if ((yi > p[1]) != (yj > p[1])) && (p[0] < (xj - xi) * (p[1] - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Applies a manipulation to a snapshot of a gate, producing the new geometry.
fn apply_manipulation(
    snapshot: &GateKind,
    target: &ManipTarget,
    start: [f64; 2],
    cur: [f64; 2],
) -> GateKind {
    let mut g = snapshot.clone();
    let center = snapshot.center();
    match target {
        ManipTarget::Move => g.translate(cur[0] - start[0], cur[1] - start[1]),
        ManipTarget::Handle(HandleId::Center) => {
            g.translate(cur[0] - start[0], cur[1] - start[1])
        }
        ManipTarget::Handle(HandleId::Rotate) => {
            let a0 = (start[1] - center[1]).atan2(start[0] - center[0]);
            let a1 = (cur[1] - center[1]).atan2(cur[0] - center[0]);
            g.rotate_about(center, a1 - a0);
        }
        ManipTarget::Handle(HandleId::Corner(_)) => {
            let d0 = ((start[0] - center[0]).powi(2) + (start[1] - center[1]).powi(2)).sqrt();
            let d1 = ((cur[0] - center[0]).powi(2) + (cur[1] - center[1]).powi(2)).sqrt();
            if d0 > 1e-9 {
                g.scale_about(center, d1 / d0);
            }
        }
        ManipTarget::Handle(HandleId::Edge(k)) => {
            if let GateKind::Ellipse {
                x_min,
                x_max,
                y_min,
                y_max,
                angle,
                ..
            } = &mut g
            {
                let cx = (*x_min + *x_max) / 2.0;
                let cy = (*y_min + *y_max) / 2.0;
                let (s, c) = angle.sin_cos();
                // Transform start and cur into the ellipse's local (unrotated) frame.
                let lx0 = (start[0] - cx) * c + (start[1] - cy) * s;
                let ly0 = -(start[0] - cx) * s + (start[1] - cy) * c;
                let lx1 = (cur[0] - cx) * c + (cur[1] - cy) * s;
                let ly1 = -(cur[0] - cx) * s + (cur[1] - cy) * c;
                match k {
                    0 | 2 => {
                        // Horizontal edges: scale x-radius about centre.
                        let old_rx = (*x_max - *x_min) / 2.0;
                        let ratio = if lx0.abs() > 1e-9 {
                            lx1.abs() / lx0.abs()
                        } else {
                            1.0
                        };
                        let new_rx = old_rx * ratio.clamp(0.01, 100.0);
                        *x_min = cx - new_rx;
                        *x_max = cx + new_rx;
                    }
                    1 | 3 => {
                        // Vertical edges: scale y-radius about centre.
                        let old_ry = (*y_max - *y_min) / 2.0;
                        let ratio = if ly0.abs() > 1e-9 {
                            ly1.abs() / ly0.abs()
                        } else {
                            1.0
                        };
                        let new_ry = old_ry * ratio.clamp(0.01, 100.0);
                        *y_min = cy - new_ry;
                        *y_max = cy + new_ry;
                    }
                    _ => {}
                }
            }
        }
        ManipTarget::Handle(HandleId::Vertex(i)) => match &mut g {
            GateKind::Polygon { vertices, .. } => {
                if *i < vertices.len() {
                    vertices[*i] = cur;
                }
            }
            GateKind::Bilinear { x1, y1, x2, y2, .. } => match i {
                0 => {
                    *x1 = cur[0];
                    *y1 = cur[1];
                }
                1 => {
                    *x2 = cur[0];
                    *y2 = cur[1];
                }
                _ => {}
            },
            _ => {}
        },
    }
    g
}

/// Per-plot interactive editing state.
#[derive(Default, Clone)]
struct Interact {
    /// Creation drag rectangle (data coords), while a gate is being drawn.
    drag_start: Option<[f64; 2]>,
    drag_current: Option<[f64; 2]>,
    /// Active gate manipulation, while a handle or the gate body is dragged.
    manip: Option<ManipSession>,
}

/// A gate manipulation in progress. The gate geometry is snapshotted when the
/// drag begins so updates are always computed from the original shape.
#[derive(Clone)]
struct ManipSession {
    gate: usize,
    snapshot: GateKind,
    target: ManipTarget,
    start: [f64; 2],
}

/// A request produced by [`drive_interact`] for the caller to apply.
enum InteractOutcome {
    Create { start: [f64; 2], end: [f64; 2] },
    Update { gate: usize, kind: GateKind },
    Click([f64; 2]),
}

/// Nearest handle within `tol` (data units) of `pos`.
fn nearest_handle(kind: &GateKind, pos: [f64; 2], tol: f64) -> Option<HandleId> {
    let mut best: Option<(f64, HandleId)> = None;
    for (hp, id) in kind.handles() {
        let d = ((hp[0] - pos[0]).powi(2) + (hp[1] - pos[1]).powi(2)).sqrt();
        if d <= tol && best.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true) {
            best = Some((d, id));
        }
    }
    best.map(|(_, id)| id)
}

/// Advances per-plot interaction given the raw [`PlotInteraction`] events.
///
/// Decides between creating a new gate (drag on empty space) and manipulating
/// the selected gate (drag on one of its handles, or on its body to move it),
/// then returns the resulting outcome for the caller to apply.
fn drive_interact(
    it: &mut Interact,
    inter: &PlotInteraction,
    gates: &[GateDef],
    selected: Option<usize>,
    new_type: NewGateType,
    tol: f64,
) -> Option<InteractOutcome> {
    // Keep the in-progress creation rectangle up to date.
    if it.drag_start.is_some() {
        if let Some(p) = inter.pointer {
            it.drag_current = Some(p);
        }
    }

    let mut outcome = None;

    // A drag just started: choose manipulation vs. creation.
    if let Some(start) = inter.started {
        let mut creating = true;
        if let Some(gi) = selected {
            if let Some(gate) = gates.get(gi) {
                if let Some(h) = nearest_handle(&gate.kind, start, tol) {
                    it.manip = Some(ManipSession {
                        gate: gi,
                        snapshot: gate.kind.clone(),
                        target: ManipTarget::Handle(h),
                        start,
                    });
                    creating = false;
                } else if gate.kind.contains_point(start) {
                    it.manip = Some(ManipSession {
                        gate: gi,
                        snapshot: gate.kind.clone(),
                        target: ManipTarget::Move,
                        start,
                    });
                    creating = false;
                }
            }
        }
        if creating {
            it.drag_start = Some(start);
            it.drag_current = Some(start);
        }
    }

    if let Some(end) = inter.released {
        if it.manip.take().is_some() {
            // Manipulation finished; nothing further to do.
        } else if let Some(a) = it.drag_start.take() {
            let b = inter.pointer.or(it.drag_current).unwrap_or(end);
            outcome = Some(InteractOutcome::Create { start: a, end: b });
        }
        it.drag_start = None;
        it.drag_current = None;
    } else if let Some(m) = &it.manip {
        if let Some(cur) = inter.pointer {
            outcome = Some(InteractOutcome::Update {
                gate: m.gate,
                kind: apply_manipulation(&m.snapshot, &m.target, m.start, cur),
            });
        }
    }

    if let Some(p) = inter.clicked {
        if matches!(new_type, NewGateType::Quadrant | NewGateType::Bilinear) {
            outcome = Some(InteractOutcome::Click(p));
        }
    }

    outcome
}

/// A gate definition, shared across all loaded samples.
#[derive(Debug, Clone, PartialEq)]
struct GateDef {
    name: String,
    /// Parameter indices this gate was defined on.
    x_idx: usize,
    y_idx: usize,
    /// Parent gate (builds the hierarchy). Always `< this gate's index`.
    parent: Option<usize>,
    kind: GateKind,
    color: egui::Color32,
    /// The sample this gate was created on (None = applies to all).
    owner: Option<usize>,
    /// Whether this gate applies to every loaded sample, or only its owner.
    apply_to_all: bool,
}

impl GateDef {
    fn to_gate(&self) -> Box<dyn GatingGate> {
        self.kind.to_gate(self.x_idx, self.y_idx)
    }
}

/// Computes the event indices for every gate, following the parent hierarchy.
///
/// `sample_idx` is the index of the sample being computed for. `sample_apply_gates`
/// is the per-sample switch set when the file was loaded (or toggled by the user);
/// when it is false the shared gate set is skipped for that sample. A gate is
/// evaluated for a sample when either the gate is marked "apply to all" *and* the
/// sample has gating enabled, or the sample is the gate's owner. Gates with an
/// out-of-range parameter index simply yield an empty population.
fn compute_gate_populations(
    data: &ndarray::Array2<f32>,
    gates: &[GateDef],
    sample_idx: Option<usize>,
    sample_apply_gates: bool,
) -> Vec<Vec<usize>> {
    let (total, n_params) = data.dim();
    let mut result: Vec<Vec<usize>> = vec![Vec::new(); gates.len()];
    for (i, gate) in gates.iter().enumerate() {
        // A gate always applies to the sample it was created on. Otherwise it
        // applies only when it is marked "apply to all" and the sample has
        // gating enabled.
        let owner = gate.owner == sample_idx;
        let shared = gate.apply_to_all && sample_apply_gates;
        if !owner && !shared {
            continue;
        }
        if gate.x_idx >= n_params || gate.y_idx >= n_params {
            continue;
        }
        let gate_obj = gate.to_gate();
        let base: Vec<usize> = match gate.parent {
            Some(p) if p < i => result[p].clone(),
            _ => (0..total).collect(),
        };
        result[i] = base
            .into_iter()
            .filter(|&e| gate_obj.contains(data.row(e)))
            .collect();
    }
    result
}

/// Children lists from the parent pointers (for the hierarchy view).
fn children_map(gates: &[GateDef]) -> Vec<Vec<usize>> {
    let mut children = vec![Vec::new(); gates.len()];
    for (i, gate) in gates.iter().enumerate() {
        if let Some(p) = gate.parent {
            if p < gates.len() {
                children[p].push(i);
            }
        }
    }
    children
}

/// The effective horizontal extent of a line gate. A (near-)vertical segment has
/// no meaningful width, so it falls back to the full axis span; this keeps the
/// gate non-empty and matches the renderer's fill.
fn effective_x_band(x1: f64, x2: f64, x_range0: f64, x_range1: f64) -> (f64, f64) {
    let (a, b) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
    if (b - a).abs() < 1e-9 {
        (x_range0.min(x_range1), x_range0.max(x_range1))
    } else {
        (a, b)
    }
}

/// Y value of the line through `(x1, y1)..(x2, y2)` at `x`.
fn line_y_at(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let dx = x2 - x1;
    if dx.abs() < 1e-9 {
        (y1 + y2) / 2.0
    } else {
        y1 + (y2 - y1) * (x - x1) / dx
    }
}

fn gate_color(i: usize) -> egui::Color32 {
    const PALETTE: [(u8, u8, u8); 8] = [
        (255, 140, 0),  // orange
        (0, 170, 220),  // cyan
        (40, 190, 80),  // green
        (215, 60, 200), // magenta
        (230, 200, 30), // yellow
        (120, 120, 255),// periwinkle
        (255, 100, 140),// pink
        (200, 60, 40),  // red
    ];
    let (r, g, b) = PALETTE[i % PALETTE.len()];
    egui::Color32::from_rgb(r, g, b)
}

fn color_to_hex(c: egui::Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

// ---------------------------------------------------------------------------
// Plot rendering
// ---------------------------------------------------------------------------

/// A gate drawn on top of a plot, in data coordinates.
struct GateVisual {
    fill: Vec<[f64; 2]>,
    lines: Vec<Vec<[f64; 2]>>,
    color: egui::Color32,
    selected: bool,
}

/// Everything needed to draw one plot.
struct PlotCfg {
    points: Vec<[f64; 2]>,
    x_range: (f64, f64),
    y_range: (f64, f64),
    x_scale: AxisScale,
    y_scale: AxisScale,
    point_color: egui::Color32,
    gradient: bool,
    view: ViewMode,
    contour_levels: usize,
    gates: Vec<GateVisual>,
    /// Draggable manipulation handles for the selected gate (data coords).
    handles: Vec<([f64; 2], HandleId)>,
    size: (f32, f32),
    x_label: String,
    y_label: String,
    interactive: bool,
    /// When true, the creation drag shows a line (for line/bilinear gates)
    /// instead of a rectangle.
    line_preview: bool,
}

/// Interaction state passed into / out of [`show_plot`].
#[derive(Default, Clone)]
struct PlotInteraction {
    drag_start: Option<[f64; 2]>,
    drag_current: Option<[f64; 2]>,
    /// Set on the frame a drag begins (the press position, in data coords).
    started: Option<[f64; 2]>,
    /// Set on the frame a drag ends.
    released: Option<[f64; 2]>,
    /// Current pointer position (data coords) whenever the pointer is over the plot.
    pointer: Option<[f64; 2]>,
    clicked: Option<[f64; 2]>,
}

fn normalize_span(a: f64, b: f64) -> (f64, f64) {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    if (hi - lo).abs() < f64::EPSILON {
        (lo - 1.0, hi + 1.0)
    } else {
        (lo, hi)
    }
}

/// Draws a single plot (main view, detached population window, or preview).
fn show_plot(ui: &mut egui::Ui, id: &str, cfg: &PlotCfg, inter: &mut PlotInteraction) {
    // Transform the axis ranges into plot space.
    let xr = cfg.x_range;
    let yr = cfg.y_range;
    let tx = |v: f64| cfg.x_scale.transform(v).unwrap_or(0.0);
    let ty = |v: f64| cfg.y_scale.transform(v).unwrap_or(0.0);
    let (bx0, bx1) = normalize_span(tx(xr.0), tx(xr.1));
    let (by0, by1) = if matches!(cfg.view, ViewMode::Histogram) {
        (0.0, 1.0) // placeholder, overridden below after histogram computation
    } else {
        normalize_span(ty(yr.0), ty(yr.1))
    };

    // Transform all points up-front.
    let tpoints: Vec<[f64; 2]> = cfg
        .points
        .iter()
        .filter_map(|p| Some([cfg.x_scale.transform(p[0])?, cfg.y_scale.transform(p[1])?]))
        .collect();

    let need_density = cfg.gradient || matches!(cfg.view, ViewMode::Contour);
    let grid = need_density.then(|| density_grid(&tpoints, bx0, bx1, by0, by1, 128, 128));
    let densities = grid.as_ref().map(|g| (g.max(), point_densities(&tpoints, g)));

    // Bucket points by density (gradient) or keep a single colour.
    let mut buckets: Vec<(egui::Color32, Vec<[f64; 2]>)> = Vec::new();
    if matches!(cfg.view, ViewMode::Dots) {
        if cfg.gradient {
            const NBINS: usize = 16;
            let mut bins: Vec<Vec<[f64; 2]>> = vec![Vec::new(); NBINS];
            if let (Some(_g), Some((max_d, dens))) = (&grid, &densities) {
                for (i, p) in tpoints.iter().enumerate() {
                    let t = if *max_d > 0.0 {
                        (dens[i] / max_d).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let b = ((t * (NBINS - 1) as f64).round() as usize).min(NBINS - 1);
                    bins[b].push(*p);
                }
            }
            for (b, pts) in bins.into_iter().enumerate() {
                if pts.is_empty() {
                    continue;
                }
                let (r, g, bl) = density_rgb(b as f64 / (NBINS - 1) as f64);
                buckets.push((egui::Color32::from_rgb(r, g, bl), pts));
            }
        } else {
            buckets.push((cfg.point_color, tpoints.clone()));
        }
    }

    // Contour segments.
    let mut contour_paths: Vec<[ [f64; 2]; 2 ]> = Vec::new();
    if matches!(cfg.view, ViewMode::Contour) {
        if let Some(g) = grid.as_ref() {
            let max_d = g.max();
            for level in crate::plot::contour_levels(max_d, cfg.contour_levels) {
                contour_paths.extend(crate::plot::contour_segments(g, level));
            }
        }
    }
    let contour_color = egui::Color32::from_rgb(30, 110, 130);

    // Histogram bars (for Histogram view mode).
    let (hist_bars, hist_max_count, hist_bin_w) =
        if matches!(cfg.view, ViewMode::Histogram) {
            let xs: Vec<f64> = cfg.points.iter().map(|p| p[0]).collect();
            let (x0, x1) = cfg.x_range;
            let bin_count = 50usize;
            let bin_w = (x1 - x0) / bin_count as f64;
            let mut counts = vec![0usize; bin_count];
            for &x in &xs {
                if !x.is_finite() || x < x0 || x > x1 {
                    continue;
                }
                let bi = ((x - x0) / bin_w) as usize;
                let bi = bi.min(bin_count - 1);
                counts[bi] += 1;
            }
            let max_count = counts.iter().max().copied().unwrap_or(1).max(1);
            let bars: Vec<Bar> = counts
                .iter()
                .enumerate()
                .map(|(i, &c)| {
                    let center = x0 + (i as f64 + 0.5) * bin_w;
                    Bar::new(center, c as f64)
                })
                .collect();
            (Some(bars), max_count as f64, bin_w)
        } else {
            (None, 0.0, 0.0)
        };

    // Override y-range for histogram: counts from 0 to max count.
    let (by0, by1) = if matches!(cfg.view, ViewMode::Histogram) {
        (0.0, hist_max_count)
    } else {
        (by0, by1)
    };

    Plot::new(id)
        .width(cfg.size.0)
        .height(cfg.size.1)
        .view_aspect(1.0)
        .allow_boxed_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .show_axes(true)
        .x_axis_label(&cfg.x_label)
        .y_axis_label(&cfg.y_label)
        .x_axis_formatter(|mark, _range| crate::plot::format_tick(mark.value, cfg.x_scale))
        .y_axis_formatter(|mark, _range| crate::plot::format_tick(mark.value, cfg.y_scale))
        .show(ui, |plot_ui| {
            plot_ui.set_plot_bounds(PlotBounds::from_min_max([bx0, by0], [bx1, by1]));

            // Drag preview (created by the user while gating).
            if let (Some(a), Some(b)) = (inter.drag_start, inter.drag_current) {
                let a = [tx(a[0]), ty(a[1])];
                let b = [tx(b[0]), ty(b[1])];
                if cfg.line_preview {
                    plot_ui.line(
                        Line::new(vec![a, b])
                            .color(egui::Color32::from_rgb(0, 120, 200))
                            .width(2.0_f32),
                    );
                } else {
                    plot_ui.polygon(
                        Polygon::new(vec![
                            [a[0], a[1]],
                            [b[0], a[1]],
                            [b[0], b[1]],
                            [a[0], b[1]],
                            [a[0], a[1]],
                        ])
                        .fill_color(egui::Color32::from_rgba_unmultiplied(0, 150, 255, 60))
                        .stroke(egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 120, 200))),
                    );
                }
            }

            // Gates.
            for g in &cfg.gates {
                if g.fill.len() >= 3 {
                    let poly: Vec<[f64; 2]> = g
                        .fill
                        .iter()
                        .filter_map(|p| Some([cfg.x_scale.transform(p[0])?, cfg.y_scale.transform(p[1])?]))
                        .collect();
                    if poly.len() >= 3 {
                        let fill = if g.selected {
                            egui::Color32::from_rgba_unmultiplied(g.color.r(), g.color.g(), g.color.b(), 70)
                        } else {
                            egui::Color32::from_rgba_unmultiplied(g.color.r(), g.color.g(), g.color.b(), 30)
                        };
                        let stroke = if g.selected {
                            egui::Stroke::new(2.5_f32, g.color)
                        } else {
                            egui::Stroke::new(1.5_f32, g.color)
                        };
                        plot_ui.polygon(Polygon::new(poly).fill_color(fill).stroke(stroke));
                    }
                }
                for line in &g.lines {
                    let pts: Vec<[f64; 2]> = line
                        .iter()
                        .filter_map(|p| Some([cfg.x_scale.transform(p[0])?, cfg.y_scale.transform(p[1])?]))
                        .collect();
                    if pts.len() >= 2 {
                        let stroke = if g.selected {
                            egui::Stroke::new(2.5_f32, g.color)
                        } else {
                            egui::Stroke::new(1.5_f32, g.color)
                        };
                        plot_ui.line(Line::new(pts).stroke(stroke));
                    }
                }
            }

            // Manipulation handles for the selected gate.
            for (hp, id) in &cfg.handles {
                if let (Some(px), Some(py)) = (
                    cfg.x_scale.transform(hp[0]),
                    cfg.y_scale.transform(hp[1]),
                ) {
                    let (color, shape) = match id {
                        HandleId::Rotate => (
                            egui::Color32::from_rgb(240, 210, 60),
                            egui_plot::MarkerShape::Diamond,
                        ),
                        HandleId::Center => (
                            egui::Color32::from_rgb(255, 255, 255),
                            egui_plot::MarkerShape::Cross,
                        ),
                        _ => (
                            egui::Color32::from_rgb(255, 255, 255),
                            egui_plot::MarkerShape::Square,
                        ),
                    };
                    plot_ui.points(
                        Points::new(vec![[px, py]])
                            .color(color)
                            .radius(4.0_f32)
                            .shape(shape)
                            .allow_hover(false),
                    );
                }
            }

            // Events (dots) or histogram bars.
            if matches!(cfg.view, ViewMode::Histogram) {
                if let Some(bars) = &hist_bars {
                    plot_ui.bar_chart(
                        BarChart::new(bars.clone())
                            .color(cfg.point_color)
                            .width(hist_bin_w * 0.9),
                    );
                }
            } else {
                for (color, pts) in &buckets {
                    plot_ui.points(
                        Points::new(pts.clone())
                            .color(*color)
                            .radius(2.0_f32)
                            .allow_hover(false),
                    );
                }
            }

            // Contour lines.
            for seg in &contour_paths {
                plot_ui.line(Line::new(vec![seg[0], seg[1]]).color(contour_color).width(1.2_f32));
            }

            // Interaction: report pointer position and drag/click events.
            if cfg.interactive {
                let resp = plot_ui.response();
                if let Some(pos) = plot_ui.pointer_coordinate() {
                    inter.pointer = Some([
                        cfg.x_scale.inverse(pos.x),
                        cfg.y_scale.inverse(pos.y),
                    ]);
                }

                if resp.drag_started() {
                    inter.started = inter.pointer;
                }
                if resp.drag_stopped() {
                    inter.released = inter.pointer.or(inter.started);
                }
                if resp.clicked() {
                    inter.clicked = inter.pointer;
                }
            }
        });

    // Set the mouse cursor based on which handle (if any) is hovered.
    if cfg.interactive {
        if let Some(ptr) = inter.pointer {
            let span = (cfg.x_range.1 - cfg.x_range.0)
                .abs()
                .max((cfg.y_range.1 - cfg.y_range.0).abs())
                .max(1.0);
            let tol = 0.03 * span;
            if let Some((_, handle_id)) = cfg.handles.iter().find(|(hp, _)| {
                ((hp[0] - ptr[0]).powi(2) + (hp[1] - ptr[1]).powi(2)).sqrt() <= tol
            }) {
                let cursor = match handle_id {
                    HandleId::Corner(0) | HandleId::Corner(2) => {
                        egui::CursorIcon::ResizeNwSe
                    }
                    HandleId::Corner(1) | HandleId::Corner(3) => {
                        egui::CursorIcon::ResizeNeSw
                    }
                    HandleId::Edge(0) | HandleId::Edge(2) => {
                        egui::CursorIcon::ResizeHorizontal
                    }
                    HandleId::Edge(1) | HandleId::Edge(3) => {
                        egui::CursorIcon::ResizeVertical
                    }
                    HandleId::Rotate => egui::CursorIcon::Alias,
                    HandleId::Vertex(_) | HandleId::Center => egui::CursorIcon::Grab,
                    _ => egui::CursorIcon::Default,
                };
                ui.ctx().set_cursor_icon(cursor);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

/// A loaded FCS sample.
struct SampleEntry {
    id: String,
    /// Full path to the source FCS file (used when saving a workspace).
    path: std::path::PathBuf,
    fcs: FcsFile,
    /// Whether the shared gate set is applied to this sample. Controlled from
    /// the sample list and by the per-file prompt shown when a file is loaded
    /// after gates already exist.
    apply_gates: bool,
}

/// Which gate type new gates get when drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum NewGateType {
    #[default]
    Rect,
    Ellipse,
    Polygon,
    Quadrant,
    Bilinear,
}

impl NewGateType {
    fn label(self) -> &'static str {
        match self {
            NewGateType::Rect => "Rectangle",
            NewGateType::Ellipse => "Ellipse",
            NewGateType::Polygon => "Polygon",
            NewGateType::Quadrant => "Quadrant",
            NewGateType::Bilinear => "Bilinear",
        }
    }
    const ALL: [NewGateType; 5] = [
        NewGateType::Rect,
        NewGateType::Ellipse,
        NewGateType::Polygon,
        NewGateType::Quadrant,
        NewGateType::Bilinear,
    ];
}

/// Font family choices for layout axis labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LabelFont {
    Sans,
    Serif,
    Mono,
}

impl LabelFont {
    const ALL: [LabelFont; 3] = [LabelFont::Sans, LabelFont::Serif, LabelFont::Mono];

    fn label(self) -> &'static str {
        match self {
            LabelFont::Sans => "Sans",
            LabelFont::Serif => "Serif",
            LabelFont::Mono => "Mono",
        }
    }

    /// CSS font-family used in SVG export.
    fn css(self) -> &'static str {
        match self {
            LabelFont::Sans => "sans-serif",
            LabelFont::Serif => "serif",
            LabelFont::Mono => "monospace",
        }
    }

    /// egui font family used for the live preview.
    fn egui(self) -> egui::FontFamily {
        match self {
            LabelFont::Mono => egui::FontFamily::Monospace,
            _ => egui::FontFamily::Proportional,
        }
    }
}

/// Styling for one axis label in the layout editor.
#[derive(Debug, Clone, PartialEq)]
struct LabelStyle {
    /// Custom text; `None` uses the parameter name.
    text: Option<String>,
    /// Font size (SVG units).
    size: f32,
    /// Rotation, in degrees (clockwise).
    rotation_deg: f32,
    font: LabelFont,
    color: egui::Color32,
}

impl LabelStyle {
    fn x_default() -> Self {
        Self {
            text: None,
            size: 11.0,
            rotation_deg: 0.0,
            font: LabelFont::Sans,
            color: egui::Color32::from_gray(90),
        }
    }

    fn y_default() -> Self {
        Self {
            text: None,
            size: 11.0,
            rotation_deg: -90.0,
            font: LabelFont::Sans,
            color: egui::Color32::from_gray(90),
        }
    }
}

/// One entry in the layout editor. Panels live on a free-form canvas and can be
/// moved / resized with the mouse (Photoshop-style).
#[derive(Clone, PartialEq)]
struct LayoutPanelSpec {
    sample: usize,
    x: usize,
    y: usize,
    view: ViewMode,
    /// Optional gate index: if set, the panel shows only that gated population.
    gate: Option<usize>,
    title: String,
    /// Position & size on the layout canvas, in canvas pixels.
    rect: egui::Rect,
    /// Normalised (0..1) preview points, cached when the panel is added.
    preview: Vec<[f32; 2]>,
    /// Styling for the X and Y axis labels.
    x_label: LabelStyle,
    y_label: LabelStyle,
}

/// An in-progress canvas manipulation (move or resize) of a layout panel.
#[derive(Clone, Copy)]
struct LayoutDrag {
    panel: usize,
    resize: bool,
    grab: egui::Vec2,
}

/// Detached-window state: an independently configurable view of one population.
struct PopoutState {
    open: bool,
    gate: Option<usize>,
    x_param: Option<usize>,
    y_param: Option<usize>,
    view: ViewMode,
    x_scale: AxisScale,
    y_scale: AxisScale,
    point_color: egui::Color32,
    gradient: bool,
    contour_levels: usize,
    x_range: (f64, f64),
    y_range: (f64, f64),
    new_gate_type: NewGateType,
    selected_gate: Option<usize>,
    interact: Interact,
}

impl Default for PopoutState {
    fn default() -> Self {
        Self {
            open: false,
            gate: None,
            x_param: None,
            y_param: None,
            view: ViewMode::Dots,
            x_scale: AxisScale::Linear,
            y_scale: AxisScale::Linear,
            point_color: egui::Color32::from_rgb(31, 119, 180),
            gradient: true,
            contour_levels: 8,
            x_range: (0.0, 1.0),
            y_range: (0.0, 1.0),
            new_gate_type: NewGateType::Rect,
            selected_gate: None,
            interact: Interact::default(),
        }
    }
}

/// Colour theme for the whole application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
        }
    }
}

/// A snapshot of the editable application state for Undo / Redo.
///
/// Loaded samples (the FCS data) are deliberately excluded: they are inputs, not
/// edits, so undoing never unloads a file. Everything a user can change about the
/// gating, the view and the layout is captured here.
#[derive(Clone, PartialEq)]
struct UndoState {
    gates: Vec<GateDef>,
    selected_gate: Option<usize>,
    x_param: Option<usize>,
    y_param: Option<usize>,
    view_mode: ViewMode,
    x_scale: AxisScale,
    y_scale: AxisScale,
    point_color: egui::Color32,
    gradient: bool,
    contour_levels: usize,
    x_range: (f64, f64),
    y_range: (f64, f64),
    layout_panels: Vec<LayoutPanelSpec>,
    layout_columns: usize,
    layout_selected: Option<usize>,
}

impl UndoState {
    /// Compares everything that counts as a *document* edit, ignoring the
    /// transient selection fields so that merely selecting a gate or panel does
    /// not create an undo step.
    fn same_document(&self, other: &Self) -> bool {
        self.gates == other.gates
            && self.x_param == other.x_param
            && self.y_param == other.y_param
            && self.view_mode == other.view_mode
            && self.x_scale == other.x_scale
            && self.y_scale == other.y_scale
            && self.point_color == other.point_color
            && self.gradient == other.gradient
            && self.contour_levels == other.contour_levels
            && self.x_range == other.x_range
            && self.y_range == other.y_range
            && self.layout_panels == other.layout_panels
            && self.layout_columns == other.layout_columns
    }
}

/// The egui application state.
pub struct FlowersApp {
    samples: Vec<SampleEntry>,
    current_sample: Option<usize>,
    x_param: Option<usize>,
    y_param: Option<usize>,

    gates: Vec<GateDef>,
    selected_gate: Option<usize>,
    /// Cached per-gate event indices for the current sample.
    pop_indices: Vec<Vec<usize>>,
    /// Cached children lists.
    children: Vec<Vec<usize>>,

    // Plot configuration.
    view_mode: ViewMode,
    x_scale: AxisScale,
    y_scale: AxisScale,
    point_color: egui::Color32,
    gradient: bool,
    contour_levels: usize,
    x_range: (f64, f64),
    y_range: (f64, f64),
    x_data_range: (f64, f64),
    y_data_range: (f64, f64),

    // Cached scatter points for the current sample + parameters.
    plot_points: Vec<[f64; 2]>,

    // Gate creation & manipulation.
    new_gate_type: NewGateType,
    interact: Interact,

    // Detached population window.
    popout: PopoutState,

    // Layout editor.
    layout_panels: Vec<LayoutPanelSpec>,
    layout_columns: usize,
    layout_open: bool,
    layout_selected: Option<usize>,
    layout_drag: Option<LayoutDrag>,

    // Appearance / export.
    theme: Theme,
    export_scale: f32,

    name_edit: String,
    status: String,
    /// Index of a newly loaded sample waiting for the "apply existing gates?"
    /// prompt. `None` when no prompt is pending.
    pending_apply_prompt: Option<usize>,

    // Undo / redo.
    /// Snapshots of [`UndoState`], oldest first. `history[history_idx]` is the
    /// currently-applied state.
    history: Vec<UndoState>,
    history_idx: usize,
    /// Set for one frame after an undo/redo so the state comparison at the end
    /// of the frame does not push a redundant entry.
    history_skip: bool,
}

impl Default for FlowersApp {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            current_sample: None,
            x_param: None,
            y_param: None,
            gates: Vec::new(),
            selected_gate: None,
            pop_indices: Vec::new(),
            children: Vec::new(),
            view_mode: ViewMode::Dots,
            x_scale: AxisScale::Linear,
            y_scale: AxisScale::Linear,
            point_color: egui::Color32::from_rgb(31, 119, 180),
            gradient: true,
            contour_levels: 8,
            x_range: (0.0, 1.0),
            y_range: (0.0, 1.0),
            x_data_range: (0.0, 1.0),
            y_data_range: (0.0, 1.0),
            plot_points: Vec::new(),
            new_gate_type: NewGateType::Rect,
            interact: Interact::default(),
            popout: PopoutState::default(),
            layout_panels: Vec::new(),
            layout_columns: 2,
            layout_open: false,
            layout_selected: None,
            layout_drag: None,
            theme: Theme::Dark,
            export_scale: 2.0,
            name_edit: String::new(),
            status: "Ready — load an FCS file".to_string(),
            pending_apply_prompt: None,
            history: Vec::new(),
            history_idx: 0,
            history_skip: false,
        }
    }
}

/// Human-readable display name for a parameter.
fn param_display_name(fcs: &FcsFile, idx: usize) -> String {
    fcs.parameters
        .get(idx)
        .map(|p| {
            let desc = p.description.trim();
            let name = p.name.trim();
            if !desc.is_empty() && desc != name {
                desc.to_string()
            } else {
                name.to_string()
            }
        })
        .unwrap_or_else(|| format!("P{idx}"))
}

/// Chooses sensible default X/Y parameters for a newly loaded file.
///
/// Prefers **FSC-H** and **SSC-H** (falling back to FSC-A/SSC-A, then any FSC/
/// SSC channel), and otherwise uses the first two parameters. The two axes are
/// guaranteed to differ when the file has at least two parameters.
fn default_param_pair(fcs: &FcsFile) -> (usize, usize) {
    let n = fcs.parameters.len();
    if n == 0 {
        return (0, 0);
    }
    let find = |want: &str| {
        fcs.parameters.iter().position(|p| {
            p.name.trim().eq_ignore_ascii_case(want)
                || p.description.trim().eq_ignore_ascii_case(want)
        })
    };
    let x = find("FSC-H")
        .or_else(|| find("FSC-A"))
        .or_else(|| find("FSC"))
        .unwrap_or(0);
    let y = find("SSC-H")
        .or_else(|| find("SSC-A"))
        .or_else(|| find("SSC"))
        .unwrap_or(if n > 1 { 1 } else { 0 });
    let y = if y == x && n > 1 { (x + 1) % n } else { y };
    (x, y)
}

/// Compact, precise formatting for editable axis-range fields.
///
/// Flow-cytometry axes are often on the order of 10^7, where a plain decimal is
/// hard to read and type. Large/small magnitudes are shown in scientific
/// notation (e.g. `5.300871e6`) and mid-range values in plain notation.
///
/// Both branches use Rust's shortest round-tripping float formatting, so the
/// displayed text parses back to exactly the same value — focusing a field and
/// clicking away never nudges the number.
fn format_edit_num(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let abs = v.abs();
    if abs >= 1.0e5 || abs < 1.0e-2 {
        format!("{v:e}")
    } else {
        format!("{v}")
    }
}

/// A `DragValue` for editing one end of an axis range.
///
/// The drag speed scales with the axis `span`, so a 10^7-wide axis can be swept
/// in a single drag instead of one unit per pixel, and the value is displayed in
/// compact scientific notation. Typing works too (`1e7`, `5.3e6`, `250000`).
fn range_drag_value(value: &mut f64, span: f64) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .speed((span.abs() / 500.0).max(1.0))
        .custom_formatter(|n, _| format_edit_num(n))
        .custom_parser(|s| s.trim().parse::<f64>().ok())
}

// -- workspace (de)serialisation helpers -----------------------------------

fn view_mode_id(v: ViewMode) -> &'static str {
    match v {
        ViewMode::Dots => "dots",
        ViewMode::Contour => "contour",
        ViewMode::Histogram => "histogram",
    }
}

fn view_mode_from_id(s: &str) -> ViewMode {
    match s {
        "contour" => ViewMode::Contour,
        "histogram" => ViewMode::Histogram,
        _ => ViewMode::Dots,
    }
}

fn axis_scale_id(s: AxisScale) -> &'static str {
    match s {
        AxisScale::Linear => "linear",
        AxisScale::Log => "log",
    }
}

fn axis_scale_from_id(s: &str) -> AxisScale {
    match s {
        "log" => AxisScale::Log,
        _ => AxisScale::Linear,
    }
}

fn label_font_id(f: LabelFont) -> &'static str {
    match f {
        LabelFont::Sans => "sans",
        LabelFont::Serif => "serif",
        LabelFont::Mono => "mono",
    }
}

fn label_font_from_id(s: &str) -> LabelFont {
    match s {
        "serif" => LabelFont::Serif,
        "mono" => LabelFont::Mono,
        _ => LabelFont::Sans,
    }
}

fn quadrant_id(q: &Quadrant) -> &'static str {
    match q {
        Quadrant::UpperLeft => "upper_left",
        Quadrant::UpperRight => "upper_right",
        Quadrant::LowerLeft => "lower_left",
        Quadrant::LowerRight => "lower_right",
    }
}

fn quadrant_from_id(s: &str) -> Quadrant {
    match s {
        "upper_left" => Quadrant::UpperLeft,
        "lower_left" => Quadrant::LowerLeft,
        "lower_right" => Quadrant::LowerRight,
        _ => Quadrant::UpperRight,
    }
}

fn gate_shape_from_kind(kind: &GateKind) -> GateShape {
    match kind {
        GateKind::Rect {
            x_min,
            x_max,
            y_min,
            y_max,
            angle,
        } => GateShape::Rect {
            x_min: *x_min,
            x_max: *x_max,
            y_min: *y_min,
            y_max: *y_max,
            angle: *angle,
        },
        GateKind::Ellipse {
            x_min,
            x_max,
            y_min,
            y_max,
            angle,
        } => GateShape::Ellipse {
            x_min: *x_min,
            x_max: *x_max,
            y_min: *y_min,
            y_max: *y_max,
            angle: *angle,
        },
        GateKind::Polygon { vertices } => GateShape::Polygon {
            vertices: vertices.clone(),
        },
        GateKind::Quadrant { x, y, quad } => GateShape::Quadrant {
            x: *x,
            y: *y,
            quad: quadrant_id(quad).to_string(),
        },
        GateKind::Bilinear {
            x1,
            y1,
            x2,
            y2,
            below,
        } => GateShape::Bilinear {
            x1: *x1,
            y1: *y1,
            x2: *x2,
            y2: *y2,
            below: *below,
        },
    }
}

fn gate_kind_from_shape(shape: &GateShape) -> GateKind {
    match shape {
        GateShape::Rect {
            x_min,
            x_max,
            y_min,
            y_max,
            angle,
        } => GateKind::Rect {
            x_min: *x_min,
            x_max: *x_max,
            y_min: *y_min,
            y_max: *y_max,
            angle: *angle,
        },
        GateShape::Ellipse {
            x_min,
            x_max,
            y_min,
            y_max,
            angle,
        } => GateKind::Ellipse {
            x_min: *x_min,
            x_max: *x_max,
            y_min: *y_min,
            y_max: *y_max,
            angle: *angle,
        },
        GateShape::Polygon { vertices } => GateKind::Polygon {
            vertices: vertices.clone(),
        },
        GateShape::Quadrant { x, y, quad } => GateKind::Quadrant {
            x: *x,
            y: *y,
            quad: quadrant_from_id(quad),
        },
        GateShape::Bilinear {
            x1,
            y1,
            x2,
            y2,
            below,
        } => GateKind::Bilinear {
            x1: *x1,
            y1: *y1,
            x2: *x2,
            y2: *y2,
            below: *below,
        },
    }
}

fn label_to_project(l: &LabelStyle) -> ProjectLabel {
    ProjectLabel {
        text: l.text.clone(),
        size: l.size,
        rotation_deg: l.rotation_deg,
        font: label_font_id(l.font).to_string(),
        color: [l.color.r(), l.color.g(), l.color.b()],
    }
}

fn label_from_project(l: &ProjectLabel) -> LabelStyle {
    LabelStyle {
        text: l.text.clone(),
        size: l.size,
        rotation_deg: l.rotation_deg,
        font: label_font_from_id(&l.font),
        color: egui::Color32::from_rgb(l.color[0], l.color[1], l.color[2]),
    }
}

/// Data-space range for auto-scaling that respects the axis scale.
fn bounds_for(values: &[f64], scale: AxisScale) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &v in values {
        if !v.is_finite() {
            continue;
        }
        if scale == AxisScale::Log && v <= 0.0 {
            continue;
        }
        min = min.min(v);
        max = max.max(v);
    }
    if !min.is_finite() {
        return match scale {
            AxisScale::Linear => (0.0, 1.0),
            AxisScale::Log => (1.0, 1000.0),
        };
    }
    if (max - min).abs() < f64::EPSILON {
        let pad = if min.abs() > 1.0 { min.abs() * 0.1 } else { 1.0 };
        return (min - pad, max + pad);
    }
    match scale {
        AxisScale::Linear => {
            let pad = (max - min) * 0.05;
            (min - pad, max + pad)
        }
        AxisScale::Log => {
            let l0 = min.log10();
            let l1 = max.log10();
            let pad = (l1 - l0) * 0.05;
            (10f64.powf(l0 - pad), 10f64.powf(l1 + pad))
        }
    }
}

impl FlowersApp {
    fn current_fcs(&self) -> Option<&FcsFile> {
        self.current_sample.map(|i| &self.samples[i].fcs)
    }

    fn load_file(&mut self, path: &std::path::Path) {
        match std::fs::read(path) {
            Ok(bytes) => match FcsParser::parse_bytes(&bytes) {
                Ok(fcs) => {
                    let id = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let params = fcs.parameters.len();
                    let events = fcs.event_count;
                    let had_gates = !self.gates.is_empty();
                    let (dx, dy) = default_param_pair(&fcs);
                    self.samples.push(SampleEntry {
                        id: id.clone(),
                        path: path.to_path_buf(),
                        fcs,
                        apply_gates: !had_gates,
                    });
                    let idx = self.samples.len() - 1;
                    self.current_sample = Some(idx);
                    self.x_param = Some(dx);
                    self.y_param = Some(dy);
                    self.plot_points.clear();
                    // Ask before applying an existing gating strategy to a new
                    // file (the user decides per file).
                    if had_gates {
                        self.pending_apply_prompt = Some(idx);
                    }
                    self.update_plot_points();
                    self.set_status(format!(
                        "Loaded {id}: {events} events × {params} parameters"
                    ));
                }
                Err(e) => self.set_status(format!("Error parsing FCS: {e}")),
            },
            Err(e) => self.set_status(format!("Error reading file: {e}")),
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
    }

    // -- undo / redo --------------------------------------------------------

    /// Captures the current editable state.
    fn snapshot(&self) -> UndoState {
        UndoState {
            gates: self.gates.clone(),
            selected_gate: self.selected_gate,
            x_param: self.x_param,
            y_param: self.y_param,
            view_mode: self.view_mode,
            x_scale: self.x_scale,
            y_scale: self.y_scale,
            point_color: self.point_color,
            gradient: self.gradient,
            contour_levels: self.contour_levels,
            x_range: self.x_range,
            y_range: self.y_range,
            layout_panels: self.layout_panels.clone(),
            layout_columns: self.layout_columns,
            layout_selected: self.layout_selected,
        }
    }

    /// Restores a snapshot, refreshing derived data (scatter points, gates).
    fn apply_state(&mut self, state: UndoState) {
        self.gates = state.gates;
        self.selected_gate = state.selected_gate;
        self.x_param = state.x_param;
        self.y_param = state.y_param;
        self.x_scale = state.x_scale;
        self.y_scale = state.y_scale;
        // Recompute scatter points + populations for the restored parameters,
        // then override the auto-fitted axis ranges with the stored ones.
        self.update_plot_points();
        self.x_range = state.x_range;
        self.y_range = state.y_range;
        self.view_mode = state.view_mode;
        self.point_color = state.point_color;
        self.gradient = state.gradient;
        self.contour_levels = state.contour_levels;
        self.layout_panels = state.layout_panels;
        self.layout_columns = state.layout_columns;
        self.layout_selected = state.layout_selected;
        if self.layout_selected.is_some_and(|s| s >= self.layout_panels.len()) {
            self.layout_selected = None;
        }
    }

    fn can_undo(&self) -> bool {
        self.history_idx > 0
    }

    fn can_redo(&self) -> bool {
        self.history_idx + 1 < self.history.len()
    }

    fn undo(&mut self) {
        if self.can_undo() {
            self.history_idx -= 1;
            let state = self.history[self.history_idx].clone();
            self.apply_state(state);
            self.history_skip = true;
            self.set_status("Undo");
        }
    }

    fn redo(&mut self) {
        if self.can_redo() {
            self.history_idx += 1;
            let state = self.history[self.history_idx].clone();
            self.apply_state(state);
            self.history_skip = true;
            self.set_status("Redo");
        }
    }

    /// Records the current state if it differs from the last snapshot.
    ///
    /// Called once per frame. While the pointer is down (mid-drag) the state is
    /// not recorded, so an entire drag collapses into a single undo step.
    fn record_history(&mut self, pointer_down: bool) {
        if self.history_skip {
            self.history_skip = false;
            return;
        }
        let current = self.snapshot();
        let changed = self
            .history
            .get(self.history_idx)
            .map(|s| !s.same_document(&current))
            .unwrap_or(true);
        if !changed || pointer_down {
            return;
        }
        // Drop any redo entries and append the new state.
        self.history.truncate(self.history_idx + 1);
        self.history.push(current);
        const MAX_HISTORY: usize = 200;
        if self.history.len() > MAX_HISTORY {
            let excess = self.history.len() - MAX_HISTORY;
            self.history.drain(0..excess);
        }
        self.history_idx = self.history.len() - 1;
    }

    // -- workspace (.wsp) ---------------------------------------------------

    /// Resets the application to an empty workspace, keeping the theme.
    fn new_workspace(&mut self) {
        self.samples.clear();
        self.gates.clear();
        self.layout_panels.clear();
        self.selected_gate = None;
        self.layout_selected = None;
        self.current_sample = None;
        self.x_param = None;
        self.y_param = None;
        self.plot_points.clear();
        self.pop_indices.clear();
        self.children.clear();
        self.pending_apply_prompt = None;
        self.popout = PopoutState::default();
        self.view_mode = ViewMode::Dots;
        self.x_scale = AxisScale::Linear;
        self.y_scale = AxisScale::Linear;
        self.gradient = true;
        self.contour_levels = 8;
        self.point_color = egui::Color32::from_rgb(31, 119, 180);
        self.x_range = (0.0, 1.0);
        self.y_range = (0.0, 1.0);
        self.layout_columns = 2;
        self.history.clear();
        self.history_idx = 0;
        self.record_history(false);
        self.set_status("New workspace");
    }

    /// Builds a serialisable snapshot of the current analysis.
    fn to_project(&self) -> Project {
        let samples = self
            .samples
            .iter()
            .map(|s| ProjectSample {
                path: s.path.to_string_lossy().to_string(),
                apply_gates: s.apply_gates,
            })
            .collect();
        let gates = self
            .gates
            .iter()
            .map(|g| ProjectGate {
                name: g.name.clone(),
                x_idx: g.x_idx,
                y_idx: g.y_idx,
                parent: g.parent,
                owner: g.owner,
                apply_to_all: g.apply_to_all,
                color: [g.color.r(), g.color.g(), g.color.b()],
                kind: gate_shape_from_kind(&g.kind),
            })
            .collect();
        let layout = self
            .layout_panels
            .iter()
            .map(|p| ProjectPanel {
                sample: p.sample,
                x: p.x,
                y: p.y,
                view: view_mode_id(p.view).to_string(),
                gate: p.gate,
                title: p.title.clone(),
                rect: [p.rect.min.x, p.rect.min.y, p.rect.width(), p.rect.height()],
                x_label: label_to_project(&p.x_label),
                y_label: label_to_project(&p.y_label),
            })
            .collect();
        Project {
            version: crate::project::PROJECT_VERSION,
            samples,
            current_sample: self.current_sample,
            x_param: self.x_param,
            y_param: self.y_param,
            selected_gate: self.selected_gate,
            view: ViewSettings {
                mode: view_mode_id(self.view_mode).to_string(),
                x_scale: axis_scale_id(self.x_scale).to_string(),
                y_scale: axis_scale_id(self.y_scale).to_string(),
                point_color: [self.point_color.r(), self.point_color.g(), self.point_color.b()],
                gradient: self.gradient,
                contour_levels: self.contour_levels,
                x_range: self.x_range,
                y_range: self.y_range,
            },
            gates,
            layout,
            layout_columns: self.layout_columns,
        }
    }

    /// Writes the current analysis to `path` as a `.wsp` (JSON) file.
    fn save_workspace(&mut self, path: &std::path::Path) {
        let project = self.to_project();
        match crate::project::to_json(&project) {
            Ok(json) => match std::fs::write(path, json) {
                Ok(()) => self.set_status(format!(
                    "Saved workspace: {} sample(s), {} gate(s)",
                    project.samples.len(),
                    project.gates.len()
                )),
                Err(e) => self.set_status(format!("Error writing workspace: {e}")),
            },
            Err(e) => self.set_status(format!("Error encoding workspace: {e}")),
        }
    }

    /// Loads a `.wsp` file, re-reading the referenced FCS files.
    fn load_workspace(&mut self, path: &std::path::Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.set_status(format!("Error reading workspace: {e}"));
                return;
            }
        };
        match crate::project::from_json(&text) {
            Ok(project) => self.apply_project(project),
            Err(e) => self.set_status(format!("Error parsing workspace: {e}")),
        }
    }

    /// Replaces the current state with a loaded project.
    fn apply_project(&mut self, project: Project) {
        self.samples.clear();
        self.gates.clear();
        self.layout_panels.clear();
        self.selected_gate = None;
        self.layout_selected = None;
        self.current_sample = None;
        self.pending_apply_prompt = None;
        self.popout = PopoutState::default();

        // View settings (axis ranges are restored after the samples load, since
        // `update_plot_points` auto-fits them).
        self.view_mode = view_mode_from_id(&project.view.mode);
        self.x_scale = axis_scale_from_id(&project.view.x_scale);
        self.y_scale = axis_scale_from_id(&project.view.y_scale);
        let [r, g, b] = project.view.point_color;
        self.point_color = egui::Color32::from_rgb(r, g, b);
        self.gradient = project.view.gradient;
        self.contour_levels = project.view.contour_levels;

        // Load the referenced FCS files.
        let mut missing: Vec<String> = Vec::new();
        for s in &project.samples {
            let p = std::path::Path::new(&s.path);
            match std::fs::read(p) {
                Ok(bytes) => match FcsParser::parse_bytes(&bytes) {
                    Ok(fcs) => {
                        let id = p
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                            .to_string();
                        self.samples.push(SampleEntry {
                            id,
                            path: p.to_path_buf(),
                            fcs,
                            apply_gates: s.apply_gates,
                        });
                    }
                    Err(e) => missing.push(format!("{} (parse: {e})", s.path)),
                },
                Err(e) => missing.push(format!("{} ({e})", s.path)),
            }
        }

        // Gates.
        for pg in &project.gates {
            self.gates.push(GateDef {
                name: pg.name.clone(),
                x_idx: pg.x_idx,
                y_idx: pg.y_idx,
                parent: pg.parent,
                kind: gate_kind_from_shape(&pg.kind),
                color: egui::Color32::from_rgb(pg.color[0], pg.color[1], pg.color[2]),
                owner: pg.owner,
                apply_to_all: pg.apply_to_all,
            });
        }

        // Layout panels.
        self.layout_columns = project.layout_columns.max(1);
        for pp in &project.layout {
            if pp.sample >= self.samples.len() {
                continue;
            }
            let preview = self.make_layout_preview(pp.sample, pp.x, pp.y);
            self.layout_panels.push(LayoutPanelSpec {
                sample: pp.sample,
                x: pp.x,
                y: pp.y,
                view: view_mode_from_id(&pp.view),
                gate: pp.gate,
                title: pp.title.clone(),
                rect: egui::Rect::from_min_size(
                    egui::pos2(pp.rect[0], pp.rect[1]),
                    egui::vec2(pp.rect[2].max(1.0), pp.rect[3].max(1.0)),
                ),
                preview,
                x_label: label_from_project(&pp.x_label),
                y_label: label_from_project(&pp.y_label),
            });
        }

        // Selection.
        self.current_sample = project
            .current_sample
            .filter(|&i| i < self.samples.len())
            .or_else(|| (!self.samples.is_empty()).then_some(0));
        self.x_param = project.x_param;
        self.y_param = project.y_param;
        self.selected_gate = project.selected_gate.filter(|&i| i < self.gates.len());
        if let Some(fcs) = self.current_fcs() {
            let (dx, dy) = default_param_pair(fcs);
            if self.x_param.is_none() {
                self.x_param = Some(dx);
            }
            if self.y_param.is_none() {
                self.y_param = Some(dy);
            }
        }
        self.update_plot_points();
        // Override the auto-fitted ranges with the saved ones.
        self.x_range = project.view.x_range;
        self.y_range = project.view.y_range;

        // The loaded state is the new baseline for undo / redo.
        self.history.clear();
        self.history_idx = 0;
        self.record_history(false);

        if missing.is_empty() {
            self.set_status(format!(
                "Loaded workspace: {} sample(s), {} gate(s)",
                self.samples.len(),
                self.gates.len()
            ));
        } else {
            self.set_status(format!(
                "Loaded workspace; {} file(s) could not be opened: {}",
                missing.len(),
                missing.join("; ")
            ));
        }
    }

    /// Load files (e.g. from the command line) and prime the first plot.
    pub fn open_paths(&mut self, paths: &[std::path::PathBuf]) {
        for p in paths {
            self.load_file(p);
        }
    }

    /// Rebuild scatter points, data ranges and axis ranges for the current
    /// sample/parameter selection.
    fn update_plot_points(&mut self) {
        self.plot_points.clear();
        let Some(fcs) = self.current_fcs() else {
            self.recompute_populations();
            return;
        };
        let (Some(x), Some(y)) = (self.x_param, self.y_param) else {
            self.recompute_populations();
            return;
        };
        let (_, n_params) = fcs.data.dim();
        if x >= n_params || y >= n_params {
            return;
        }

        let xs: Vec<f64> = fcs.data.column(x).iter().map(|v| *v as f64).collect();
        let ys: Vec<f64> = fcs.data.column(y).iter().map(|v| *v as f64).collect();
        self.plot_points = xs.iter().zip(ys.iter()).map(|(&a, &b)| [a, b]).collect();

        self.x_data_range = bounds_for(&xs, self.x_scale);
        self.y_data_range = bounds_for(&ys, self.y_scale);
        self.x_range = self.x_data_range;
        self.y_range = self.y_data_range;
        self.recompute_populations();
    }

    /// Recompute the event indices of every gate for the current sample.
    fn recompute_populations(&mut self) {
        let apply_gates = self
            .current_sample
            .and_then(|i| self.samples.get(i))
            .map(|s| s.apply_gates)
            .unwrap_or(false);
        self.pop_indices = match self.current_fcs() {
            Some(fcs) => {
                compute_gate_populations(&fcs.data, &self.gates, self.current_sample, apply_gates)
            }
            None => vec![Vec::new(); self.gates.len()],
        };
        self.children = children_map(&self.gates);
    }

    /// Auto-adjust an axis to fit the current points.
    fn auto_x(&mut self) {
        self.x_data_range = bounds_for(
            &self.plot_points.iter().map(|p| p[0]).collect::<Vec<_>>(),
            self.x_scale,
        );
        self.x_range = self.x_data_range;
    }

    fn auto_y(&mut self) {
        self.y_data_range = bounds_for(
            &self.plot_points.iter().map(|p| p[1]).collect::<Vec<_>>(),
            self.y_scale,
        );
        self.y_range = self.y_data_range;
    }

    // -- gate creation ------------------------------------------------------

    /// Add a single gate under an explicit parent, returning its index.
    fn push_gate(
        &mut self,
        name: String,
        kind: GateKind,
        parent: Option<usize>,
        x_idx: usize,
        y_idx: usize,
    ) -> usize {
        let color = gate_color(self.gates.len());
        self.gates.push(GateDef {
            name,
            x_idx,
            y_idx,
            parent,
            kind,
            color,
            owner: self.current_sample,
            apply_to_all: true,
        });
        self.gates.len() - 1
    }

    /// Create gate(s) from a drag rectangle for an explicit parameter pair.
    /// Returns the indices of the gates that were created.
    fn create_gates(
        &mut self,
        start: [f64; 2],
        end: [f64; 2],
        x_idx: usize,
        y_idx: usize,
        parent: Option<usize>,
        new_type: NewGateType,
    ) -> Vec<usize> {
        let mut created = Vec::new();
        let x_min = start[0].min(end[0]);
        let x_max = start[0].max(end[0]);
        let y_min = start[1].min(end[1]);
        let y_max = start[1].max(end[1]);
        let cx = (x_min + x_max) / 2.0;
        let cy = (y_min + y_max) / 2.0;

        match new_type {
            NewGateType::Rect => {
                if (x_max - x_min).abs() < f64::EPSILON || (y_max - y_min).abs() < f64::EPSILON {
                    return created;
                }
                let n = self.gates.len() + 1;
                created.push(self.push_gate(
                    format!("Rect {n}"),
                    GateKind::Rect {
                        x_min,
                        x_max,
                        y_min,
                        y_max,
                        angle: 0.0,
                    },
                    parent,
                    x_idx,
                    y_idx,
                ));
            }
            NewGateType::Ellipse => {
                if (x_max - x_min).abs() < f64::EPSILON || (y_max - y_min).abs() < f64::EPSILON {
                    return created;
                }
                let n = self.gates.len() + 1;
                created.push(self.push_gate(
                    format!("Ellipse {n}"),
                    GateKind::Ellipse {
                        x_min,
                        x_max,
                        y_min,
                        y_max,
                        angle: 0.0,
                    },
                    parent,
                    x_idx,
                    y_idx,
                ));
            }
            NewGateType::Polygon => {
                if (x_max - x_min).abs() < f64::EPSILON || (y_max - y_min).abs() < f64::EPSILON {
                    return created;
                }
                let n = self.gates.len() + 1;
                let verts = regular_polygon(cx, cy, (x_max - x_min) / 2.0, (y_max - y_min) / 2.0, 5);
                created.push(self.push_gate(
                    format!("Polygon {n}"),
                    GateKind::Polygon { vertices: verts },
                    parent,
                    x_idx,
                    y_idx,
                ));
            }
            NewGateType::Quadrant => {
                created = self.create_quadrant_at([cx, cy], x_idx, y_idx, parent);
            }
            NewGateType::Bilinear => {
                // Use the drag itself as the dividing line: the gate is the
                // region under that line, bounded by the segment's X extent.
                let len = ((end[0] - start[0]).powi(2) + (end[1] - start[1]).powi(2)).sqrt();
                if len < f64::EPSILON {
                    return created;
                }
                let n = self.gates.len() + 1;
                created.push(self.push_gate(
                    format!("Bilinear {n}"),
                    GateKind::Bilinear {
                        x1: start[0],
                        y1: start[1],
                        x2: end[0],
                        y2: end[1],
                        below: true,
                    },
                    parent,
                    x_idx,
                    y_idx,
                ));
            }
        }
        self.finish_gate_creation();
        created
    }

    /// Create gate(s) from a drag on the main plot (uses the current X/Y pair).
    fn create_gate_from_drag(&mut self, start: [f64; 2], end: [f64; 2]) {
        let (Some(x), Some(y)) = (self.x_param, self.y_param) else {
            return;
        };
        let parent = self.selected_gate;
        let ty = self.new_gate_type;
        let created = self.create_gates(start, end, x, y, parent, ty);
        if let Some(&last) = created.last() {
            self.selected_gate = Some(last);
        }
    }

    /// Create the four quadrant gates at a split point (as siblings).
    fn create_quadrant_at(
        &mut self,
        split: [f64; 2],
        x_idx: usize,
        y_idx: usize,
        parent: Option<usize>,
    ) -> Vec<usize> {
        let base = self.gates.len() + 1;
        let mut created = Vec::new();
        for (i, quad) in [
            Quadrant::UpperLeft,
            Quadrant::UpperRight,
            Quadrant::LowerLeft,
            Quadrant::LowerRight,
        ]
        .into_iter()
        .enumerate()
        {
            let kind = GateKind::Quadrant {
                x: split[0],
                y: split[1],
                quad: quad.clone(),
            };
            created.push(self.push_gate(
                format!("Q{base}.{} {}", i + 1, quad_label(&quad)),
                kind,
                parent,
                x_idx,
                y_idx,
            ));
        }
        created
    }

    /// Create a single line gate as a horizontal threshold at `center[1]`,
    /// spanning `center[0] ± radius`. Used by "+ Add Gate" and click-to-place.
    fn create_bilinear_at(
        &mut self,
        center: [f64; 2],
        radius: f64,
        x_idx: usize,
        y_idx: usize,
        parent: Option<usize>,
    ) -> Vec<usize> {
        let base = self.gates.len() + 1;
        let r = radius.max(1.0);
        let kind = GateKind::Bilinear {
            x1: center[0] - r,
            y1: center[1],
            x2: center[0] + r,
            y2: center[1],
            below: true,
        };
        vec![self.push_gate(
            format!("Bilinear {base}"),
            kind,
            parent,
            x_idx,
            y_idx,
        )]
    }

    fn finish_gate_creation(&mut self) {
        self.interact.drag_start = None;
        self.interact.drag_current = None;
        self.sync_name_edit();
        self.recompute_populations();
        self.set_status(format!("{} gates defined", self.gates.len()));
    }

    fn sync_name_edit(&mut self) {
        self.name_edit = self
            .selected_gate
            .and_then(|i| self.gates.get(i))
            .map(|g| g.name.clone())
            .unwrap_or_default();
    }

    /// Remove a gate, re-parenting its children and shifting indices.
    fn remove_gate(&mut self, idx: usize) {
        if idx >= self.gates.len() {
            return;
        }
        let removed_parent = self.gates[idx].parent;
        self.gates.remove(idx);
        for gate in &mut self.gates {
            match gate.parent {
                Some(p) if p == idx => gate.parent = removed_parent,
                Some(p) if p > idx => gate.parent = Some(p - 1),
                _ => {}
            }
        }
        self.selected_gate = if self.gates.is_empty() {
            None
        } else {
            Some(idx.min(self.gates.len() - 1))
        };
        self.popout.gate = None;
        self.popout.selected_gate = None;
        self.sync_name_edit();
        self.recompute_populations();
        self.set_status("Removed gate");
    }

    fn add_default_gate(&mut self) {
        let (Some(x_idx), Some(y_idx)) = (self.x_param, self.y_param) else {
            self.set_status("Select X and Y parameters first");
            return;
        };
        let parent = self.selected_gate;
        let cx = (self.x_range.0 + self.x_range.1) / 2.0;
        let cy = (self.y_range.0 + self.y_range.1) / 2.0;
        let base = self.gates.len() + 1;
        let created = match self.new_gate_type {
            NewGateType::Quadrant => self.create_quadrant_at([cx, cy], x_idx, y_idx, parent),
            NewGateType::Bilinear => {
                let r = ((self.x_range.1 - self.x_range.0).abs()
                    .max((self.y_range.1 - self.y_range.0).abs()))
                    / 2.0;
                self.create_bilinear_at([cx, cy], r, x_idx, y_idx, parent)
            }
            NewGateType::Rect => vec![self.push_gate(
                format!("Rect {base}"),
                GateKind::Rect {
                    x_min: self.x_range.0,
                    x_max: self.x_range.1,
                    y_min: self.y_range.0,
                    y_max: self.y_range.1,
                    angle: 0.0,
                },
                parent,
                x_idx,
                y_idx,
            )],
            NewGateType::Ellipse => vec![self.push_gate(
                format!("Ellipse {base}"),
                GateKind::Ellipse {
                    x_min: self.x_range.0,
                    x_max: self.x_range.1,
                    y_min: self.y_range.0,
                    y_max: self.y_range.1,
                    angle: 0.0,
                },
                parent,
                x_idx,
                y_idx,
            )],
            NewGateType::Polygon => {
                let rx = (self.x_range.1 - self.x_range.0).abs() / 2.0;
                let ry = (self.y_range.1 - self.y_range.0).abs() / 2.0;
                let verts = regular_polygon(cx, cy, rx, ry, 5);
                vec![self.push_gate(
                    format!("Polygon {base}"),
                    GateKind::Polygon { vertices: verts },
                    parent,
                    x_idx,
                    y_idx,
                )]
            }
        };
        if let Some(&last) = created.last() {
            self.selected_gate = Some(last);
        }
        self.finish_gate_creation();
    }

    /// Create a quadrant / bilinear gate at a click position.
    ///
    /// `popout` selects which view (main plot vs. detached window) the click
    /// came from, so the new gate gets the right parent and parameter pair.
    fn create_click_gate(&mut self, p: [f64; 2], popout: bool) {
        let (x_idx, y_idx, parent, ty, span) = if popout {
            let (Some(x), Some(y)) = (self.popout.x_param, self.popout.y_param) else {
                return;
            };
            let span = (self.popout.x_range.1 - self.popout.x_range.0)
                .abs()
                .max((self.popout.y_range.1 - self.popout.y_range.0).abs());
            (x, y, self.popout.gate, self.popout.new_gate_type, span)
        } else {
            let (Some(x), Some(y)) = (self.x_param, self.y_param) else {
                return;
            };
            let span = (self.x_range.1 - self.x_range.0)
                .abs()
                .max((self.y_range.1 - self.y_range.0).abs());
            (x, y, self.selected_gate, self.new_gate_type, span)
        };
        let created = match ty {
            NewGateType::Quadrant => self.create_quadrant_at(p, x_idx, y_idx, parent),
            NewGateType::Bilinear => self.create_bilinear_at(p, span / 2.0, x_idx, y_idx, parent),
            _ => return,
        };
        if let Some(&last) = created.last() {
            if popout {
                self.popout.selected_gate = Some(last);
            } else {
                self.selected_gate = Some(last);
            }
        }
        self.finish_gate_creation();
    }

    /// Open the detached window for a population and prime its view controls
    /// from the main plot.
    fn open_popout(&mut self, gate_idx: usize) {
        let Some(gate) = self.gates.get(gate_idx) else {
            return;
        };
        let (x, y) = (gate.x_idx, gate.y_idx);
        self.popout.gate = Some(gate_idx);
        self.popout.x_param = Some(x);
        self.popout.y_param = Some(y);
        self.popout.open = true;
        self.popout.selected_gate = None;
        self.popout.interact = Interact::default();
        self.popout.view = self.view_mode;
        self.popout.x_scale = self.x_scale;
        self.popout.y_scale = self.y_scale;
        self.popout.point_color = self.point_color;
        self.popout.gradient = self.gradient;
        self.popout.contour_levels = self.contour_levels;
        self.popout.new_gate_type = self.new_gate_type;
        self.popout_auto_range();
    }

    /// Auto-fit the detached window's ranges to its population.
    fn popout_auto_range(&mut self) {
        let Some(gate_idx) = self.popout.gate else {
            return;
        };
        let (Some(x), Some(y)) = (self.popout.x_param, self.popout.y_param) else {
            return;
        };
        let pts = self.population_points_xy(gate_idx, x, y);
        let xs: Vec<f64> = pts.iter().map(|p| p[0]).collect();
        let ys: Vec<f64> = pts.iter().map(|p| p[1]).collect();
        self.popout.x_range = bounds_for(&xs, self.popout.x_scale);
        self.popout.y_range = bounds_for(&ys, self.popout.y_scale);
    }

    /// Points of a population for an explicit parameter pair.
    fn population_points_xy(&self, gate_idx: usize, x: usize, y: usize) -> Vec<[f64; 2]> {
        let Some(fcs) = self.current_fcs() else {
            return Vec::new();
        };
        let (_, n_params) = fcs.data.dim();
        if x >= n_params || y >= n_params {
            return Vec::new();
        }
        let Some(idxs) = self.pop_indices.get(gate_idx) else {
            return Vec::new();
        };
        idxs.iter()
            .map(|&e| [fcs.data[[e, x]] as f64, fcs.data[[e, y]] as f64])
            .collect()
    }

    /// Controls for the detached population window. Mirrors the main window so
    /// every plotting feature (axes, scales, ranges, colour, view, gating and
    /// gate manipulation) is available there too.
    fn popout_controls(&mut self, ui: &mut egui::Ui, gate_idx: usize) {
        let names: Vec<String> = match self.current_fcs() {
            Some(f) => (0..f.parameters.len())
                .map(|i| param_display_name(f, i))
                .collect(),
            None => Vec::new(),
        };
        let subset_len = self.pop_indices.get(gate_idx).map(|v| v.len()).unwrap_or(0);

        ui.horizontal(|ui| {
            let xt = self
                .popout
                .x_param
                .and_then(|i| names.get(i).cloned())
                .unwrap_or_else(|| "Select X".to_string());
            egui::ComboBox::new("popout_x", "X")
                .selected_text(xt)
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.popout.x_param, Some(i), name.clone());
                    }
                });
            let yt = self
                .popout
                .y_param
                .and_then(|i| names.get(i).cloned())
                .unwrap_or_else(|| "Select Y".to_string());
            egui::ComboBox::new("popout_y", "Y")
                .selected_text(yt)
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.popout.y_param, Some(i), name.clone());
                    }
                });
            if ui.small_button("Auto range").clicked() {
                self.popout_auto_range();
            }
            ui.label(format!("{subset_len} events"));
        });

        ui.horizontal(|ui| {
            ui.label("X scale:");
            for scale in [AxisScale::Linear, AxisScale::Log] {
                if ui
                    .selectable_label(self.popout.x_scale == scale, scale.label())
                    .clicked()
                {
                    self.popout.x_scale = scale;
                    self.popout_auto_range();
                }
            }
            ui.label("range:");
            let span = (self.popout.x_range.1 - self.popout.x_range.0).abs().max(1.0);
            ui.add(range_drag_value(&mut self.popout.x_range.0, span));
            ui.add(range_drag_value(&mut self.popout.x_range.1, span));
        });
        ui.horizontal(|ui| {
            ui.label("Y scale:");
            for scale in [AxisScale::Linear, AxisScale::Log] {
                if ui
                    .selectable_label(self.popout.y_scale == scale, scale.label())
                    .clicked()
                {
                    self.popout.y_scale = scale;
                    self.popout_auto_range();
                }
            }
            ui.label("range:");
            let span = (self.popout.y_range.1 - self.popout.y_range.0).abs().max(1.0);
            ui.add(range_drag_value(&mut self.popout.y_range.0, span));
            ui.add(range_drag_value(&mut self.popout.y_range.1, span));
        });

        ui.horizontal(|ui| {
            ui.label("Dot colour:");
            ui.color_edit_button_srgba(&mut self.popout.point_color);
            ui.checkbox(&mut self.popout.gradient, "Gradient");
            ui.label("View:");
            for mode in [ViewMode::Dots, ViewMode::Contour, ViewMode::Histogram] {
                if ui
                    .selectable_label(self.popout.view == mode, mode.label())
                    .clicked()
                {
                    self.popout.view = mode;
                }
            }
            if matches!(self.popout.view, ViewMode::Contour) {
                ui.add(egui::Slider::new(&mut self.popout.contour_levels, 3..=20).text("levels"));
            }
        });

        ui.horizontal(|ui| {
            ui.label("New gate:");
            for gt in NewGateType::ALL {
                ui.radio_value(&mut self.popout.new_gate_type, gt, gt.label());
            }
        });

        // Manipulation buttons for the gate selected inside the popout.
        if let Some(gi) = self.popout.selected_gate {
            if gi < self.gates.len() {
                let span = (self.popout.x_range.1 - self.popout.x_range.0)
                    .abs()
                    .max((self.popout.y_range.1 - self.popout.y_range.0).abs())
                    .max(1.0);
                let step = span * 0.02;
                ui.horizontal(|ui| {
                    ui.label("Transform:");
                    let mut t = false;
                    if ui.small_button("⬅").clicked() {
                        self.gates[gi].kind.translate(-step, 0.0);
                        t = true;
                    }
                    if ui.small_button("➡").clicked() {
                        self.gates[gi].kind.translate(step, 0.0);
                        t = true;
                    }
                    if ui.small_button("⬆").clicked() {
                        self.gates[gi].kind.translate(0.0, step);
                        t = true;
                    }
                    if ui.small_button("⬇").clicked() {
                        self.gates[gi].kind.translate(0.0, -step);
                        t = true;
                    }
                    if ui.small_button("⟲").on_hover_text("Rotate -5°").clicked() {
                        let c = self.gates[gi].kind.center();
                        self.gates[gi].kind.rotate_about(c, (-5.0f64).to_radians());
                        t = true;
                    }
                    if ui.small_button("⟳").on_hover_text("Rotate +5°").clicked() {
                        let c = self.gates[gi].kind.center();
                        self.gates[gi].kind.rotate_about(c, (5.0f64).to_radians());
                        t = true;
                    }
                    if ui.small_button("➕").on_hover_text("Enlarge").clicked() {
                        let c = self.gates[gi].kind.center();
                        self.gates[gi].kind.scale_about(c, 1.1);
                        t = true;
                    }
                    if ui.small_button("➖").on_hover_text("Shrink").clicked() {
                        let c = self.gates[gi].kind.center();
                        self.gates[gi].kind.scale_about(c, 0.9);
                        t = true;
                    }
                    if t {
                        self.recompute_populations();
                    }
                });
            }
        }
    }

    // -- rendering helpers --------------------------------------------------

    /// Gate overlays (in data coordinates) for the active X/Y pair.
    fn gate_visuals(&self) -> Vec<GateVisual> {
        let (Some(x), Some(y)) = (self.x_param, self.y_param) else {
            return Vec::new();
        };
        self.gate_visuals_for(x, y, self.x_range, self.y_range, self.selected_gate)
    }

    /// Gate overlays for an explicit parameter pair and axis ranges.
    fn gate_visuals_for(
        &self,
        x: usize,
        y: usize,
        x_range: (f64, f64),
        y_range: (f64, f64),
        selected: Option<usize>,
    ) -> Vec<GateVisual> {
        let mut out = Vec::new();
        for (i, gate) in self.gates.iter().enumerate() {
            if gate.x_idx != x || gate.y_idx != y {
                continue;
            }
            out.push(GateVisual {
                fill: gate.kind.polygon(x_range, y_range),
                lines: gate.kind.divider_lines(x_range, y_range),
                color: gate.color,
                selected: selected == Some(i),
            });
        }
        out
    }

    /// Build the plot configuration for the main view.
    fn main_plot_cfg(&self) -> PlotCfg {
        let x = self.x_param.unwrap_or(0);
        let y = self.y_param.unwrap_or(0);
        let x_label = self
            .current_fcs()
            .and_then(|f| self.x_param.map(|x| param_display_name(f, x)))
            .unwrap_or_default();
        let y_label = self
            .current_fcs()
            .and_then(|f| self.y_param.map(|y| param_display_name(f, y)))
            .unwrap_or_default();
        let handles = match self.selected_gate {
            Some(i) if self.gates.get(i).map(|g| (g.x_idx, g.y_idx)) == Some((x, y)) => {
                self.gates[i].kind.handles()
            }
            _ => Vec::new(),
        };
        PlotCfg {
            points: self.plot_points.clone(),
            x_range: self.x_range,
            y_range: self.y_range,
            x_scale: self.x_scale,
            y_scale: self.y_scale,
            point_color: self.point_color,
            gradient: self.gradient,
            view: self.view_mode,
            contour_levels: self.contour_levels,
            gates: self.gate_visuals(),
            handles,
            size: (560.0, 520.0),
            x_label: format!("{} ({})", x_label, scale_label(self.x_scale)),
            y_label: if matches!(self.view_mode, ViewMode::Histogram) {
                "Count".to_string()
            } else {
                format!("{} ({})", y_label, scale_label(self.y_scale))
            },
            interactive: true,
            line_preview: matches!(self.new_gate_type, NewGateType::Bilinear),
        }
    }

    /// Build the SVG for the current main plot, with gate overlays.
    /// Returns the SVG text plus its logical `(width, height)`.
    fn current_plot_svg(&self) -> Result<(String, u32, u32), String> {
        let Some(fcs) = self.current_fcs() else {
            return Err("No sample loaded to export".to_string());
        };
        let (Some(x), Some(y)) = (self.x_param, self.y_param) else {
            return Err("Select X and Y parameters to export".to_string());
        };

        let overlays: Vec<PlotOverlay> = self
            .gate_visuals()
            .into_iter()
            .map(|g| PlotOverlay {
                points: g.fill,
                stroke: color_to_hex(g.color),
                fill: None,
                closed: true,
            })
            .collect();

        let style = PlotStyle {
            point_color: color_to_hex(self.point_color),
            x_scale: self.x_scale,
            y_scale: self.y_scale,
            gradient: self.gradient,
            contour: matches!(self.view_mode, ViewMode::Contour),
            contour_levels: self.contour_levels,
            title: param_display_name(fcs, y),
            line_color: color_to_hex(egui::Color32::from_rgb(44, 160, 44)),
            ..Default::default()
        };

        let svg = if matches!(self.view_mode, ViewMode::Histogram) {
            crate::plot::histogram_svg(
                &fcs.data,
                x,
                &param_display_name(fcs, x),
                "Count",
                &[],
                Some((self.x_range.0 as f32, self.x_range.1 as f32)),
                50,
                &style,
            )
        } else {
            dot_plot_svg(
                &fcs.data,
                x,
                y,
                &param_display_name(fcs, x),
                &param_display_name(fcs, y),
                &[],
                Some((self.x_range.0 as f32, self.x_range.1 as f32)),
                Some((self.y_range.0 as f32, self.y_range.1 as f32)),
                &overlays,
                &style,
            )
        };
        Ok((svg, style.width, style.height))
    }

    /// Export the current plot (with gates) to SVG.
    fn export_svg(&mut self, path: &std::path::Path) {
        match self.current_plot_svg() {
            Ok((svg, _, _)) => match std::fs::write(path, &svg) {
                Ok(()) => self.set_status(format!("Exported plot to {}", path.display())),
                Err(e) => self.set_status(format!("Export failed: {e}")),
            },
            Err(e) => self.set_status(e),
        }
    }

    /// Export the current plot to a raster format (PNG / JPEG / TIFF).
    fn export_image(&mut self, path: &std::path::Path) {
        match self.current_plot_svg() {
            Ok((svg, w, h)) => {
                let scale = self.export_scale;
                match save_svg_as_image(&svg, path, w, h, scale) {
                    Ok(fmt) => self.set_status(format!(
                        "Exported {} ({}×{}, {}×) to {}",
                        fmt.label(),
                        (w as f32 * scale).round() as u32,
                        (h as f32 * scale).round() as u32,
                        scale,
                        path.display()
                    )),
                    Err(e) => self.set_status(format!("Image export failed: {e}")),
                }
            }
            Err(e) => self.set_status(e),
        }
    }

    /// Collect population rows across *all* loaded samples.
    fn population_rows(&self) -> Vec<PopulationRow> {
        let mut rows = Vec::new();
        for (sample_idx, sample) in self.samples.iter().enumerate() {
            let pops = compute_gate_populations(
                &sample.fcs.data,
                &self.gates,
                Some(sample_idx),
                sample.apply_gates,
            );
            for (i, gate) in self.gates.iter().enumerate() {
                if i >= pops.len() {
                    continue;
                }
                rows.push(PopulationRow {
                    sample: sample.id.clone(),
                    population: gate.name.clone(),
                    parent: gate.parent.map(|p| self.gates[p].name.clone()),
                    count: pops[i].len(),
                    total: sample.fcs.event_count,
                    parent_count: gate
                        .parent
                        .and_then(|p| pops.get(p).map(|v| v.len())),
                    gate_summary: Some(gate.kind.summary()),
                });
            }
        }
        rows
    }

    fn export_csv(&mut self, path: &std::path::Path) {
        let rows = self.population_rows();
        match write_populations_csv(path, &rows) {
            Ok(()) => self.set_status(format!(
                "Exported {} population rows to {}",
                rows.len(),
                path.display()
            )),
            Err(e) => self.set_status(format!("CSV export failed: {e}")),
        }
    }

    /// Build the panel SVGs and export the layout figure.
    /// Compose the layout canvas panels into a single SVG document.
    fn layout_svg(&self) -> (String, usize) {
        let mut max_x = 100.0f32;
        let mut max_y = 100.0f32;
        for p in &self.layout_panels {
            max_x = max_x.max(p.rect.max.x);
            max_y = max_y.max(p.rect.max.y);
        }
        let canvas_w = (max_x + 20.0) as u32;
        let canvas_h = (max_y + 20.0) as u32;

        let placements: Vec<LayoutPlacement> = self
            .layout_panels
            .iter()
            .filter_map(|spec| {
                let sample = self.samples.get(spec.sample)?;
                let fcs = &sample.fcs;
                let x_text = spec
                    .x_label
                    .text
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| param_display_name(fcs, spec.x));
                let y_text = spec
                    .y_label
                    .text
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| {
                        if matches!(spec.view, ViewMode::Histogram) {
                            "Count".to_string()
                        } else {
                            param_display_name(fcs, spec.y)
                        }
                    });
                let style = PlotStyle {
                    point_color: color_to_hex(self.point_color),
                    x_scale: AxisScale::Linear,
                    y_scale: AxisScale::Linear,
                    gradient: self.gradient,
                    contour: matches!(spec.view, ViewMode::Contour),
                    contour_levels: self.contour_levels,
                    title: spec.title.clone(),
                    x_label_size: spec.x_label.size,
                    y_label_size: spec.y_label.size,
                    x_label_rotation: spec.x_label.rotation_deg,
                    y_label_rotation: spec.y_label.rotation_deg,
                    x_label_font: spec.x_label.font.css().to_string(),
                    y_label_font: spec.y_label.font.css().to_string(),
                    ..Default::default()
                };
                let svg = if matches!(spec.view, ViewMode::Histogram) {
                    crate::plot::histogram_svg(
                        &fcs.data,
                        spec.x,
                        &x_text,
                        &y_text,
                        &[],
                        None,
                        50,
                        &style,
                    )
                } else {
                    let indices: Vec<usize> = spec
                        .gate
                        .map(|g| {
                            compute_gate_populations(
                                &fcs.data,
                                &self.gates,
                                Some(spec.sample),
                                sample.apply_gates,
                            )
                            .get(g)
                            .cloned()
                            .unwrap_or_default()
                        })
                        .unwrap_or_default();
                    dot_plot_svg(
                        &fcs.data,
                        spec.x,
                        spec.y,
                        &x_text,
                        &y_text,
                        &indices,
                        None,
                        None,
                        &[],
                        &style,
                    )
                };
                Some(LayoutPlacement {
                    title: spec.title.clone(),
                    svg,
                    svg_width: style.width,
                    svg_height: style.height,
                    x: spec.rect.min.x,
                    y: spec.rect.min.y,
                    width: spec.rect.width(),
                    height: spec.rect.height(),
                })
            })
            .collect();

        let svg = render_layout_free(canvas_w, canvas_h, &placements, &LayoutOptions::default());
        (svg, placements.len())
    }

    fn export_layout(&mut self, path: &std::path::Path) {
        let (svg, n) = self.layout_svg();
        match std::fs::write(path, &svg) {
            Ok(()) => self.set_status(format!(
                "Exported {n} panel layout to {}",
                path.display()
            )),
            Err(e) => self.set_status(format!("Layout export failed: {e}")),
        }
    }

    /// Export the composed layout to a raster format.
    fn export_layout_image(&mut self, path: &std::path::Path) {
        let (svg, n) = self.layout_svg();
        let (w, h) = layout_canvas_size(&self.layout_panels);
        match save_svg_as_image(&svg, path, w, h, self.export_scale) {
            Ok(fmt) => self.set_status(format!(
                "Exported {n}-panel layout as {} to {}",
                fmt.label(),
                path.display()
            )),
            Err(e) => self.set_status(format!("Layout image export failed: {e}")),
        }
    }

    /// Add the current main plot to the layout canvas.
    fn add_layout_current(&mut self) {
        let (Some(s), Some(x), Some(y)) = (self.current_sample, self.x_param, self.y_param)
        else {
            self.set_status("Select a sample and X/Y parameters first");
            return;
        };
        let title = param_display_name(&self.samples[s].fcs, y);
        let preview = self.make_layout_preview(s, x, y);
        let n = self.layout_panels.len();
        let cols = 3usize;
        let col = (n % cols) as f32;
        let row = (n / cols) as f32;
        let rect = egui::Rect::from_min_size(
            egui::pos2(20.0 + col * 300.0, 20.0 + row * 240.0),
            egui::vec2(280.0, 220.0),
        );
        self.layout_panels.push(LayoutPanelSpec {
            sample: s,
            x,
            y,
            view: self.view_mode,
            gate: None,
            title,
            rect,
            preview,
            x_label: LabelStyle::x_default(),
            y_label: LabelStyle::y_default(),
        });
        self.layout_selected = Some(self.layout_panels.len() - 1);
        self.layout_open = true;
        self.set_status("Added panel to layout");
    }

    /// Add the detached population window's current view to the layout canvas.
    fn add_layout_popout(&mut self) {
        let (Some(s), Some(gate_idx), Some(x), Some(y)) =
            (self.current_sample, self.popout.gate, self.popout.x_param, self.popout.y_param)
        else {
            self.set_status("Open a detached population window first");
            return;
        };
        let gate_name = self.gates[gate_idx].name.clone();
        let x_name = param_display_name(&self.samples[s].fcs, x);
        let y_name = param_display_name(&self.samples[s].fcs, y);
        let title = format!("Population: {gate_name} ({x_name} vs {y_name})");
        let preview = self.make_popout_preview(s, gate_idx, x, y);
        let n = self.layout_panels.len();
        let cols = 3usize;
        let col = (n % cols) as f32;
        let row = (n / cols) as f32;
        let rect = egui::Rect::from_min_size(
            egui::pos2(20.0 + col * 300.0, 20.0 + row * 240.0),
            egui::vec2(280.0, 220.0),
        );
        self.layout_panels.push(LayoutPanelSpec {
            sample: s,
            x,
            y,
            view: self.popout.view,
            gate: Some(gate_idx),
            title,
            rect,
            preview,
            x_label: LabelStyle::x_default(),
            y_label: LabelStyle::y_default(),
        });
        self.layout_selected = Some(self.layout_panels.len() - 1);
        self.layout_open = true;
        self.set_status("Added population panel to layout");
    }

    /// Compute normalised (0..1) preview points for a sample's parameter pair.
    fn make_layout_preview(&self, sample: usize, x: usize, y: usize) -> Vec<[f32; 2]> {
        self.make_preview_points(sample, None, x, y)
    }

    /// Compute normalised (0..1) preview points for a gated population.
    fn make_popout_preview(&self, sample: usize, gate_idx: usize, x: usize, y: usize) -> Vec<[f32; 2]> {
        self.make_preview_points(sample, Some(gate_idx), x, y)
    }

    /// Shared helper: normalised preview points for all events or a population.
    fn make_preview_points(
        &self,
        sample: usize,
        gate: Option<usize>,
        x: usize,
        y: usize,
    ) -> Vec<[f32; 2]> {
        let Some(s) = self.samples.get(sample) else {
            return Vec::new();
        };
        let fcs = &s.fcs;
        let (n, n_params) = fcs.data.dim();
        if x >= n_params || y >= n_params {
            return Vec::new();
        }

        // Collect the (x, y) values for all events or the gated subset.
        let pts: Vec<[f64; 2]> = match gate {
            Some(g) => {
                let pops = compute_gate_populations(&fcs.data, &self.gates, Some(sample), s.apply_gates);
                let idxs = pops.get(g).cloned().unwrap_or_default();
                idxs.iter()
                    .map(|&e| [fcs.data[[e, x]] as f64, fcs.data[[e, y]] as f64])
                    .collect()
            }
            None => (0..n)
                .filter_map(|i| {
                    let px = fcs.data[[i, x]] as f64;
                    let py = fcs.data[[i, y]] as f64;
                    if px.is_finite() && py.is_finite() {
                        Some([px, py])
                    } else {
                        None
                    }
                })
                .collect(),
        };

        if pts.is_empty() {
            return Vec::new();
        }

        let stride = (pts.len() / 1500).max(1);
        let (mut xmin, mut xmax) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut out: Vec<[f32; 2]> = Vec::new();
        for (i, p) in pts.iter().enumerate() {
            if i % stride != 0 {
                continue;
            }
            xmin = xmin.min(p[0]);
            xmax = xmax.max(p[0]);
            ymin = ymin.min(p[1]);
            ymax = ymax.max(p[1]);
            out.push([p[0] as f32, p[1] as f32]);
        }
        let xspan = (xmax - xmin).max(f64::EPSILON);
        let yspan = (ymax - ymin).max(f64::EPSILON);
        out.into_iter()
            .map(|p| [
                ((p[0] as f64 - xmin) / xspan) as f32,
                ((p[1] as f64 - ymin) / yspan) as f32,
            ])
            .collect()
    }

    /// Arrange all layout panels in a tidy grid.
    fn tile_layout(&mut self) {
        let cols = self.layout_columns.max(1);
        let (w, h, pad) = (280.0f32, 220.0f32, 20.0f32);
        for (i, p) in self.layout_panels.iter_mut().enumerate() {
            let col = (i % cols) as f32;
            let row = (i / cols) as f32;
            p.rect = egui::Rect::from_min_size(
                egui::pos2(pad + col * (w + pad), pad + row * (h + pad)),
                egui::vec2(w, h),
            );
        }
    }

    /// The interactive layout canvas (drag to move, corner to resize).
    fn layout_canvas(&mut self, ui: &mut egui::Ui) {
        let canvas_size = egui::vec2(920.0, 560.0);
        let (response, painter) =
            ui.allocate_painter(canvas_size, egui::Sense::click_and_drag());
        let origin = response.rect.min;
        let bg = if self.theme == Theme::Dark {
            egui::Color32::from_gray(45)
        } else {
            egui::Color32::from_gray(225)
        };
        painter.rect_filled(response.rect, 4.0_f32, bg);

        let pointer = ui.input(|i| i.pointer.interact_pos());
        let down = ui.input(|i| i.pointer.primary_down());

        // Begin / end a manipulation.
        if !down {
            self.layout_drag = None;
        }
        if down && self.layout_drag.is_none() {
            if let Some(p) = pointer {
                // Only clear/select from canvas clicks; ignore clicks on
                // toolbar buttons (which lie outside the canvas rect).
                if response.rect.contains(p) {
                    let mut hit: Option<(usize, bool)> = None;
                    for (i, panel) in self.layout_panels.iter().enumerate().rev() {
                        let r = panel.rect.translate(origin.to_vec2());
                        let handle = egui::Rect::from_min_size(
                            r.max - egui::vec2(14.0, 14.0),
                            egui::vec2(14.0, 14.0),
                        );
                        if handle.contains(p) {
                            hit = Some((i, true));
                            break;
                        }
                        if r.contains(p) {
                            hit = Some((i, false));
                            break;
                        }
                    }
                    match hit {
                        Some((i, resize)) => {
                            self.layout_selected = Some(i);
                            let r = self.layout_panels[i].rect.translate(origin.to_vec2());
                            let grab = if resize {
                                egui::Vec2::ZERO
                            } else {
                                p - r.min
                            };
                            self.layout_drag = Some(LayoutDrag {
                                panel: i,
                                resize,
                                grab,
                            });
                        }
                        None => self.layout_selected = None,
                    }
                }
            }
        }
        // Apply the manipulation.
        if let Some(drag) = self.layout_drag {
            if let Some(p) = pointer {
                let local = egui::pos2(p.x - origin.x, p.y - origin.y);
                if let Some(panel) = self.layout_panels.get_mut(drag.panel) {
                    if drag.resize {
                        let size = (local - panel.rect.min).max(egui::vec2(80.0, 60.0));
                        panel.rect = egui::Rect::from_min_max(panel.rect.min, panel.rect.min + size);
                    } else {
                        let new_min = local - drag.grab;
                        panel.rect = egui::Rect::from_min_size(new_min, panel.rect.size());
                    }
                }
            }
        }

        // Draw panels back-to-front.
        for (i, panel) in self.layout_panels.iter().enumerate() {
            let r = panel.rect.translate(origin.to_vec2());
            let selected = self.layout_selected == Some(i);
            painter.rect_filled(r, 2.0_f32, egui::Color32::WHITE);

            // Title.
            painter.text(
                r.min + egui::vec2(r.width() / 2.0, -6.0),
                egui::Align2::CENTER_BOTTOM,
                &panel.title,
                egui::FontId::proportional(12.0),
                egui::Color32::from_gray(60),
            );

            // Preview points.
            let inner = r.shrink(8.0);
            let dot = egui::Color32::from_rgba_unmultiplied(
                self.point_color.r(),
                self.point_color.g(),
                self.point_color.b(),
                180,
            );
            for pt in &panel.preview {
                let x = inner.min.x + pt[0].clamp(0.0, 1.0) * inner.width();
                let y = inner.max.y - pt[1].clamp(0.0, 1.0) * inner.height();
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(1.5, 1.5)),
                    0.0_f32,
                    dot,
                );
            }

            // Axis labels (using the per-panel label style).
            let fcs = &self.samples[panel.sample].fcs;
            let x_text = panel
                .x_label
                .text
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| param_display_name(fcs, panel.x));
            let y_text = panel
                .y_label
                .text
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| {
                    if matches!(panel.view, ViewMode::Histogram) {
                        "Count".to_string()
                    } else {
                        param_display_name(fcs, panel.y)
                    }
                });
            // Panels are authored at 500x450, so scale label text to the panel.
            let lscale = (r.width() / 500.0).clamp(0.25, 4.0);
            paint_label(
                &painter,
                r.min + egui::vec2(r.width() / 2.0, r.height() - 5.0),
                &x_text,
                panel.x_label.size * lscale,
                panel.x_label.color,
                panel.x_label.font.egui(),
                panel.x_label.rotation_deg.to_radians(),
            );
            paint_label(
                &painter,
                r.min + egui::vec2(6.0, r.height() / 2.0),
                &y_text,
                panel.y_label.size * lscale,
                panel.y_label.color,
                panel.y_label.font.egui(),
                panel.y_label.rotation_deg.to_radians(),
            );

            let stroke = if selected {
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(255, 140, 0))
            } else {
                egui::Stroke::new(1.0_f32, egui::Color32::from_gray(140))
            };
            painter.rect_stroke(r, 2.0_f32, stroke);

            if selected {
                let handle = egui::Rect::from_min_size(
                    r.max - egui::vec2(14.0, 14.0),
                    egui::vec2(14.0, 14.0),
                );
                painter.rect_filled(handle, 2.0_f32, egui::Color32::from_rgb(255, 140, 0));
            }
        }
    }

    /// The floating layout-editor window.
    fn layout_window(&mut self, ctx: &egui::Context) {
        if !self.layout_open {
            return;
        }
        let mut open = self.layout_open;
        egui::Window::new("Layout Editor")
            .open(&mut open)
            .resizable(true)
            .default_size([980.0, 700.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Add current plot").clicked() {
                        self.add_layout_current();
                    }
                    let popout_open = self.popout.open;
                    if ui
                        .add_enabled(popout_open, egui::Button::new("Add detached plot"))
                        .clicked()
                    {
                        self.add_layout_popout();
                    }
                    if ui.button("Tile").clicked() {
                        self.tile_layout();
                    }
                    ui.label("Columns:");
                    ui.add(egui::DragValue::new(&mut self.layout_columns).range(1..=4));
                    if ui.button("Bring to front").clicked() {
                        if let Some(i) = self.layout_selected {
                            if i < self.layout_panels.len() {
                                let p = self.layout_panels.remove(i);
                                self.layout_panels.push(p);
                                self.layout_selected = Some(self.layout_panels.len() - 1);
                            }
                        }
                    }
                    if ui.button("Delete").clicked() {
                        if let Some(i) = self.layout_selected {
                            if i < self.layout_panels.len() {
                                self.layout_panels.remove(i);
                            }
                            self.layout_selected = None;
                        }
                    }
                    if ui.button("Clear").clicked() {
                        self.layout_panels.clear();
                        self.layout_selected = None;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Export:");
                    if ui.button("SVG…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("SVG", &["svg"])
                            .set_file_name("layout.svg")
                            .save_file()
                        {
                            self.export_layout(&path);
                        }
                    }
                    if ui.button("Image…").clicked() {
                        let mut dlg = rfd::FileDialog::new().set_file_name("layout.png");
                        for &(name, exts) in ImageFormat::dialog_filters() {
                            dlg = dlg.add_filter(name, exts);
                        }
                        if let Some(path) = dlg.save_file() {
                            self.export_layout_image(&path);
                        }
                    }
                    ui.label("scale:");
                    ui.add(
                        egui::DragValue::new(&mut self.export_scale)
                            .range(1.0..=4.0)
                            .speed(0.1),
                    );
                    ui.label("Tip: drag a panel to move it; drag its corner handle to resize.");
                });
                ui.separator();

                egui::ScrollArea::both().show(ui, |ui| {
                    self.layout_canvas(ui);
                });

                // Per-axis label styling for the selected panel.
                if let Some(i) = self.layout_selected {
                    if let Some(panel) = self.layout_panels.get_mut(i) {
                        ui.separator();
                        ui.strong("Axis label style (selected panel)");
                        ui.horizontal(|ui| {
                            ui.label("X text:");
                            label_text_edit(ui, "lx_text", &mut panel.x_label);
                            ui.label("size:");
                            ui.add(
                                egui::DragValue::new(&mut panel.x_label.size)
                                    .range(4.0..=72.0)
                                    .speed(0.5),
                            );
                            ui.label("rotate:");
                            ui.add(
                                egui::DragValue::new(&mut panel.x_label.rotation_deg)
                                    .range(-180.0..=180.0)
                                    .suffix("°"),
                            );
                            label_font_combo(ui, "lx_font", &mut panel.x_label.font);
                            ui.color_edit_button_srgba(&mut panel.x_label.color);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Y text:");
                            label_text_edit(ui, "ly_text", &mut panel.y_label);
                            ui.label("size:");
                            ui.add(
                                egui::DragValue::new(&mut panel.y_label.size)
                                    .range(4.0..=72.0)
                                    .speed(0.5),
                            );
                            ui.label("rotate:");
                            ui.add(
                                egui::DragValue::new(&mut panel.y_label.rotation_deg)
                                    .range(-180.0..=180.0)
                                    .suffix("°"),
                            );
                            label_font_combo(ui, "ly_font", &mut panel.y_label.font);
                            ui.color_edit_button_srgba(&mut panel.y_label.color);
                        });
                        ui.weak(
                            "Leave a text box blank to use the parameter name. The (linear)/(log) suffix is kept.",
                        );
                    }
                }

                // Per-panel quick list (select / remove).
                ui.separator();
                let mut remove: Option<usize> = None;
                for (i, panel) in self.layout_panels.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let name = self
                            .samples
                            .get(panel.sample)
                            .map(|s| s.id.clone())
                            .unwrap_or_else(|| "?".into());
                        let label = format!(
                            "{}. {} — {} vs {} ({})",
                            i + 1,
                            name,
                            param_display_name(&self.samples[panel.sample].fcs, panel.x),
                            param_display_name(&self.samples[panel.sample].fcs, panel.y),
                            panel.view.label()
                        );
                        if ui
                            .selectable_label(self.layout_selected == Some(i), label)
                            .clicked()
                        {
                            self.layout_selected = Some(i);
                        }
                        if ui.small_button("✖").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    self.layout_panels.remove(i);
                    if self.layout_selected == Some(i) {
                        self.layout_selected = None;
                    }
                }
            });
        self.layout_open = open;
    }
}

/// The pixel size of the layout canvas implied by the placed panels.
fn layout_canvas_size(panels: &[LayoutPanelSpec]) -> (u32, u32) {
    let mut max_x = 100.0f32;
    let mut max_y = 100.0f32;
    for p in panels {
        max_x = max_x.max(p.rect.max.x);
        max_y = max_y.max(p.rect.max.y);
    }
    ((max_x + 20.0) as u32, (max_y + 20.0) as u32)
}

fn scale_label(s: AxisScale) -> &'static str {    match s {
        AxisScale::Linear => "linear",
        AxisScale::Log => "log10",
    }
}

fn quad_label(q: &Quadrant) -> &'static str {
    match q {
        Quadrant::UpperLeft => "UL",
        Quadrant::UpperRight => "UR",
        Quadrant::LowerLeft => "LL",
        Quadrant::LowerRight => "LR",
    }
}

/// Actions produced while drawing the recursive gate tree.
enum TreeAction {
    Select(usize),
    Remove(usize),
    Popout(usize),
}

/// Renders the hierarchical gate tree, returning actions for the caller.
fn gate_tree_ui(
    ui: &mut egui::Ui,
    gates: &[GateDef],
    children: &[Vec<usize>],
    counts: &[usize],
    total: usize,
    selected: Option<usize>,
) -> Vec<TreeAction> {
    let mut actions = Vec::new();
    let roots: Vec<usize> = (0..gates.len())
        .filter(|i| gates[*i].parent.is_none())
        .collect();

    fn rec(
        node: usize,
        depth: usize,
        ui: &mut egui::Ui,
        gates: &[GateDef],
        children: &[Vec<usize>],
        counts: &[usize],
        total: usize,
        selected: Option<usize>,
        actions: &mut Vec<TreeAction>,
    ) {
        let gate = &gates[node];
        let parent_count = gate
            .parent
            .and_then(|p| counts.get(p).copied())
            .unwrap_or(total);
        let count = counts.get(node).copied().unwrap_or(0);
        let pct = if parent_count == 0 {
            0.0
        } else {
            count as f64 / parent_count as f64 * 100.0
        };

        ui.horizontal(|ui| {
            ui.add_space((depth as f32) * 12.0);
            let swatch = egui::Frame::none()
                .fill(gate.color)
                .rounding(2.0)
                .inner_margin(egui::Margin::same(3.0));
            swatch.show(ui, |ui| {
                ui.label(" ");
            });
            let label = format!("{}  [{}]  {} ({:.1}%)", gate.name, gate.kind.label(), count, pct);
            if ui
                .selectable_label(selected == Some(node), label)
                .clicked()
            {
                actions.push(TreeAction::Select(node));
            }
            if ui.small_button("◧").on_hover_text("Show in detached window").clicked() {
                actions.push(TreeAction::Popout(node));
            }
            if ui.small_button("✖").on_hover_text("Delete gate").clicked() {
                actions.push(TreeAction::Remove(node));
            }
        });

        for &child in &children[node] {
            rec(
                child,
                depth + 1,
                ui,
                gates,
                children,
                counts,
                total,
                selected,
                actions,
            );
        }
    }

    if roots.is_empty() {
        ui.weak("No gates yet — draw one on the plot.");
    }
    for root in roots {
        rec(
            root,
            0,
            ui,
            gates,
            children,
            counts,
            total,
            selected,
            &mut actions,
        );
    }
    actions
}

impl eframe::App for FlowersApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply the selected theme before drawing anything this frame.
        ctx.set_visuals(match self.theme {
            Theme::Dark => egui::Visuals::dark(),
            Theme::Light => egui::Visuals::light(),
        });

        // ----- Undo / redo keyboard shortcuts -----
        // Skipped while a text field has focus so the widget keeps its own undo.
        if !ctx.wants_keyboard_input() {
            let cmd = egui::Modifiers::COMMAND;
            // Most specific shortcut first: extra Shift is ignored by matching.
            let redo_shift = ctx
                .input_mut(|i| i.consume_key(cmd | egui::Modifiers::SHIFT, egui::Key::Z));
            let redo_y = ctx.input_mut(|i| i.consume_key(cmd, egui::Key::Y));
            let undo = ctx.input_mut(|i| i.consume_key(cmd, egui::Key::Z));
            if redo_shift || redo_y {
                self.redo();
            } else if undo {
                self.undo();
            }
        }

        // ----- Top menu -----
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                // File dropdown.
                ui.menu_button("File", |ui| {
                    if ui.button("Load FCS").clicked() {
                        ui.close_menu();
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            self.load_file(&path);
                        }
                    }
                    ui.separator();
                    if ui.button("New Workspace").clicked() {
                        ui.close_menu();
                        self.new_workspace();
                    }
                    if ui.button("Load Workspace…").clicked() {
                        ui.close_menu();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Flowers workspace", &["wsp", "json"])
                            .pick_file()
                        {
                            self.load_workspace(&path);
                        }
                    }
                    if ui.button("Save Workspace…").clicked() {
                        ui.close_menu();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Flowers workspace", &["wsp", "json"])
                            .set_file_name("workspace.wsp")
                            .save_file()
                        {
                            self.save_workspace(&path);
                        }
                    }
                    ui.separator();
                    if ui.button("Export SVG").clicked() {
                        ui.close_menu();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("SVG", &["svg"])
                            .set_file_name("plot.svg")
                            .save_file()
                        {
                            self.export_svg(&path);
                        }
                    }
                    if ui.button("Export Image…").clicked() {
                        ui.close_menu();
                        let mut dlg = rfd::FileDialog::new().set_file_name("plot.png");
                        for &(name, exts) in ImageFormat::dialog_filters() {
                            dlg = dlg.add_filter(name, exts);
                        }
                        if let Some(path) = dlg.save_file() {
                            self.export_image(&path);
                        }
                    }
                    if ui.button("Export CSV").clicked() {
                        ui.close_menu();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("CSV", &["csv"])
                            .set_file_name("populations.csv")
                            .save_file()
                        {
                            self.export_csv(&path);
                        }
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.close_menu();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                // Edit dropdown (undo / redo).
                ui.menu_button("Edit", |ui| {
                    let undo_shortcut = egui::KeyboardShortcut::new(
                        egui::Modifiers::COMMAND,
                        egui::Key::Z,
                    );
                    let redo_shortcut = egui::KeyboardShortcut::new(
                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                        egui::Key::Z,
                    );
                    if ui
                        .add_enabled(
                            self.can_undo(),
                            egui::Button::new("Undo").shortcut_text(
                                ui.ctx().format_shortcut(&undo_shortcut),
                            ),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.undo();
                    }
                    if ui
                        .add_enabled(
                            self.can_redo(),
                            egui::Button::new("Redo").shortcut_text(
                                ui.ctx().format_shortcut(&redo_shortcut),
                            ),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.redo();
                    }
                });

                // Layout Editor (standalone button).
                if ui.button("Layout Editor").clicked() {
                    self.layout_open = true;
                }

                // Settings dropdown.
                ui.menu_button("Settings", |ui| {
                    ui.label("Theme:");
                    for t in [Theme::Dark, Theme::Light] {
                        if ui
                            .selectable_label(self.theme == t, t.label())
                            .clicked()
                        {
                            ui.close_menu();
                            self.theme = t;
                        }
                    }
                });

                ui.separator();
                ui.label("flowers — FlowJo alternative");
            });
        });

        // ----- Left sidebar -----
        egui::SidePanel::left("sidebar")
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("flowers");
                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Load FCS File").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            self.load_file(&path);
                        }
                    }
                });

                ui.separator();
                ui.label("Loaded samples:");
                let mut clicked_sample: Option<usize> = None;
                let mut toggled_gates: Option<(usize, bool)> = None;
                for (i, sample) in self.samples.iter().enumerate() {
                    let selected = self.current_sample == Some(i);
                    let id = sample.id.clone();
                    let mut apply = sample.apply_gates;
                    ui.horizontal(|ui| {
                        if ui.selectable_label(selected, &id).clicked() {
                            clicked_sample = Some(i);
                        }
                        if ui
                            .checkbox(&mut apply, "gates")
                            .on_hover_text("Apply the shared gate set to this file")
                            .changed()
                        {
                            toggled_gates = Some((i, apply));
                        }
                    });
                }
                if let Some(i) = clicked_sample {
                    self.current_sample = Some(i);
                    self.update_plot_points();
                }
                if let Some((i, apply)) = toggled_gates {
                    if let Some(s) = self.samples.get_mut(i) {
                        s.apply_gates = apply;
                    }
                    self.recompute_populations();
                }
                if self.samples.is_empty() {
                    ui.weak("(none)");
                }

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.separator();
                    ui.label("Plot parameters:");
                    let Some(fcs) = self.current_fcs() else {
                        ui.weak("No sample selected");
                        return;
                    };
                    let names: Vec<String> = (0..fcs.parameters.len())
                        .map(|i| param_display_name(fcs, i))
                        .collect();
                    let events = fcs.event_count;

                    egui::ComboBox::new("x_param", "X")
                        .selected_text(
                            self.x_param
                                .map(|i| names[i].clone())
                                .unwrap_or_else(|| "Select X".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            for (i, name) in names.iter().enumerate() {
                                if ui
                                    .selectable_value(&mut self.x_param, Some(i), name.clone())
                                    .changed()
                                {
                                    self.update_plot_points();
                                }
                            }
                        });
                    egui::ComboBox::new("y_param", "Y")
                        .selected_text(
                            self.y_param
                                .map(|i| names[i].clone())
                                .unwrap_or_else(|| "Select Y".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            for (i, name) in names.iter().enumerate() {
                                if ui
                                    .selectable_value(&mut self.y_param, Some(i), name.clone())
                                    .changed()
                                {
                                    self.update_plot_points();
                                }
                            }
                        });

                    if self.x_param.is_some() && self.y_param.is_some() {
                        ui.label(format!("Events: {events}"));
                    }

                    ui.separator();
                    ui.collapsing("Axis options", |ui| {
                        // X axis.
                        ui.horizontal(|ui| {
                            ui.label("X scale:");
                            for scale in [AxisScale::Linear, AxisScale::Log] {
                                if ui
                                    .selectable_label(self.x_scale == scale, scale.label())
                                    .clicked()
                                {
                                    self.x_scale = scale;
                                    self.auto_x();
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("X range:");
                            let span = (self.x_range.1 - self.x_range.0).abs().max(1.0);
                            ui.add(range_drag_value(&mut self.x_range.0, span));
                            ui.add(range_drag_value(&mut self.x_range.1, span));
                            if ui.small_button("Auto").on_hover_text("Fit X to data").clicked() {
                                self.auto_x();
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Y scale:");
                            for scale in [AxisScale::Linear, AxisScale::Log] {
                                if ui
                                    .selectable_label(self.y_scale == scale, scale.label())
                                    .clicked()
                                {
                                    self.y_scale = scale;
                                    self.auto_y();
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Y range:");
                            let span = (self.y_range.1 - self.y_range.0).abs().max(1.0);
                            ui.add(range_drag_value(&mut self.y_range.0, span));
                            ui.add(range_drag_value(&mut self.y_range.1, span));
                            if ui.small_button("Auto").on_hover_text("Fit Y to data").clicked() {
                                self.auto_y();
                            }
                        });
                    });

                    ui.collapsing("Point & view options", |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Dot colour:");
                            ui.color_edit_button_srgba(&mut self.point_color);
                        });
                        ui.checkbox(&mut self.gradient, "Density gradient layering");
                        ui.horizontal(|ui| {
                            ui.label("View:");
                            for mode in [ViewMode::Dots, ViewMode::Contour, ViewMode::Histogram] {
                                if ui
                                    .selectable_label(self.view_mode == mode, mode.label())
                                    .clicked()
                                {
                                    self.view_mode = mode;
                                }
                            }
                        });
                        if matches!(self.view_mode, ViewMode::Contour) {
                            ui.add(
                                egui::Slider::new(&mut self.contour_levels, 3..=20)
                                    .text("contour levels"),
                            );
                        }
                    });

                    ui.separator();
                    ui.label("New gate type:");
                    for gt in NewGateType::ALL {
                        ui.radio_value(&mut self.new_gate_type, gt, gt.label());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("+ Add Gate").clicked() {
                            self.add_default_gate();
                        }
                        if ui.button("Remove selected").clicked() {
                            if let Some(i) = self.selected_gate {
                                self.remove_gate(i);
                            }
                        }
                    });
                    ui.label("Tip: drag on the plot to draw; click places a quadrant/bilinear gate.");
                });
            });

        // ----- Central panel -----
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.current_fcs().is_none() {
                ui.centered_and_justified(|ui| ui.label("Load an FCS file to begin"));
                return;
            }
            if self.x_param.is_none() || self.y_param.is_none() {
                ui.centered_and_justified(|ui| ui.label("Select X and Y parameters to plot"));
                return;
            }
            if self.plot_points.is_empty() {
                self.update_plot_points();
            }

            let cfg = self.main_plot_cfg();
            egui::ScrollArea::both().show(ui, |ui| {
                let mut inter = PlotInteraction {
                    drag_start: self.interact.drag_start,
                    drag_current: self.interact.drag_current,
                    ..Default::default()
                };
                show_plot(ui, "scatter_plot", &cfg, &mut inter);

                let tol = 0.02
                    * (self.x_range.1 - self.x_range.0)
                        .abs()
                        .max((self.y_range.1 - self.y_range.0).abs())
                        .max(1.0);
                let outcome = drive_interact(
                    &mut self.interact,
                    &inter,
                    &self.gates,
                    self.selected_gate,
                    self.new_gate_type,
                    tol,
                );
                if let Some(outcome) = outcome {
                    match outcome {
                        InteractOutcome::Create { start, end } => {
                            self.create_gate_from_drag(start, end)
                        }
                        InteractOutcome::Update { gate, kind } => {
                            if let Some(g) = self.gates.get_mut(gate) {
                                g.kind = kind;
                            }
                            self.recompute_populations();
                        }
                        InteractOutcome::Click(p) => {
                            self.create_click_gate(p, false)
                        }
                    }
                }
            });
        });

        // ----- Bottom panel: gate editor -----
        egui::TopBottomPanel::bottom("gate_editor")
            .resizable(true)
            .default_height(180.0)
            .show(ctx, |ui| {
                let sel = self.selected_gate;
                ui.horizontal(|ui| {
                    ui.label("Selected gate:");
                    if let Some(i) = sel {
                        ui.text_edit_singleline(&mut self.name_edit);
                        if ui.button("Rename").clicked() {
                            self.gates[i].name = self.name_edit.clone();
                        }
                        let mut color = self.gates[i].color;
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            self.gates[i].color = color;
                        }
                        ui.separator();
                        // Parent selector: only earlier gates are offered, which
                        // keeps the hierarchy acyclic and topologically ordered.
                        let current = self.gates[i].parent;
                        let current_label = current
                            .map(|p| self.gates[p].name.clone())
                            .unwrap_or_else(|| "(root)".to_string());
                        let options: Vec<(usize, String)> =
                            (0..i).map(|p| (p, self.gates[p].name.clone())).collect();
                        let mut new_parent = current;
                        egui::ComboBox::new("parent_sel", "Parent")
                            .selected_text(current_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut new_parent, None, "(root)");
                                for (p, name) in &options {
                                    ui.selectable_value(&mut new_parent, Some(*p), name.clone());
                                }
                            });
                        if new_parent != current {
                            self.gates[i].parent = new_parent;
                            self.recompute_populations();
                        }
                        // Apply-to-all toggle.
                        ui.horizontal(|ui| {
                            ui.label("Apply to all samples:");
                            ui.checkbox(&mut self.gates[i].apply_to_all, "")
                                .on_hover_text(
                                    "Off: this gate applies only to its owner sample. On: it also applies to every other sample that has gating enabled (the \"gates\" checkbox in the sample list).",
                                );
                            if let Some(owner_idx) = self.gates[i].owner {
                                ui.label(format!(
                                    "(created on sample {})",
                                    self.samples[owner_idx].id
                                ));
                            } else {
                                ui.label("(created before any sample loaded)");
                            }
                        });
                    } else {
                        ui.label("none");
                    }
                });

                let Some(i) = self.selected_gate else {
                    ui.label("No gate selected.");
                    return;
                };
                if i >= self.gates.len() {
                    return;
                }

                // Numeric editor for the selected gate.
                let mut new_kind = self.gates[i].kind.clone();
                let mut changed = false;
                match &mut new_kind {
                    GateKind::Rect {
                        x_min,
                        x_max,
                        y_min,
                        y_max,
                        angle,
                    }
                    | GateKind::Ellipse {
                        x_min,
                        x_max,
                        y_min,
                        y_max,
                        angle,
                    } => {
                        ui.horizontal(|ui| {
                            ui.label("X min:");
                            changed |= ui.add(egui::DragValue::new(x_min).speed(1.0)).changed();
                            ui.label("X max:");
                            changed |= ui.add(egui::DragValue::new(x_max).speed(1.0)).changed();
                            ui.label("Y min:");
                            changed |= ui.add(egui::DragValue::new(y_min).speed(1.0)).changed();
                            ui.label("Y max:");
                            changed |= ui.add(egui::DragValue::new(y_max).speed(1.0)).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("Rotation (deg):");
                            let mut deg = angle.to_degrees();
                            if ui.add(egui::DragValue::new(&mut deg).speed(1.0)).changed() {
                                *angle = deg.to_radians();
                                changed = true;
                            }
                        });
                    }
                    GateKind::Polygon { vertices } => {
                        ui.label(format!(
                            "{} vertices — drag a vertex handle on the plot to reshape.",
                            vertices.len()
                        ));
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "Centroid: ({:.1}, {:.1})",
                                centroid(vertices)[0],
                                centroid(vertices)[1]
                            ));
                        });
                        ui.horizontal(|ui| {
                            if ui.button("+ Add vertex").clicked() {
                                // Insert a new vertex at the midpoint of the longest edge.
                                if vertices.len() >= 2 {
                                    let mut best_edge = 0usize;
                                    let mut best_len = f64::NEG_INFINITY;
                                    for j in 0..vertices.len() {
                                        let a = vertices[j];
                                        let b = vertices[(j + 1) % vertices.len()];
                                        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                                        if len > best_len {
                                            best_len = len;
                                            best_edge = j;
                                        }
                                    }
                                    let a = vertices[best_edge];
                                    let b = vertices[(best_edge + 1) % vertices.len()];
                                    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                                    vertices.insert(best_edge + 1, mid);
                                    changed = true;
                                }
                            }
                            if ui.button("− Remove vertex").clicked() {
                                if vertices.len() > 3 {
                                    vertices.pop();
                                    changed = true;
                                }
                            }
                            ui.label("Minimum 3 vertices.");
                        });
                    }
                    GateKind::Quadrant { x, y, .. } => {
                        ui.horizontal(|ui| {
                            ui.label("Split X:");
                            changed |= ui.add(egui::DragValue::new(x).speed(1.0)).changed();
                            ui.label("Split Y:");
                            changed |= ui.add(egui::DragValue::new(y).speed(1.0)).changed();
                        });
                        ui.label("Tip: all four quadrant gates share the same split point when drawn together.");
                    }
                    GateKind::Bilinear {
                        x1,
                        y1,
                        x2,
                        y2,
                        below,
                    } => {
                        ui.horizontal(|ui| {
                            ui.label("Start X:");
                            changed |= ui.add(egui::DragValue::new(x1).speed(1.0)).changed();
                            ui.label("Start Y:");
                            changed |= ui.add(egui::DragValue::new(y1).speed(1.0)).changed();
                            ui.label("End X:");
                            changed |= ui.add(egui::DragValue::new(x2).speed(1.0)).changed();
                            ui.label("End Y:");
                            changed |= ui.add(egui::DragValue::new(y2).speed(1.0)).changed();
                        });
                        ui.horizontal(|ui| {
                            changed |= ui
                                .checkbox(below, "Select points below the line")
                                .changed();
                            ui.weak("(uncheck to select the region above)");
                        });
                    }
                }
                if changed {
                    self.gates[i].kind = new_kind;
                    self.recompute_populations();
                }

                // Non-interactive transform buttons (a reliable fallback to the
                // drag handles drawn on the plot).
                let step = (self.x_range.1 - self.x_range.0).abs().max(1.0) * 0.02;
                ui.horizontal(|ui| {
                    ui.label("Transform:");
                    let mut t = false;
                    if ui.small_button("⬅").on_hover_text("Move left").clicked() {
                        self.gates[i].kind.translate(-step, 0.0);
                        t = true;
                    }
                    if ui.small_button("➡").on_hover_text("Move right").clicked() {
                        self.gates[i].kind.translate(step, 0.0);
                        t = true;
                    }
                    if ui.small_button("⬆").on_hover_text("Move up").clicked() {
                        self.gates[i].kind.translate(0.0, step);
                        t = true;
                    }
                    if ui.small_button("⬇").on_hover_text("Move down").clicked() {
                        self.gates[i].kind.translate(0.0, -step);
                        t = true;
                    }
                    if ui.small_button("⟲").on_hover_text("Rotate -5°").clicked() {
                        let c = self.gates[i].kind.center();
                        self.gates[i].kind.rotate_about(c, (-5.0f64).to_radians());
                        t = true;
                    }
                    if ui.small_button("⟳").on_hover_text("Rotate +5°").clicked() {
                        let c = self.gates[i].kind.center();
                        self.gates[i].kind.rotate_about(c, (5.0f64).to_radians());
                        t = true;
                    }
                    if ui.small_button("➕").on_hover_text("Enlarge").clicked() {
                        let c = self.gates[i].kind.center();
                        self.gates[i].kind.scale_about(c, 1.1);
                        t = true;
                    }
                    if ui.small_button("➖").on_hover_text("Shrink").clicked() {
                        let c = self.gates[i].kind.center();
                        self.gates[i].kind.scale_about(c, 0.9);
                        t = true;
                    }
                    if t {
                        self.recompute_populations();
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "Events in gate: {}",
                        self.pop_indices.get(i).map(|v| v.len()).unwrap_or(0)
                    ));
                    if ui.button("Open detached window").clicked() {
                        self.open_popout(i);
                    }
                });
            });

        // ----- Right panel: hierarchy, cross-sample summary, layout editor -----
        egui::SidePanel::right("populations")
            .default_width(330.0)
            .show(ctx, |ui| {
                ui.heading("Gate tree");
                ui.separator();
                let total = self.current_fcs().map(|f| f.event_count).unwrap_or(0);
                let counts: Vec<usize> = self.pop_indices.iter().map(|v| v.len()).collect();
                let actions = gate_tree_ui(
                    ui,
                    &self.gates,
                    &self.children,
                    &counts,
                    total,
                    self.selected_gate,
                );
                for action in actions {
                    match action {
                        TreeAction::Select(i) => {
                            self.selected_gate = Some(i);
                            self.sync_name_edit();
                        }
                        TreeAction::Remove(i) => self.remove_gate(i),
                        TreeAction::Popout(i) => {
                            self.open_popout(i);
                        }
                    }
                }

                ui.separator();
                ui.collapsing("Populations across samples", |ui| {
                    if self.samples.is_empty() || self.gates.is_empty() {
                        ui.weak("Load samples and create gates.");
                    } else {
                        egui::Grid::new("cross_sample").striped(true).show(ui, |ui| {
                            ui.strong("Sample");
                            ui.strong("Pop");
                            ui.strong("Count");
                            ui.strong("%");
                            ui.end_row();
                            for (sample_idx, sample) in self.samples.iter().enumerate() {
                                let pops = compute_gate_populations(
                                    &sample.fcs.data,
                                    &self.gates,
                                    Some(sample_idx),
                                    sample.apply_gates,
                                );
                                for (i, gate) in self.gates.iter().enumerate() {
                                    let c = pops.get(i).map(|v| v.len()).unwrap_or(0);
                                    let pct = if sample.fcs.event_count == 0 {
                                        0.0
                                    } else {
                                        c as f64 / sample.fcs.event_count as f64 * 100.0
                                    };
                                    ui.label(&sample.id);
                                    ui.label(&gate.name);
                                    ui.label(format!("{c}"));
                                    ui.label(format!("{pct:.1}%"));
                                    ui.end_row();
                                }
                            }
                        });
                    }
                    if ui.button("Export CSV…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("CSV", &["csv"])
                            .set_file_name("populations.csv")
                            .save_file()
                        {
                            self.export_csv(&path);
                        }
                    }
                });
            });

        // ----- Layout editor window -----
        self.layout_window(ctx);

        // ----- Detached population window -----
        if self.popout.open {
            let mut open = self.popout.open;
            let gate_idx = self.popout.gate;
            let title = gate_idx
                .and_then(|i| self.gates.get(i))
                .map(|g| format!("Population: {}", g.name))
                .unwrap_or_else(|| "Population".to_string());
            egui::Window::new(title)
                .open(&mut open)
                .resizable(true)
                .default_size([760.0, 600.0])
                .show(ctx, |ui| {
                    let Some(i) = gate_idx else {
                        ui.label("Select a gate in the tree.");
                        return;
                    };
                    if self.current_sample.is_none() {
                        ui.label("No sample selected.");
                        return;
                    }

                    ui.horizontal(|ui| {
                        if ui
                            .button("Add this plot to layout")
                            .on_hover_text("Add this detached population view to the Layout Editor")
                            .clicked()
                        {
                            self.add_layout_popout();
                        }
                    });
                    ui.separator();

                    // The detached window exposes the same controls as the main
                    // window: axes, ranges, scales, colour, view and gating.
                    self.popout_controls(ui, i);
                    ui.separator();

                    let (Some(x), Some(y)) = (self.popout.x_param, self.popout.y_param) else {
                        ui.label("Select X and Y parameters.");
                        return;
                    };
                    let (x_label, y_label) = match self.current_fcs() {
                        Some(f) => (param_display_name(f, x), param_display_name(f, y)),
                        None => (String::new(), String::new()),
                    };
                    let pts = self.population_points_xy(i, x, y);
                    let visuals = self.gate_visuals_for(
                        x,
                        y,
                        self.popout.x_range,
                        self.popout.y_range,
                        self.popout.selected_gate,
                    );
                    let handles = match self.popout.selected_gate {
                        Some(gi) if self.gates.get(gi).map(|g| (g.x_idx, g.y_idx)) == Some((x, y)) => {
                            self.gates[gi].kind.handles()
                        }
                        _ => Vec::new(),
                    };
                    let cfg = PlotCfg {
                        points: pts,
                        x_range: self.popout.x_range,
                        y_range: self.popout.y_range,
                        x_scale: self.popout.x_scale,
                        y_scale: self.popout.y_scale,
                        point_color: self.popout.point_color,
                        gradient: self.popout.gradient,
                        view: self.popout.view,
                        contour_levels: self.popout.contour_levels,
                        gates: visuals,
                        handles,
                        size: (620.0, 500.0),
                        x_label: format!("{x_label} ({})", scale_label(self.popout.x_scale)),
                        y_label: if matches!(self.popout.view, ViewMode::Histogram) {
                            "Count".to_string()
                        } else {
                            format!("{y_label} ({})", scale_label(self.popout.y_scale))
                        },
                        interactive: true,
                        line_preview: matches!(self.popout.new_gate_type, NewGateType::Bilinear),
                    };
                    ui.label(format!("{} events shown — drag to gate, drag handles to edit", cfg.points.len()));

                    let mut inter = PlotInteraction {
                        drag_start: self.popout.interact.drag_start,
                        drag_current: self.popout.interact.drag_current,
                        ..Default::default()
                    };
                    show_plot(ui, "popout_plot", &cfg, &mut inter);

                    let tol = 0.02
                        * (self.popout.x_range.1 - self.popout.x_range.0)
                            .abs()
                            .max((self.popout.y_range.1 - self.popout.y_range.0).abs())
                            .max(1.0);
                    let outcome = drive_interact(
                        &mut self.popout.interact,
                        &inter,
                        &self.gates,
                        self.popout.selected_gate,
                        self.popout.new_gate_type,
                        tol,
                    );
                    if let Some(outcome) = outcome {
                        match outcome {
                            InteractOutcome::Create { start, end } => {
                                let created = self.create_gates(
                                    start,
                                    end,
                                    x,
                                    y,
                                    self.popout.gate,
                                    self.popout.new_gate_type,
                                );
                                if let Some(&last) = created.last() {
                                    self.popout.selected_gate = Some(last);
                                }
                            }
                            InteractOutcome::Update { gate, kind } => {
                                if let Some(g) = self.gates.get_mut(gate) {
                                    g.kind = kind;
                                }
                                self.recompute_populations();
                            }
                            InteractOutcome::Click(p) => self.create_click_gate(p, true),
                        }
                    }
                });
            self.popout.open = open;
        }

        // ----- "Apply existing gates to this file?" prompt -----
        if let Some(idx) = self.pending_apply_prompt {
            if idx >= self.samples.len() {
                self.pending_apply_prompt = None;
            } else {
                let name = self.samples[idx].id.clone();
                let n = self.gates.len();
                let mut choice: Option<bool> = None;
                egui::Window::new("Apply gates to new file?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Loaded \"{name}\"."));
                        ui.label(format!(
                            "Apply the existing {n} gate(s) to this file?"
                        ));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button("Apply gates").clicked() {
                                choice = Some(true);
                            }
                            if ui.button("Don't apply").clicked() {
                                choice = Some(false);
                            }
                        });
                        ui.weak("You can change this later with the \"gates\" checkbox in the sample list.");
                    });
                if let Some(apply) = choice {
                    if let Some(s) = self.samples.get_mut(idx) {
                        s.apply_gates = apply;
                    }
                    self.pending_apply_prompt = None;
                    self.recompute_populations();
                }
            }
        }

        // ----- Undo history -----
        // Record once per frame (coalescing drags into a single step).
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        self.record_history(pointer_down);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fcs::{FcsFile, Parameter};
    use ndarray::array;
    use std::collections::HashMap;

    fn fcs_with(names: &[&str]) -> FcsFile {
        let parameters = names
            .iter()
            .map(|n| Parameter {
                name: (*n).to_string(),
                description: (*n).to_string(),
                range: 1.0,
                exponent: 0.0,
                min: 0.0,
                max: 1.0,
            })
            .collect();
        FcsFile {
            version: "FCS3.1".to_string(),
            keywords: HashMap::new(),
            parameters,
            event_count: 0,
            data: ndarray::Array2::zeros((0, names.len())),
        }
    }

    #[test]
    fn default_axes_prefer_fsc_h_and_ssc_h() {
        let fcs = fcs_with(&["FSC-A", "SSC-A", "FSC-H", "SSC-H", "CD3"]);
        assert_eq!(default_param_pair(&fcs), (2, 3));

        // Lazily falls back to FSC-A / SSC-A.
        let fcs = fcs_with(&["FSC-A", "SSC-A", "CD3"]);
        assert_eq!(default_param_pair(&fcs), (0, 1));

        // No FSC/SSC channels: use the first two parameters.
        let fcs = fcs_with(&["A", "B", "C"]);
        assert_eq!(default_param_pair(&fcs), (0, 1));

        // Single parameter: both axes clamp to index 0.
        let fcs = fcs_with(&["FSC-H"]);
        assert_eq!(default_param_pair(&fcs), (0, 0));
    }

    #[test]
    fn edit_number_formatting_round_trips() {
        assert_eq!(format_edit_num(0.0), "0");
        assert_eq!(format_edit_num(7_000_000.0), "7e6");
        assert_eq!(format_edit_num(5_300_871.0), "5.300871e6");
        assert_eq!(format_edit_num(123.0), "123");
        assert_eq!(format_edit_num(1.5), "1.5");
        assert_eq!(format_edit_num(1e-4), "1e-4");
        // The shown text must parse back to exactly the original value.
        for v in [0.0, 7_000_000.0, 5_300_871.0, 123.0, 1.5, 1e-4, 42.0, 0.015] {
            let s = format_edit_num(v);
            let back: f64 = s.parse().unwrap_or_else(|_| panic!("{s} did not parse"));
            assert_eq!(back, v, "{v} -> {s} -> {back}");
        }
    }

    #[test]
    fn undo_redo_restores_gate_edits() {
        let rect = || GateKind::Rect {
            x_min: 0.0,
            x_max: 1.0,
            y_min: 0.0,
            y_max: 1.0,
            angle: 0.0,
        };
        let mut app = FlowersApp::default();
        // Seed the empty starting state (as the first frame would).
        app.record_history(false);
        assert!(!app.can_undo());
        assert!(!app.can_redo());

        app.push_gate("G1".into(), rect(), None, 0, 1);
        app.record_history(false);
        assert_eq!(app.gates.len(), 1);
        assert!(app.can_undo());

        app.push_gate("G2".into(), rect(), None, 0, 1);
        app.record_history(false);
        assert_eq!(app.gates.len(), 2);

        app.undo();
        assert_eq!(app.gates.len(), 1);
        assert!(app.can_redo());
        app.undo();
        assert_eq!(app.gates.len(), 0);
        assert!(!app.can_undo());

        app.redo();
        assert_eq!(app.gates.len(), 1);
        app.redo();
        assert_eq!(app.gates.len(), 2);
        assert!(!app.can_redo());
    }

    #[test]
    fn undo_is_not_duplicated_when_nothing_changes() {
        let mut app = FlowersApp::default();
        app.record_history(false);
        // No edit: repeated frames must not grow the history.
        app.record_history(false);
        app.record_history(false);
        assert_eq!(app.history.len(), 1);
        assert!(!app.can_undo());
    }

    #[test]
    fn selecting_a_gate_does_not_create_history_entry() {
        let mut app = FlowersApp::default();
        app.record_history(false);
        app.push_gate(
            "G1".into(),
            GateKind::Rect {
                x_min: 0.0,
                x_max: 1.0,
                y_min: 0.0,
                y_max: 1.0,
                angle: 0.0,
            },
            None,
            0,
            1,
        );
        app.record_history(false);
        assert_eq!(app.history.len(), 2);
        // Selecting a gate is navigation, not an edit.
        app.selected_gate = Some(0);
        app.record_history(false);
        assert_eq!(app.history.len(), 2);
        assert!(!app.can_redo());
    }

    /// Writes a minimal, valid FCS 3.0 file (see `workspace::tests`).
    fn write_fcs(path: &std::path::Path, events: usize, params: usize) {
        use crate::fcs::FCS_HEADER_LEN;
        let mut text = Vec::new();
        text.push(b'/');
        text.extend_from_slice(format!("$TOT/{events}/").as_bytes());
        text.extend_from_slice(format!("$PAR/{params}/").as_bytes());
        for n in 1..=params {
            text.extend_from_slice(format!("$P{n}N/P{n}/").as_bytes());
            text.extend_from_slice(format!("$P{n}R/4.0/").as_bytes());
        }
        text.extend_from_slice(b"$BYTEORD/1,2,3,4/");
        let text_start = FCS_HEADER_LEN;
        let text_end = text_start + text.len();
        let data_start = text_end;
        let data_end = data_start;
        let analysis_start = data_end + 1;
        let analysis_end = analysis_start + 1;
        fn ptr(n: usize) -> [u8; 8] {
            let s = format!("{n:>8}");
            let mut b = [0u8; 8];
            b.copy_from_slice(s.as_bytes());
            b
        }
        let mut buf = Vec::new();
        buf.extend_from_slice(b"FCS3.0    ");
        buf.extend_from_slice(&ptr(text_start));
        buf.extend_from_slice(&ptr(text_end));
        buf.extend_from_slice(&ptr(data_start));
        buf.extend_from_slice(&ptr(data_end));
        buf.extend_from_slice(&ptr(analysis_start));
        buf.extend_from_slice(&ptr(analysis_end));
        buf.extend_from_slice(&text);
        for i in 0..events {
            for _ in 0..params {
                buf.extend_from_slice(&(i as f32).to_le_bytes());
            }
        }
        std::fs::write(path, buf).unwrap();
    }

    #[test]
    fn save_and_load_workspace_round_trips() {
        let dir = std::path::Path::new("/tmp/opencode");
        std::fs::create_dir_all(dir).unwrap();
        let fcs_path = dir.join("ws_roundtrip.fcs");
        write_fcs(&fcs_path, 4, 2);

        let mut app = FlowersApp::default();
        app.load_file(&fcs_path);
        assert_eq!(app.samples.len(), 1);
        app.push_gate(
            "G1".into(),
            GateKind::Rect {
                x_min: 0.0,
                x_max: 1.0,
                y_min: 0.5,
                y_max: 2.0,
                angle: 0.25,
            },
            None,
            0,
            1,
        );
        app.selected_gate = Some(0);
        app.view_mode = ViewMode::Contour;
        app.contour_levels = 6;
        app.x_range = (1.0, 3.0);
        app.y_range = (2.0, 4.0);
        app.layout_columns = 3;
        app.add_layout_current();
        assert_eq!(app.layout_panels.len(), 1);

        let wsp = dir.join("ws_roundtrip.wsp");
        app.save_workspace(&wsp);
        assert!(wsp.exists());

        let mut app2 = FlowersApp::default();
        app2.load_workspace(&wsp);
        assert_eq!(app2.samples.len(), 1);
        assert_eq!(app2.samples[0].id, "ws_roundtrip.fcs");
        assert!(app2.samples[0].path.ends_with("ws_roundtrip.fcs"));
        assert_eq!(app2.gates.len(), 1);
        assert_eq!(app2.gates[0].name, "G1");
        assert_eq!(
            app2.gates[0].kind,
            GateKind::Rect {
                x_min: 0.0,
                x_max: 1.0,
                y_min: 0.5,
                y_max: 2.0,
                angle: 0.25,
            }
        );
        assert_eq!(app2.selected_gate, Some(0));
        assert_eq!(app2.view_mode, ViewMode::Contour);
        assert_eq!(app2.contour_levels, 6);
        assert_eq!(app2.x_range, (1.0, 3.0));
        assert_eq!(app2.y_range, (2.0, 4.0));
        assert_eq!(app2.layout_columns, 3);
        assert_eq!(app2.layout_panels.len(), 1);
        assert_eq!(app2.layout_panels[0].sample, 0);
    }

    #[test]
    fn bilinear_gate_selects_region_under_line() {
        // Horizontal line at y=5 spanning x in [0, 10].
        let kind = GateKind::Bilinear {
            x1: 0.0,
            y1: 5.0,
            x2: 10.0,
            y2: 5.0,
            below: true,
        };
        let gate = kind.to_gate(0, 1);
        let data = array![[1.0, 1.0], [1.0, 9.0], [20.0, 1.0], [5.0, 5.0]];
        // (1,1) under the line & in band; (1,9) above; (20,1) outside band;
        // (5,5) exactly on the line is included.
        assert_eq!(gate.filter(&data), vec![0, 3]);
    }

    #[test]
    fn bilinear_gate_follows_a_diagonal() {
        // Line from (0,10) to (10,0): y = 10 - x.
        let kind = GateKind::Bilinear {
            x1: 0.0,
            y1: 10.0,
            x2: 10.0,
            y2: 0.0,
            below: true,
        };
        let gate = kind.to_gate(0, 1);
        let data = array![[5.0, 4.0], [5.0, 6.0], [2.0, 1.0]];
        // (5,4) below y=5 -> in; (5,6) above -> out; (2,1) below y=8 -> in.
        assert_eq!(gate.filter(&data), vec![0, 2]);
    }

    #[test]
    fn bilinear_polygon_spans_the_window_height() {
        let kind = GateKind::Bilinear {
            x1: 2.0,
            y1: 5.0,
            x2: 8.0,
            y2: 5.0,
            below: true,
        };
        let poly = kind.polygon((0.0, 10.0), (0.0, 10.0));
        // Top edge is the line; the fill drops to the bottom of the Y axis.
        assert_eq!(poly[0], [2.0, 5.0]);
        assert_eq!(poly[1], [8.0, 5.0]);
        assert_eq!(poly[2], [8.0, 0.0]);
        assert_eq!(poly[3], [2.0, 0.0]);
    }
}
