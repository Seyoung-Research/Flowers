//! egui-based GUI for `flowers`.
//!
//! Provides a desktop application for loading FCS files, drawing gates on
//! scatter plots (drag to create), saving populations, and exporting SVG plots.

use eframe::egui;
use egui_plot::{Plot, Points, Polygon};
use std::path::PathBuf;

use crate::fcs::FcsParser;
use crate::gating::{EllipseGate, Gate as GatingGate, RectangularGate};
use crate::plot::{dot_plot_svg, PlotStyle};

/// Shape of a gate.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum GateShape {
    #[default]
    Rect,
    Ellipse,
}

/// A single gate drawn on the plot.
#[derive(Debug, Clone, PartialEq)]
struct Gate {
    shape: GateShape,
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            shape: GateShape::Rect,
            x_min: 0.0,
            x_max: 1.0,
            y_min: 0.0,
            y_max: 1.0,
        }
    }
}

impl Gate {
    /// Returns the gate boundary as a closed polygon for egui_plot rendering.
    /// Ellipses are approximated with 32 segments.
    fn polygon_points(&self) -> Vec<[f64; 2]> {
        match self.shape {
            GateShape::Rect => vec![
                [self.x_min, self.y_min],
                [self.x_max, self.y_min],
                [self.x_max, self.y_max],
                [self.x_min, self.y_max],
                [self.x_min, self.y_min],
            ],
            GateShape::Ellipse => {
                let cx = (self.x_min + self.x_max) / 2.0;
                let cy = (self.y_min + self.y_max) / 2.0;
                let rx = (self.x_max - self.x_min) / 2.0;
                let ry = (self.y_max - self.y_min) / 2.0;
                let n = 32;
                (0..=n)
                    .map(|i| {
                        let angle = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
                        [cx + rx * angle.cos(), cy + ry * angle.sin()]
                    })
                    .collect()
            }
        }
    }

    /// Build the appropriate `Gate` trait object for filtering events.
    fn to_gate(&self, x_idx: usize, y_idx: usize) -> Box<dyn GatingGate> {
        match self.shape {
            GateShape::Rect => Box::new(RectangularGate::new(
                x_idx,
                y_idx,
                self.x_min as f32,
                self.x_max as f32,
                self.y_min as f32,
                self.y_max as f32,
            )),
            GateShape::Ellipse => Box::new(EllipseGate::new(
                x_idx,
                y_idx,
                ((self.x_min + self.x_max) / 2.0) as f32,
                ((self.y_min + self.y_max) / 2.0) as f32,
                ((self.x_max - self.x_min) / 2.0) as f32,
                ((self.y_max - self.y_min) / 2.0) as f32,
            )),
        }
    }
}

/// A named, saved gated population.
#[derive(Debug, Clone)]
struct PopulationEntry {
    name: String,
    count: usize,
    total: usize,
    gate: Gate,
    x_idx: usize,
    y_idx: usize,
}

impl PopulationEntry {
    fn percent(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.count as f64 / self.total as f64 * 100.0
        }
    }
}

/// A loaded FCS sample.
struct SampleEntry {
    id: String,
    fcs: crate::fcs::FcsFile,
}

/// The egui application state.
pub struct FlowersApp {
    samples: Vec<SampleEntry>,
    current_sample: Option<usize>,
    x_param: Option<usize>,
    y_param: Option<usize>,
    /// All active gates for the current X/Y parameter pair.
    gates: Vec<Gate>,
    /// Index of the currently selected gate being edited, if any.
    selected_gate: Option<usize>,
    /// Default gate shape for newly created gates.
    default_gate_shape: GateShape,
    /// Drag-to-create state: None when not dragging, Some([x, y]) at drag start.
    drag_start: Option<[f64; 2]>,
    /// Current mouse position during a drag (plot coordinates).
    drag_current: Option<[f64; 2]>,
    populations: Vec<PopulationEntry>,
    plot_points: Vec<[f64; 2]>,
    x_range: (f64, f64),
    y_range: (f64, f64),
    x_data_range: (f64, f64),
    y_data_range: (f64, f64),
    status: String,
    new_pop_name: String,
    export_path: Option<PathBuf>,
    show_export_saved: bool,
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
            default_gate_shape: GateShape::Rect,
            drag_start: None,
            drag_current: None,
            populations: Vec::new(),
            plot_points: Vec::new(),
            x_range: (0.0, 1.0),
            y_range: (0.0, 1.0),
            x_data_range: (0.0, 1.0),
            y_data_range: (0.0, 1.0),
            status: "Ready — load an FCS file".to_string(),
            new_pop_name: "Population 1".to_string(),
            export_path: None,
            show_export_saved: false,
        }
    }
}

