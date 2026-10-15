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
pub mod fcs;
pub mod gating;
pub mod gui;
pub mod plot;
pub mod statistics;
pub mod workspace;

pub use compensation::CompensationEngine;
pub use fcs::{FcsError, FcsFile, FcsParser, Parameter};
pub use gating::{BooleanGate, EllipseGate, Gate, PolygonGate, QuadrantGate, RectangularGate};
pub use statistics::{PopulationStat, Statistic};
pub use workspace::{Population, Workspace};
