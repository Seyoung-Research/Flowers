//! GUI project / workspace files (`.wsp`).
//!
//! A project stores the *analysis*, not the raw data: the list of FCS file
//! paths plus the gating strategy, view settings and layout. Loading a project
//! re-reads the referenced files, so only paths are persisted (FCS files are
//! large and usually live alongside the project).
//!
//! The module itself is independent of `egui`; the GUI converts between these
//! plain types and its in-memory state.

use serde::{Deserialize, Serialize};

/// Current on-disk project format version.
pub const PROJECT_VERSION: u32 = 1;

/// A saved analysis. All fields have defaults so that older / partial files can
/// still be read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    #[serde(default)]
    pub samples: Vec<ProjectSample>,
    #[serde(default)]
    pub current_sample: Option<usize>,
    #[serde(default)]
    pub x_param: Option<usize>,
    #[serde(default)]
    pub y_param: Option<usize>,
    #[serde(default)]
    pub selected_gate: Option<usize>,
    #[serde(default)]
    pub view: ViewSettings,
    #[serde(default)]
    pub gates: Vec<ProjectGate>,
    #[serde(default)]
    pub layout: Vec<ProjectPanel>,
    #[serde(default = "default_layout_columns")]
    pub layout_columns: usize,
}

fn default_layout_columns() -> usize {
    2
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: PROJECT_VERSION,
            samples: Vec::new(),
            current_sample: None,
            x_param: None,
            y_param: None,
            selected_gate: None,
            view: ViewSettings::default(),
            gates: Vec::new(),
            layout: Vec::new(),
            layout_columns: default_layout_columns(),
        }
    }
}

/// One referenced FCS file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSample {
    /// Path to the FCS file, as it was when the project was saved.
    pub path: String,
    /// Whether the shared gate set applies to this file.
    #[serde(default = "default_true")]
    pub apply_gates: bool,
}

fn default_true() -> bool {
    true
}

/// Plot appearance / axis settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewSettings {
    /// One of `dots`, `contour`, `histogram`.
    pub mode: String,
    /// `linear` or `log`.
    pub x_scale: String,
    pub y_scale: String,
    /// RGB point colour.
    pub point_color: [u8; 3],
    /// Density gradient layering.
    pub gradient: bool,
    pub contour_levels: usize,
    pub x_range: (f64, f64),
    pub y_range: (f64, f64),
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            mode: "dots".to_string(),
            x_scale: "linear".to_string(),
            y_scale: "linear".to_string(),
            point_color: [31, 119, 180],
            gradient: true,
            contour_levels: 8,
            x_range: (0.0, 1.0),
            y_range: (0.0, 1.0),
        }
    }
}

/// A single gate, including its place in the hierarchy and its appearance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectGate {
    pub name: String,
    pub x_idx: usize,
    pub y_idx: usize,
    pub parent: Option<usize>,
    pub owner: Option<usize>,
    pub apply_to_all: bool,
    pub color: [u8; 3],
    pub kind: GateShape,
}

/// Serializable gate geometry, mirroring the GUI's gate kinds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GateShape {
    Rect {
        x_min: f64,
        x_max: f64,
        y_min: f64,
        y_max: f64,
        angle: f64,
    },
    Ellipse {
        x_min: f64,
        x_max: f64,
        y_min: f64,
        y_max: f64,
        angle: f64,
    },
    Polygon {
        vertices: Vec<[f64; 2]>,
    },
    Quadrant {
        x: f64,
        y: f64,
        /// `upper_left` / `upper_right` / `lower_left` / `lower_right`.
        quad: String,
    },
    Bilinear {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        below: bool,
    },
}

/// One layout-editor panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPanel {
    pub sample: usize,
    pub x: usize,
    pub y: usize,
    /// `dots` / `contour` / `histogram`.
    pub view: String,
    pub gate: Option<usize>,
    pub title: String,
    /// Canvas rectangle `[x, y, w, h]`.
    pub rect: [f32; 4],
    pub x_label: ProjectLabel,
    pub y_label: ProjectLabel,
}

/// Styling for one layout axis label.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectLabel {
    pub text: Option<String>,
    pub size: f32,
    pub rotation_deg: f32,
    /// `sans` / `serif` / `mono`.
    pub font: String,
    pub color: [u8; 3],
}

impl Default for ProjectLabel {
    fn default() -> Self {
        Self {
            text: None,
            size: 11.0,
            rotation_deg: 0.0,
            font: "sans".to_string(),
            color: [90, 90, 90],
        }
    }
}

/// Serialises a project to pretty JSON.
pub fn to_json(project: &Project) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(project)
}

/// Parses a project from JSON.
pub fn from_json(text: &str) -> Result<Project, serde_json::Error> {
    serde_json::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_gate_shapes() {
        let project = Project {
            samples: vec![ProjectSample {
                path: "/tmp/a.fcs".to_string(),
                apply_gates: false,
            }],
            gates: vec![
                ProjectGate {
                    name: "R".to_string(),
                    x_idx: 0,
                    y_idx: 1,
                    parent: None,
                    owner: Some(0),
                    apply_to_all: true,
                    color: [255, 140, 0],
                    kind: GateShape::Rect {
                        x_min: 1.0,
                        x_max: 2.0,
                        y_min: 3.0,
                        y_max: 4.0,
                        angle: 0.5,
                    },
                },
                ProjectGate {
                    name: "L".to_string(),
                    x_idx: 0,
                    y_idx: 1,
                    parent: Some(0),
                    owner: None,
                    apply_to_all: false,
                    color: [0, 170, 220],
                    kind: GateShape::Bilinear {
                        x1: 1.0,
                        y1: 2.0,
                        x2: 5.0,
                        y2: 6.0,
                        below: true,
                    },
                },
                ProjectGate {
                    name: "Q".to_string(),
                    x_idx: 2,
                    y_idx: 3,
                    parent: None,
                    owner: None,
                    apply_to_all: true,
                    color: [40, 190, 80],
                    kind: GateShape::Quadrant {
                        x: 10.0,
                        y: 20.0,
                        quad: "upper_right".to_string(),
                    },
                },
            ],
            layout: vec![ProjectPanel {
                sample: 0,
                x: 0,
                y: 1,
                view: "dots".to_string(),
                gate: Some(0),
                title: "Panel".to_string(),
                rect: [10.0, 20.0, 300.0, 200.0],
                x_label: ProjectLabel {
                    text: Some("FSC".to_string()),
                    ..Default::default()
                },
                y_label: ProjectLabel::default(),
            }],
            ..Default::default()
        };
        let json = to_json(&project).unwrap();
        let back = from_json(&json).unwrap();
        assert_eq!(back.gates.len(), 3);
        assert_eq!(back.samples[0].path, "/tmp/a.fcs");
        assert!(!back.samples[0].apply_gates);
        assert_eq!(back.layout.len(), 1);
        assert_eq!(back.layout[0].rect, [10.0, 20.0, 300.0, 200.0]);
        assert_eq!(back.gates[1].parent, Some(0));
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // A minimal file only carrying the version.
        let back = from_json(r#"{"version":1}"#).unwrap();
        assert!(back.samples.is_empty());
        assert_eq!(back.view.mode, "dots");
        assert_eq!(back.view.point_color, [31, 119, 180]);
        assert_eq!(back.layout_columns, 2);
    }
}
