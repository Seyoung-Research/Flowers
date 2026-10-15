use ndarray::{ArrayView1, Axis};

/// A gate selects a subset of events from a flow cytometry sample.
///
/// Every gate implements `contains` (point-in-gate test) and inherits
/// `filter` (returns the indices of all passing events).
pub trait Gate: std::fmt::Debug + Send + Sync {
    /// Returns true if `event` is inside the gate.
    fn contains(&self, event: ArrayView1<f32>) -> bool;

    /// Returns the indices of events that pass the gate.
    fn filter(&self, data: &ndarray::Array2<f32>) -> Vec<usize> {
        let mut indices = Vec::new();
        for (idx, event) in data.axis_iter(Axis(0)).enumerate() {
            if self.contains(event) {
                indices.push(idx);
            }
        }
        indices
    }

    /// The parameter indices (channels) this gate operates on.
    fn parameter_indices(&self) -> Vec<usize>;
}

// ---------------------------------------------------------------------------
// Rectangular gate (axis-aligned box in 2D)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RectangularGate {
    pub param_idx_x: usize,
    pub param_idx_y: usize,
    pub min_x: f32,
    pub max_x: f32,
    pub min_y: f32,
    pub max_y: f32,
}

impl RectangularGate {
    pub fn new(x: usize, y: usize, min_x: f32, max_x: f32, min_y: f32, max_y: f32) -> Self {
        Self {
            param_idx_x: x,
            param_idx_y: y,
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }
}

impl Gate for RectangularGate {
    fn contains(&self, event: ArrayView1<f32>) -> bool {
        let x = event[self.param_idx_x];
        let y = event[self.param_idx_y];
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }
    fn parameter_indices(&self) -> Vec<usize> {
        vec![self.param_idx_x, self.param_idx_y]
    }
}

// ---------------------------------------------------------------------------
// Ellipse gate (2D)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EllipseGate {
    pub param_idx_x: usize,
    pub param_idx_y: usize,
    pub center_x: f32,
    pub center_y: f32,
    pub radius_x: f32,
    pub radius_y: f32,
}

impl EllipseGate {
    pub fn new(x: usize, y: usize, center_x: f32, center_y: f32, radius_x: f32, radius_y: f32) -> Self {
        Self {
            param_idx_x: x,
            param_idx_y: y,
            center_x,
            center_y,
            radius_x,
            radius_y,
        }
    }
}

impl Gate for EllipseGate {
    fn contains(&self, event: ArrayView1<f32>) -> bool {
        let x = event[self.param_idx_x];
        let y = event[self.param_idx_y];
        let dx = (x - self.center_x) / self.radius_x;
        let dy = (y - self.center_y) / self.radius_y;
        (dx * dx) + (dy * dy) <= 1.0
    }
    fn parameter_indices(&self) -> Vec<usize> {
        vec![self.param_idx_x, self.param_idx_y]
    }
}

// ---------------------------------------------------------------------------
// Polygon gate (arbitrary closed polygon in 2D, even-odd rule)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PolygonGate {
    pub param_idx_x: usize,
    pub param_idx_y: usize,
    /// Vertices of the polygon in order.
    pub vertices: Vec<(f32, f32)>,
}

impl PolygonGate {
    pub fn new(x: usize, y: usize, vertices: Vec<(f32, f32)>) -> Self {
        Self {
            param_idx_x: x,
            param_idx_y: y,
            vertices,
        }
    }
}

impl Gate for PolygonGate {
    fn contains(&self, event: ArrayView1<f32>) -> bool {
        let x = event[self.param_idx_x];
        let y = event[self.param_idx_y];
        let verts = &self.vertices;
        if verts.len() < 3 {
            return false;
        }
        let mut inside = false;
        let mut j = verts.len() - 1;
        for i in 0..verts.len() {
            let (xi, yi) = verts[i];
            let (xj, yj) = verts[j];
            if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
    fn parameter_indices(&self) -> Vec<usize> {
        vec![self.param_idx_x, self.param_idx_y]
    }
}

// ---------------------------------------------------------------------------
// Quadrant gate (four quadrants defined by a vertical and horizontal line)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Quadrant {
    UpperLeft,
    UpperRight,
    LowerLeft,
    LowerRight,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QuadrantGate {
    pub param_idx_x: usize,
    pub param_idx_y: usize,
    pub x: f32,
    pub y: f32,
    /// Which of the four quadrants to select.
    pub quadrant: Quadrant,
}

impl QuadrantGate {
    pub fn new(x: usize, y: usize, x_split: f32, y_split: f32, quadrant: Quadrant) -> Self {
        Self {
            param_idx_x: x,
            param_idx_y: y,
            x: x_split,
            y: y_split,
            quadrant,
        }
    }

    /// Returns all four quadrants as separate gates.
    pub fn all(x: usize, y: usize, x_split: f32, y_split: f32) -> [Self; 4] {
        [
            Self::new(x, y, x_split, y_split, Quadrant::UpperLeft),
            Self::new(x, y, x_split, y_split, Quadrant::UpperRight),
            Self::new(x, y, x_split, y_split, Quadrant::LowerLeft),
            Self::new(x, y, x_split, y_split, Quadrant::LowerRight),
        ]
    }
}

impl Gate for QuadrantGate {
    fn contains(&self, event: ArrayView1<f32>) -> bool {
        let xv = event[self.param_idx_x];
        let yv = event[self.param_idx_y];
        match self.quadrant {
            Quadrant::UpperLeft => xv <= self.x && yv >= self.y,
            Quadrant::UpperRight => xv >= self.x && yv >= self.y,
            Quadrant::LowerLeft => xv <= self.x && yv <= self.y,
            Quadrant::LowerRight => xv >= self.x && yv <= self.y,
        }
    }
    fn parameter_indices(&self) -> Vec<usize> {
        vec![self.param_idx_x, self.param_idx_y]
    }
}

// ---------------------------------------------------------------------------
// Boolean gate (compose gates with AND / OR / NOT)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BooleanOp {
    And,
    Or,
    Not,
}

/// Boolean composition of gates. Note: not `Clone`/`PartialEq`/`Serialize`
/// because it holds `Box<dyn Gate>`; serialize concrete gate trees manually.
#[derive(Debug)]
pub enum BooleanGate {
    /// A leaf gate.
    Leaf(Box<dyn Gate>),
    /// Combine two boolean gates.
    Binary {
        op: BooleanOp,
        left: Box<BooleanGate>,
        right: Box<BooleanGate>,
    },
    /// Negate a single gate.
    Unary {
        op: BooleanOp, // must be Not
        child: Box<BooleanGate>,
    },
}

impl BooleanGate {
    pub fn leaf<G: Gate + 'static>(gate: G) -> Self {
        Self::Leaf(Box::new(gate))
    }

