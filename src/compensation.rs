use ndarray::Array2;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompensationError {
    #[error("Matrix dimensions mismatch: expected {expected}, found {found}")]
    DimensionMismatch { expected: String, found: String },
}

pub struct CompensationEngine {
    /// The compensation matrix (spillover matrix)
    /// Rows = Original channels, Cols = Corrected channels
    pub matrix: Array2<f32>,
}

impl CompensationEngine {
    pub fn new(matrix: Array2<f32>) -> Self {
        Self { matrix }
    }

    /// Applies compensation to a dataset.
    /// data: (events x channels)
    /// result: (events x channels)
    pub fn compensate(&self, data: &Array2<f32>) -> Result<Array2<f32>, CompensationError> {
        let (_events, channels) = data.dim();
        let (mat_rows, _mat_cols) = self.matrix.dim();

        if channels != mat_rows {
            return Err(CompensationError::DimensionMismatch {
                expected: format!("{} channels", mat_rows),
                found: format!("{} channels", channels),
            });
        }

        // Perform matrix multiplication: [Events x Channels] * [Channels x Channels]
        // ndarray's dot product is efficient
        Ok(data.dot(&self.matrix))
    }
}
