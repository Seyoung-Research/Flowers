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
//! * hierarchical gate tree with counts and percentages
//! * CSV export of counts / percentages
//! * quadrant gate (draw or click)
//! * bilinear gate
//! * dark / light theme switch
//! * detached population window with the full plotting & gating toolkit
//! * interactive gate manipulation (move / rotate / enlarge / shrink)
//! * free-form visual layout editor (drag, resize, reorder, preview)
//! * figure export to SVG, PNG, JPEG and TIFF

use eframe::egui;
use egui_plot::{Bar, BarChart, Line, Plot, PlotBounds, Points, Polygon};

use crate::export::{write_populations_csv, PopulationRow};
use crate::fcs::{FcsFile, FcsParser};
use crate::gating::{
    BilinearGate, EllipseGate, Gate as GatingGate, Quadrant, QuadrantGate, RectangularGate,
};
use crate::layout::{
    render_layout_free, LayoutOptions, LayoutPlacement,
};
use crate::plot::{
    density_grid, density_rgb, dot_plot_svg, point_densities, AxisScale, PlotOverlay, PlotStyle,
};
use crate::raster::{save_svg_as_image, ImageFormat};

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
    /// Bilinear line-end handle (0..4).
    End(usize),
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
    /// One wedge of a 4-way bilinear (crossing lines) split.
    Bilinear {
        cx: f64,
        cy: f64,
        angle1: f64,
        angle2: f64,
        radius: f64,
        side1: bool,
        side2: bool,
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
                cx,
                cy,
                angle1,
                angle2,
                radius,
                side1,
                side2,
            } => Box::new(BilinearGate::new(
                x_idx,
                y_idx,
                *cx as f32,
                *cy as f32,
                *angle1 as f32,
                *angle2 as f32,
                *radius as f32,
                *side1,
                *side2,
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
                cx,
                cy,
                angle1,
                angle2,
                side1,
                side2,
                ..
            } => {
                let rect = vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
                let p1 = clip_halfplane(&rect, line_coeffs(*cx, *cy, *angle1), *side1);
                let p2 = clip_halfplane(&p1, line_coeffs(*cx, *cy, *angle2), *side2);
                if p2.len() >= 3 {
                    let mut closed = p2;
                    closed.push(closed[0]);
                    closed
                } else {
                    Vec::new()
                }
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
                cx,
                cy,
                angle1,
                angle2,
                radius,
                ..
            } => {
                let line = |angle: f64| {
                    vec![
                        [cx - radius * angle.cos(), cy - radius * angle.sin()],
                        [cx + radius * angle.cos(), cy + radius * angle.sin()],
                    ]
                };
                vec![line(*angle1), line(*angle2)]
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
            GateKind::Bilinear { cx, cy, .. } => {
                format!("bilinear centre ({cx:.2},{cy:.2})")
            }
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
            GateKind::Bilinear { cx, cy, .. } => [*cx, *cy],
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
            GateKind::Bilinear { cx, cy, .. } => {
                *cx += dx;
                *cy += dy;
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
                angle1, angle2, ..
            } => {
                *angle1 += delta;
                *angle2 += delta;
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
                cx, cy, radius, ..
            } => {
                *cx = center[0] + (*cx - center[0]) * f;
                *cy = center[1] + (*cy - center[1]) * f;
                *radius *= f;
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
            GateKind::Bilinear {
                cx,
                cy,
                angle1,
                angle2,
                radius,
                ..
            } => {
                let end = |a: f64, sgn: f64| {
                    [cx + sgn * radius * a.cos(), cy + sgn * radius * a.sin()]
                };
                vec![
                    ([*cx, *cy], HandleId::Center),
                    (end(*angle1, -1.0), HandleId::End(0)),
                    (end(*angle1, 1.0), HandleId::End(1)),
                    (end(*angle2, -1.0), HandleId::End(2)),
                    (end(*angle2, 1.0), HandleId::End(3)),
                ]
            }
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
            GateKind::Bilinear {
                cx,
                cy,
                angle1,
                angle2,
                side1,
                side2,
                ..
            } => {
                let d1 = (x - cx) * angle1.sin() - (y - cy) * angle1.cos();
                let d2 = (x - cx) * angle2.sin() - (y - cy) * angle2.cos();
                (d1 >= 0.0) == *side1 && (d2 >= 0.0) == *side2
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
        ManipTarget::Handle(HandleId::Vertex(i)) => {
            if let GateKind::Polygon { vertices, .. } = &mut g {
                if *i < vertices.len() {
                    vertices[*i] = cur;
                }
            }
        }
        ManipTarget::Handle(HandleId::End(k)) => {
            if let GateKind::Bilinear {
                cx,
                cy,
                angle1,
                angle2,
                radius,
                ..
            } = &mut g
            {
                let line = *k / 2;
                let ang = (cur[1] - *cy).atan2(cur[0] - *cx);
                let r = ((cur[0] - *cx).powi(2) + (cur[1] - *cy).powi(2)).sqrt();
                if line == 0 {
                    *angle1 = ang;
                } else {
                    *angle2 = ang;
                }
                *radius = r.max(1e-6);
            }
        }
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
#[derive(Debug, Clone)]
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
/// `sample_idx` is the index of the sample being computed for. Gates with
/// `apply_to_all == false` are only evaluated for their owner sample; gates
/// with an out-of-range parameter index simply yield an empty population.
fn compute_gate_populations(
    data: &ndarray::Array2<f32>,
    gates: &[GateDef],
    sample_idx: Option<usize>,
) -> Vec<Vec<usize>> {
    let (total, n_params) = data.dim();
    let mut result: Vec<Vec<usize>> = vec![Vec::new(); gates.len()];
    for (i, gate) in gates.iter().enumerate() {
        // Respect the per-gate "apply to all samples" flag.
        if !gate.apply_to_all {
            if gate.owner != sample_idx {
                continue;
            }
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

/// Half-plane coefficients `(a, b, c)` such that `a*x + b*y + c >= 0` selects
/// the positive side of the line through `(cx, cy)` with orientation `angle`.
fn line_coeffs(cx: f64, cy: f64, angle: f64) -> (f64, f64, f64) {
    let a = angle.sin();
    let b = -angle.cos();
    let c = cy * angle.cos() - cx * angle.sin();
    (a, b, c)
}

/// Sutherland–Hodgman clip of a polygon by the half-plane `a*x + b*y + c >= 0`
/// (or the opposite side when `positive` is false).
fn clip_halfplane(poly: &[[f64; 2]], (a, b, c): (f64, f64, f64), positive: bool) -> Vec<[f64; 2]> {
    let (a, b, c) = if positive { (a, b, c) } else { (-a, -b, -c) };
    let dist = |p: [f64; 2]| a * p[0] + b * p[1] + c;
    let mut out: Vec<[f64; 2]> = Vec::new();
    if poly.is_empty() {
        return out;
    }
    for i in 0..poly.len() {
        let cur = poly[i];
        let prev = poly[(i + poly.len() - 1) % poly.len()];
        let d_cur = dist(cur);
        let d_prev = dist(prev);
        let cur_in = d_cur >= 0.0;
        let prev_in = d_prev >= 0.0;
        if cur_in {
            if !prev_in {
                out.push(intersect(prev, cur, d_prev, d_cur));
            }
            out.push(cur);
        } else if prev_in {
            out.push(intersect(prev, cur, d_prev, d_cur));
        }
    }
    out
}

fn intersect(p0: [f64; 2], p1: [f64; 2], d0: f64, d1: f64) -> [f64; 2] {
    let denom = d0 - d1;
    let t = if denom.abs() < f64::EPSILON {
        0.0
    } else {
        (d0 / denom).clamp(0.0, 1.0)
    };
    [p0[0] + (p1[0] - p0[0]) * t, p0[1] + (p1[1] - p0[1]) * t]
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

            // Drag rectangle (created by the user while gating).
            if let (Some(a), Some(b)) = (inter.drag_start, inter.drag_current) {
                let a = [tx(a[0]), ty(a[1])];
                let b = [tx(b[0]), ty(b[1])];
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
    fcs: FcsFile,
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

/// One entry in the layout editor. Panels live on a free-form canvas and can be
/// moved / resized with the mouse (Photoshop-style).
#[derive(Clone)]
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
                    self.samples.push(SampleEntry { id: id.clone(), fcs });
                    self.current_sample = Some(self.samples.len() - 1);
                    self.x_param = None;
                    self.y_param = None;
                    self.plot_points.clear();
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

    /// Load files (e.g. from the command line) and prime the first plot.
    pub fn open_paths(&mut self, paths: &[std::path::PathBuf]) {
        for p in paths {
            self.load_file(p);
        }
        if self.current_sample.is_some() && self.x_param.is_none() {
            if let Some(fcs) = self.current_fcs() {
                let n = fcs.parameters.len();
                if n > 0 {
                    self.x_param = Some(0);
                    self.y_param = Some(if n > 1 { 1 } else { 0 });
                }
            }
            self.update_plot_points();
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
        self.pop_indices = match self.current_fcs() {
            Some(fcs) => {
                compute_gate_populations(&fcs.data, &self.gates, self.current_sample)
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
                let radius = ((x_max - x_min).abs().max((y_max - y_min).abs())) / 2.0;
                created = self.create_bilinear_at([cx, cy], radius, x_idx, y_idx, parent);
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

    /// Create bilinear gates as a horizontal-line threshold (below / above).
    fn create_bilinear_at(
        &mut self,
        center: [f64; 2],
        radius: f64,
        x_idx: usize,
        y_idx: usize,
        parent: Option<usize>,
    ) -> Vec<usize> {
        let base = self.gates.len() + 1;
        let radius = radius.max(1.0);
        let mut created = Vec::new();
        // Both dividing lines are horizontal (angle = 0), so each gate selects
        // one side of a single horizontal threshold line at `cy`.
        //   d = (x - cx) * sin(0) - (y - cy) * cos(0) = cy - y
        //   side = true  → d >= 0 → y <= cy  (below the line)
        //   side = false → d < 0  → y >  cy  (above the line)
        for (_i, (name, s1, s2)) in [
            (format!("B{base}.1 Below"), true, true),
            (format!("B{base}.2 Above"), false, false),
        ]
        .into_iter()
        .enumerate()
        {
            let kind = GateKind::Bilinear {
                cx: center[0],
                cy: center[1],
                angle1: 0.0,
                angle2: 0.0,
                radius,
                side1: s1,
                side2: s2,
            };
            created.push(self.push_gate(
                name,
                kind,
                parent,
                x_idx,
                y_idx,
            ));
        }
        created
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
            ui.add(egui::DragValue::new(&mut self.popout.x_range.0).speed(1.0));
            ui.add(egui::DragValue::new(&mut self.popout.x_range.1).speed(1.0));
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
            ui.add(egui::DragValue::new(&mut self.popout.y_range.0).speed(1.0));
            ui.add(egui::DragValue::new(&mut self.popout.y_range.1).speed(1.0));
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
                let style = PlotStyle {
                    point_color: color_to_hex(self.point_color),
                    x_scale: AxisScale::Linear,
                    y_scale: AxisScale::Linear,
                    gradient: self.gradient,
                    contour: matches!(spec.view, ViewMode::Contour),
                    contour_levels: self.contour_levels,
                    title: spec.title.clone(),
                    ..Default::default()
                };
                let svg = if matches!(spec.view, ViewMode::Histogram) {
                    crate::plot::histogram_svg(
                        &fcs.data,
                        spec.x,
                        &param_display_name(fcs, spec.x),
                        &[],
                        None,
                        50,
                        &style,
                    )
                } else {
                    let indices: Vec<usize> = spec
                        .gate
                        .map(|g| {
                            compute_gate_populations(&fcs.data, &self.gates, Some(spec.sample))
                                .get(g)
                                .cloned()
                                .unwrap_or_default()
                        })
                        .unwrap_or_default();
                    dot_plot_svg(
                        &fcs.data,
                        spec.x,
                        spec.y,
                        &param_display_name(fcs, spec.x),
                        &param_display_name(fcs, spec.y),
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
                let pops = compute_gate_populations(&fcs.data, &self.gates, Some(sample));
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

            // Axis labels.
            let fcs = &self.samples[panel.sample].fcs;
            let x_name = param_display_name(fcs, panel.x);
            let y_name = param_display_name(fcs, panel.y);
            painter.text(
                r.min + egui::vec2(r.width() / 2.0, r.height() - 5.0),
                egui::Align2::CENTER_TOP,
                &x_name,
                egui::FontId::proportional(9.0),
                egui::Color32::from_gray(100),
            );
            painter.text(
                r.min + egui::vec2(6.0, r.height() / 2.0),
                egui::Align2::CENTER_CENTER,
                &y_name,
                egui::FontId::proportional(9.0),
                egui::Color32::from_gray(100),
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
                for (i, sample) in self.samples.iter().enumerate() {
                    let selected = self.current_sample == Some(i);
                    if ui.selectable_label(selected, &sample.id).clicked() {
                        clicked_sample = Some(i);
                    }
                }
                if let Some(i) = clicked_sample {
                    self.current_sample = Some(i);
                    self.update_plot_points();
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
                            ui.add(egui::DragValue::new(&mut self.x_range.0).speed(1.0));
                            ui.add(egui::DragValue::new(&mut self.x_range.1).speed(1.0));
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
                            ui.add(egui::DragValue::new(&mut self.y_range.0).speed(1.0));
                            ui.add(egui::DragValue::new(&mut self.y_range.1).speed(1.0));
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
                            ui.checkbox(&mut self.gates[i].apply_to_all, "");
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
                        cx,
                        cy,
                        angle1,
                        angle2,
                        radius,
                        side1,
                        side2,
                    } => {
                        ui.horizontal(|ui| {
                            ui.label("Centre X:");
                            changed |= ui.add(egui::DragValue::new(cx).speed(1.0)).changed();
                            ui.label("Centre Y:");
                            changed |= ui.add(egui::DragValue::new(cy).speed(1.0)).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("Angle 1 (deg):");
                            let mut deg1 = angle1.to_degrees();
                            if ui.add(egui::DragValue::new(&mut deg1).speed(1.0)).changed() {
                                *angle1 = deg1.to_radians();
                                changed = true;
                            }
                            ui.label("Angle 2 (deg):");
                            let mut deg2 = angle2.to_degrees();
                            if ui.add(egui::DragValue::new(&mut deg2).speed(1.0)).changed() {
                                *angle2 = deg2.to_radians();
                                changed = true;
                            }
                            ui.label("Length:");
                            changed |= ui.add(egui::DragValue::new(radius).speed(1.0)).changed();
                        });
                        ui.horizontal(|ui| {
                            changed |= ui.checkbox(side1, "Positive side of line 1").changed();
                            changed |= ui.checkbox(side2, "Positive side of line 2").changed();
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
    }
}