    pub fn and(left: BooleanGate, right: BooleanGate) -> Self {
        Self::Binary {
            op: BooleanOp::And,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    pub fn or(left: BooleanGate, right: BooleanGate) -> Self {
        Self::Binary {
            op: BooleanOp::Or,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    pub fn not(child: BooleanGate) -> Self {
        Self::Unary {
            op: BooleanOp::Not,
            child: Box::new(child),
        }
    }
}

impl Gate for BooleanGate {
    fn contains(&self, event: ArrayView1<f32>) -> bool {
        match self {
            BooleanGate::Leaf(g) => g.contains(event),
            BooleanGate::Binary { op, left, right } => {
                let l = left.contains(event);
                let r = right.contains(event);
                match op {
                    BooleanOp::And => l && r,
                    BooleanOp::Or => l || r,
                    BooleanOp::Not => unreachable!("Not handled by Unary variant"),
                }
            }
            BooleanGate::Unary { op, child } => {
                debug_assert!(matches!(op, BooleanOp::Not));
                !child.contains(event)
            }
        }
    }
    fn parameter_indices(&self) -> Vec<usize> {
        match self {
            BooleanGate::Leaf(g) => g.parameter_indices(),
            BooleanGate::Binary { left, right, .. } => {
                let mut idx = left.parameter_indices();
                idx.extend(right.parameter_indices());
                idx
            }
            BooleanGate::Unary { child, .. } => child.parameter_indices(),
        }
    }
}

// ---------------------------------------------------------------------------
// A gated population result: indices + per-parameter values for reporting.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GatedPopulation {
    pub name: String,
    pub event_indices: Vec<usize>,
    pub gate_summary: String,
}

impl GatedPopulation {
    pub fn new(name: &str, indices: Vec<usize>, gate_summary: &str) -> Self {
        Self {
            name: name.to_string(),
            event_indices: indices,
            gate_summary: gate_summary.to_string(),
        }
    }

    pub fn count(&self) -> usize {
        self.event_indices.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    fn sample_data() -> ndarray::Array2<f32> {
        // 6 events, 2 parameters
        array![
            [1.0, 1.0],
            [2.0, 2.0],
            [8.0, 1.0],
            [9.0, 2.0],
            [1.0, 8.0],
            [9.0, 9.0],
        ]
    }

    #[test]
    fn rectangular_gate_filters() {
        let data = sample_data();
        let gate = RectangularGate::new(0, 1, 0.0, 5.0, 0.0, 5.0);
        let idx = gate.filter(&data);
        assert_eq!(idx, vec![0, 1]);
    }

    #[test]
    fn ellipse_gate_filters() {
        let data = sample_data();
        let gate = EllipseGate::new(0, 1, 1.5, 1.5, 1.0, 1.0);
        let idx = gate.filter(&data);
        assert_eq!(idx, vec![0, 1]);
    }

    #[test]
    fn polygon_gate_filters() {
        let data = sample_data();
        let gate = PolygonGate::new(
            0,
            1,
            vec![(0.0, 0.0), (5.0, 0.0), (5.0, 5.0), (0.0, 5.0)],
        );
        let idx = gate.filter(&data);
        assert_eq!(idx, vec![0, 1]);
    }

    #[test]
    fn quadrant_gate_filters() {
        let data = sample_data();
        let quads = QuadrantGate::all(0, 1, 5.0, 5.0);
        let ul = quads[0].filter(&data); // x<=5, y>=5  -> event 4 [1,8]
        let ur = quads[1].filter(&data); // x>=5, y>=5  -> event 5 [9,9]
        let ll = quads[2].filter(&data); // x<=5, y<=5  -> events 0 [1,1], 1 [2,2]
        let lr = quads[3].filter(&data); // x>=5, y<=5  -> events 2 [8,1], 3 [9,2]
        assert_eq!(ul, vec![4]);
        assert_eq!(ur, vec![5]);
        assert_eq!(ll, vec![0, 1]);
        assert_eq!(lr, vec![2, 3]);
    }

    #[test]
    fn boolean_gate_combines() {
        let data = sample_data();
        let r1 = RectangularGate::new(0, 1, 0.0, 5.0, 0.0, 10.0);
        let r2 = RectangularGate::new(0, 1, 0.0, 10.0, 0.0, 5.0);
        let combo = BooleanGate::and(BooleanGate::leaf(r1), BooleanGate::leaf(r2));
        let idx = combo.filter(&data);
        assert_eq!(idx, vec![0, 1]); // x<=5 AND y<=5
    }
}
