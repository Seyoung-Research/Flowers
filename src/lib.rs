//! `flowers` — an open-source FlowJo alternative for flow cytometry analysis.
//!
//! Core capabilities:
//! - FCS 2.0 / 3.0 file parsing (`fcs`)
//! - Compensation / spillover correction (`compensation`)
//! - Gating: rectangular, ellipse, polygon, quadrant, boolean (`gating`)
//! - Population statistics (`statistics`)
//! - Experiment workspace & project files (`workspace`)
//! - SVG visualization: dot plots & histograms (`plot`)
//!
//! See `src/main.rs` for the command-line interface.

pub mod compensation;
pub mod export;
pub mod fcs;
pub mod gating;
pub mod gui;
pub mod layout;
pub mod plot;
pub mod raster;
pub mod statistics;
pub mod workspace;

pub use compensation::CompensationEngine;
pub use export::{populations_csv, write_populations_csv, PopulationRow};
pub use fcs::{FcsError, FcsFile, FcsParser, Parameter};
pub use gating::{
    BilinearGate, BooleanGate, EllipseGate, Gate, PolygonGate, Quadrant, QuadrantGate,
    RectangularGate,
};
pub use layout::{render_layout, render_layout_free, LayoutOptions, LayoutPanel, LayoutPlacement};
pub use plot::{density_grid, point_densities, AxisScale, PlotOverlay, PlotStyle};
pub use raster::{rasterize_svg, save_svg_as_image, ImageFormat};
pub use statistics::{PopulationStat, Statistic};
pub use workspace::{Population, Workspace};