/// Return a human-readable display name for a parameter.
fn param_display_name(fcs: &crate::fcs::FcsFile, idx: usize) -> String {
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

impl FlowersApp {
    /// Load an FCS file from disk and make it the current sample.
    fn load_file(&mut self, path: &std::path::Path) {
        match std::fs::read(path) {
            Ok(bytes) => match FcsParser::parse_bytes(&bytes) {
                Ok(fcs) => {
                    let id = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    self.samples.push(SampleEntry { id, fcs });
                    self.current_sample = Some(self.samples.len() - 1);
                    self.x_param = None;
                    self.y_param = None;
                    self.gates.clear();
                    self.selected_gate = None;
                    self.drag_start = None;
                    self.drag_current = None;
                    self.plot_points.clear();
                    self.populations.clear();
                    self.status = format!(
                        "Loaded {}: {} events x {} params",
                        self.samples.last().unwrap().id,
                        self.samples.last().unwrap().fcs.event_count,
                        self.samples.last().unwrap().fcs.parameters.len(),
                    );
                }
                Err(e) => self.status = format!("Error parsing FCS: {}", e),
            },
            Err(e) => self.status = format!("Error reading file: {}", e),
        }
    }

    fn current_fcs(&self) -> Option<&crate::fcs::FcsFile> {
        self.current_sample.map(|i| &self.samples[i].fcs)
    }

    /// Rebuild scatter points, axis ranges, and auto-create a default gate.
    fn update_plot_points(&mut self) {
        self.plot_points.clear();
        self.gates.clear();
        self.selected_gate = None;
        self.drag_start = None;
        self.drag_current = None;
        let Some(fcs) = self.current_fcs() else { return };
        let Some(x) = self.x_param else { return };
        let Some(y) = self.y_param else { return };

        let (_, n_params) = fcs.data.dim();
        if x >= n_params || y >= n_params {
            return;
        }

        let x_col: Vec<f32> = fcs.data.column(x).iter().copied().collect();
        let y_col: Vec<f32> = fcs.data.column(y).iter().copied().collect();

        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        for i in 0..x_col.len() {
            let xv = x_col[i] as f64;
            let yv = y_col[i] as f64;
            self.plot_points.push([xv, yv]);
            min_x = min_x.min(xv);
            max_x = max_x.max(xv);
            min_y = min_y.min(yv);
            max_y = max_y.max(yv);
        }

        if min_x.is_infinite() || min_y.is_infinite() {
            return;
        }

        self.x_data_range = (min_x, max_x);
        self.y_data_range = (min_y, max_y);

        let pad_x = (max_x - min_x) * 0.05;
        let pad_y = (max_y - min_y) * 0.05;
        self.x_range = (min_x - pad_x, max_x + pad_x);
        self.y_range = (min_y - pad_y, max_y + pad_y);

        // Auto-create a default gate over the middle 50% of the data range.
        self.gates.push(Gate {
            shape: self.default_gate_shape,
            x_min: min_x + (max_x - min_x) * 0.25,
            x_max: min_x + (max_x - min_x) * 0.75,
            y_min: min_y + (max_y - min_y) * 0.25,
            y_max: min_y + (max_y - min_y) * 0.75,
        });
        self.selected_gate = Some(0);

        self.update_gate_stats();
    }

    /// Recompute stats for all gates and update the status line.
    fn update_gate_stats(&mut self) {
        let Some(fcs) = self.current_fcs() else { return };
        let Some(x) = self.x_param else { return };
        let Some(y) = self.y_param else { return };

        let mut lines: Vec<String> = Vec::new();
        for (i, gate) in self.gates.iter().enumerate() {
            let gate_obj = gate.to_gate(x, y);
            let count = gate_obj.filter(&fcs.data).len();
            let pct = if fcs.event_count > 0 {
                count as f64 / fcs.event_count as f64 * 100.0
            } else {
                0.0
            };
            let shape_label = match gate.shape {
                GateShape::Rect => "R",
                GateShape::Ellipse => "E",
            };
            lines.push(format!(
                "Gate {}{}: {} events ({:.1}%)",
                i + 1,
                shape_label,
                count,
                pct
            ));
        }

        if lines.is_empty() {
            self.status = "No gates defined".to_string();
        } else {
            self.status = lines.join("  |  ");
        }
    }

    /// Create a gate from a drag rectangle (plot coordinates).
    fn create_gate_from_drag(&mut self, start: [f64; 2], end: [f64; 2]) {
        let gate = Gate {
            shape: self.default_gate_shape,
            x_min: start[0].min(end[0]),
            x_max: start[0].max(end[0]),
            y_min: start[1].min(end[1]),
            y_max: start[1].max(end[1]),
        };
        // Ignore zero-size gates.
        if (gate.x_max - gate.x_min).abs() < f64::EPSILON
            || (gate.y_max - gate.y_min).abs() < f64::EPSILON
        {
            return;
        }
        self.gates.push(gate);
        self.selected_gate = Some(self.gates.len() - 1);
        self.drag_start = None;
        self.drag_current = None;
        self.update_gate_stats();
    }

    /// Add a new empty gate covering the full data range.
    fn add_gate(&mut self) {
        self.gates.push(Gate {
            shape: self.default_gate_shape,
            x_min: self.x_data_range.0,
            x_max: self.x_data_range.1,
            y_min: self.y_data_range.0,
            y_max: self.y_data_range.1,
        });
        self.selected_gate = Some(self.gates.len() - 1);
        self.update_gate_stats();
    }

    /// Remove the currently selected gate.
    fn remove_selected_gate(&mut self) {
        if let Some(idx) = self.selected_gate {
            if idx < self.gates.len() {
                self.gates.remove(idx);
                self.selected_gate = if self.gates.is_empty() {
                    None
                } else {
                    Some(idx.min(self.gates.len() - 1))
                };
                self.update_gate_stats();
            }
        }
    }

    /// Save the currently selected gate as a named population.
    fn save_selected_gate_as_population(&mut self) {
        let Some(fcs) = self.current_fcs() else { return };
        let Some(x) = self.x_param else { return };
        let Some(y) = self.y_param else { return };
        let Some(gate_idx) = self.selected_gate else {
            self.status = "Select a gate to save".to_string();
            return;
        };
        let gate = self.gates[gate_idx].clone();

        let gate_obj = gate.to_gate(x, y);

        let count = gate_obj.filter(&fcs.data).len();
        let total = fcs.event_count;

        self.populations.push(PopulationEntry {
            name: self.new_pop_name.clone(),
            count,
            total,
            gate,
            x_idx: x,
            y_idx: y,
        });

        self.status = format!(
            "Saved population '{}' ({} / {} events)",
            self.new_pop_name, count, total
        );
    }

    /// Export the current plot (with all gates) to SVG.
    fn export_svg(&mut self, path: &std::path::Path) {
        let Some(fcs) = self.current_fcs() else {
            self.status = "No sample loaded to export".to_string();
            return;
        };
        let Some(x) = self.x_param else {
            self.status = "Select X parameter to export".to_string();
            return;
        };
        let Some(y) = self.y_param else {
            self.status = "Select Y parameter to export".to_string();
            return;
        };

        let x_name = param_display_name(fcs, x);
        let y_name = param_display_name(fcs, y);

        // Union of all gate indices for overlay coloring.
        let mut gate_indices: Vec<usize> = Vec::new();
        for gate in &self.gates {
            let gate_obj = gate.to_gate(x, y);
            for idx in gate_obj.filter(&fcs.data) {
                if !gate_indices.contains(&idx) {
                    gate_indices.push(idx);
                }
            }
        }
        gate_indices.sort_unstable();

        let svg = dot_plot_svg(
            &fcs.data,
            x,
            y,
            &x_name,
            &y_name,
            &gate_indices,
            Some((self.x_range.0 as f32, self.x_range.1 as f32)),
            Some((self.y_range.0 as f32, self.y_range.1 as f32)),
            &PlotStyle::default(),
        );

        match std::fs::write(path, &svg) {
            Ok(()) => {
                self.status = format!("Exported plot to {}", path.display());
                self.show_export_saved = true;
                self.export_path = Some(path.to_path_buf());
            }
            Err(e) => self.status = format!("Export failed: {}", e),
        }
    }
}

impl eframe::App for FlowersApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ----- Top menu -----
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                if ui.button("Load FCS").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        self.load_file(&path);
                    }
                }
                if ui.button("Export SVG").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("SVG", &["svg"])
                        .set_file_name("plot.svg")
                        .save_file()
                    {
                        self.export_svg(&path);
                    }
                }
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                ui.separator();
                ui.label("flowers — FlowJo alternative");
            });
        });

        // ----- Left sidebar: samples & parameters -----
        egui::SidePanel::left("sidebar").show(ctx, |ui| {
            ui.heading("flowers");
            ui.separator();

            if ui.button("Load FCS File").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.load_file(&path);
                }
            }

            ui.separator();
            ui.label("Loaded samples:");
            for (i, sample) in self.samples.iter().enumerate() {
                let selected = self.current_sample == Some(i);
                if ui.selectable_label(selected, &sample.id).clicked() {
                    self.current_sample = Some(i);
                    self.x_param = None;
                    self.y_param = None;
                    self.gates.clear();
                    self.selected_gate = None;
                    self.drag_start = None;
                    self.drag_current = None;
                    self.plot_points.clear();
                }
            }

            ui.separator();
            ui.label("Plot parameters:");

            let Some(fcs) = self.current_fcs() else {
                ui.label("No sample selected");
                return;
            };

            let param_display_names: Vec<String> = (0..fcs.parameters.len())
                .map(|i| param_display_name(fcs, i))
                .collect();
            let event_count = fcs.event_count;

            ui.horizontal(|ui| {
                ui.label("X:");
                egui::ComboBox::new("x_param", "Select X")
                    .selected_text(
                        self.x_param
                            .map(|i| param_display_names[i].clone())
                            .unwrap_or_else(|| "Select".to_string()),
                    )
                    .show_ui(ui, |ui| {
                        for (i, name) in param_display_names.iter().enumerate() {
                            if ui
                                .selectable_value(&mut self.x_param, Some(i), name.clone())
                                .changed()
                            {
                                self.update_plot_points();
                            }
                        }
                    });
            });

            ui.horizontal(|ui| {
                ui.label("Y:");
                egui::ComboBox::new("y_param", "Select Y")
                    .selected_text(
                        self.y_param
                            .map(|i| param_display_names[i].clone())
                            .unwrap_or_else(|| "Select".to_string()),
                    )
                    .show_ui(ui, |ui| {
                        for (i, name) in param_display_names.iter().enumerate() {
                            if ui
                                .selectable_value(&mut self.y_param, Some(i), name.clone())
                                .changed()
                            {
                                self.update_plot_points();
                            }
                        }
                    });
            });

            if self.x_param.is_some() && self.y_param.is_some() {
                ui.separator();
                ui.label(format!("Events: {event_count}"));
            }
        });

        // ----- Central panel: scatter plot + drag-to-create gates -----
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.current_fcs().is_none() {
                ui.centered_and_justified(|ui| {
                    ui.label("Load an FCS file to begin");
                });
                return;
            }

            if self.x_param.is_none() || self.y_param.is_none() {
                ui.centered_and_justified(|ui| {
                    ui.label("Select X and Y parameters to plot");
                });
                return;
            }

            if self.plot_points.is_empty() {
                self.update_plot_points();
            }

            let plot_points = self.plot_points.clone();

            // Track drag-to-create gate.
            let plot_response = Plot::new("scatter_plot")
                .view_aspect(1.0)
                .show_axes(true)
                .allow_boxed_zoom(false)
                .allow_drag(false)
                .show(ui, |plot_ui| {
                    // Draw drag rectangle while dragging.
                    if let (Some(start), Some(current)) =
                        (self.drag_start, self.drag_current)
                    {
                        let drag_color =
                            egui::Color32::from_rgba_premultiplied(0, 150, 255, 60);
                        let drag_stroke =
                            egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 120, 200));
                        plot_ui.polygon(
                            Polygon::new(vec![
                                [start[0], start[1]],
                                [current[0], start[1]],
                                [current[0], current[1]],
                                [start[0], current[1]],
                                [start[0], start[1]],
                            ])
                            .fill_color(drag_color)
                            .stroke(drag_stroke),
                        );
                    }

                    // Draw all gate overlays.
                    for (i, gate) in self.gates.iter().enumerate() {
                        let is_selected = self.selected_gate == Some(i);
                        let fill = if is_selected {
                            egui::Color32::from_rgba_premultiplied(255, 165, 0, 80)
                        } else {
                            egui::Color32::from_rgba_premultiplied(255, 165, 0, 40)
                        };
                        let stroke = if is_selected {
                            egui::Stroke::new(3.0_f32, egui::Color32::ORANGE)
                        } else {
                            egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(255, 140, 0))
                        };
                        plot_ui.polygon(
                            Polygon::new(gate.polygon_points())
                                .fill_color(fill)
                                .stroke(stroke),
                        );
                    }

                    plot_ui.points(Points::new(plot_points));

                    // Drag-to-create: start on left-button press, update on move, end on release.
                    if let Some(pointer_pos) = plot_ui.pointer_coordinate() {
                        let pos = [pointer_pos.x, pointer_pos.y];
                        if plot_ui.response().drag_started() {
                            self.drag_start = Some(pos);
                            self.drag_current = Some(pos);
                        } else if self.drag_start.is_some() {
                            self.drag_current = Some(pos);
                            if plot_ui.response().drag_stopped() {
                                let start = self.drag_start.take().unwrap();
                                let end = self.drag_current.take().unwrap();
                                self.create_gate_from_drag(start, end);
                            }
                        }
                    }
                });

            // If drag was cancelled (e.g. right-click), clear it.
            if self.drag_start.is_some()
                && plot_response.response.drag_stopped()
                && self.drag_current.is_some()
            {
                // Already handled above via drag_stopped inside the closure.
            }
        });

        // ----- Bottom panel: gate editor -----
        egui::TopBottomPanel::bottom("gate_editor").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Gates:");
                ui.label("Shape:");
                egui::ComboBox::new("gate_shape", "")
                    .selected_text(
                        match self.default_gate_shape {
                            GateShape::Rect => "Rect",
                            GateShape::Ellipse => "Ellipse",
                        }
                        .to_string(),
                    )
                    .show_ui(ui, |ui| {
                        if ui.selectable_value(
                            &mut self.default_gate_shape,
                            GateShape::Rect,
                            "Rect",
                        )
                        .clicked()
                        {
                        }
                        if ui.selectable_value(
                            &mut self.default_gate_shape,
                            GateShape::Ellipse,
                            "Ellipse",
                        )
                        .clicked()
                        {
                        }
                    });

                if ui.button("+ Add Gate").clicked() {
                    self.add_gate();
                }
                if ui.button("Remove Selected").clicked() {
                    self.remove_selected_gate();
                }
                ui.separator();
                ui.label("Drag on plot to create a new gate.");
                ui.separator();
                ui.label(&self.status);
            });

            if self.gates.is_empty() {
                ui.label("No gates defined yet. Drag on the plot to create a gate.");
            } else {
                egui::CollapsingHeader::new("Gate editor")
                    .default_open(true)
                    .show(ui, |ui| {
                        // Collect which gate is being edited and any changed values,
                        // then apply mutations after the loop to satisfy the borrow checker.
                        let mut selected_idx: Option<usize> = None;
                        let mut gate_edits: Vec<(usize, GateShape, f64, f64, f64, f64)> =
                            Vec::new();

                        egui::Grid::new("gate_list_bottom").show(ui, |ui| {
                            for (i, gate) in self.gates.iter().enumerate() {
                                let is_selected = self.selected_gate == Some(i);
                                let selected_label = if is_selected { " ●" } else { "" };
                                ui.label(format!("Gate {}{}", i + 1, selected_label));
                                if ui.button("Select").clicked() {
                                    selected_idx = Some(i);
                                }
                                ui.end_row();

                                let mut shape = gate.shape;
                                let mut x_min = gate.x_min;
                                let mut x_max = gate.x_max;
                                let mut y_min = gate.y_min;
                                let mut y_max = gate.y_max;

                                ui.label("  Shape:");
                                egui::ComboBox::new(format!("gate_shape_{i}"), "")
                                    .selected_text(
                                        match shape {
                                            GateShape::Rect => "Rect",
                                            GateShape::Ellipse => "Ellipse",
                                        }
                                        .to_string(),
                                    )
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_value(
                                                &mut shape,
                                                GateShape::Rect,
                                                "Rect",
                                            )
                                            .changed()
                                        {
                                        }
                                        if ui
                                            .selectable_value(
                                                &mut shape,
                                                GateShape::Ellipse,
                                                "Ellipse",
                                            )
                                            .changed()
                                        {
                                        }
                                    });
                                ui.end_row();

                                ui.label("  X min:");
                                ui.add(egui::DragValue::new(&mut x_min).speed(1.0));
                                ui.label("  X max:");
                                ui.add(egui::DragValue::new(&mut x_max).speed(1.0));
                                ui.end_row();
                                ui.label("  Y min:");
                                ui.add(egui::DragValue::new(&mut y_min).speed(1.0));
                                ui.label("  Y max:");
                                ui.add(egui::DragValue::new(&mut y_max).speed(1.0));
                                ui.end_row();

                                if shape != gate.shape
                                    || x_min != gate.x_min
                                    || x_max != gate.x_max
                                    || y_min != gate.y_min
                                    || y_max != gate.y_max
                                {
                                    gate_edits.push((i, shape, x_min, x_max, y_min, y_max));
                                }
                            }
                        });

                        // Apply edits
                        for (i, shape, x_min, x_max, y_min, y_max) in gate_edits {
                            self.gates[i].shape = shape;
                            self.gates[i].x_min = x_min;
                            self.gates[i].x_max = x_max;
                            self.gates[i].y_min = y_min;
                            self.gates[i].y_max = y_max;
                            selected_idx = Some(i);
                        }

                        if let Some(i) = selected_idx {
                            self.selected_gate = Some(i);
                            self.update_gate_stats();
                        }

                        if self.selected_gate.is_some() {
                            ui.separator();
                            ui.label("Save selected gate as population:");
                            ui.text_edit_singleline(&mut self.new_pop_name);
                            if ui.button("Save Population").clicked() {
                                self.save_selected_gate_as_population();
                            }
                        }
                    });
            }
        });

        // ----- Right sidebar: saved populations -----
        egui::SidePanel::right("populations").show(ctx, |ui| {
            ui.heading("Populations");
            ui.separator();

            if self.populations.is_empty() {
                ui.label("No populations saved yet");
            } else {
                for pop in &self.populations {
                    egui::Frame::none().fill(egui::Color32::from_gray(25)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(&pop.name);
                            ui.label(format!(
                                "{} / {} ({:.1}%)",
                                pop.count,
                                pop.total,
                                pop.percent()
                            ));
                        });
                    });
                }
            }

            ui.separator();
            if ui.button("Clear All Populations").clicked() {
                self.populations.clear();
            }

            ui.separator();
            ui.label("Status:");
            ui.label(&self.status);

            if self.show_export_saved {
                if let Some(path) = &self.export_path {
                    ui.colored_label(
                        egui::Color32::GREEN,
                        format!("Exported: {}", path.display()),
                    );
                }
                if ui.button("Dismiss").clicked() {
                    self.show_export_saved = false;
                }
            }
        });
    }
}
