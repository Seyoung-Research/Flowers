use clap::{Parser, Subcommand};
use flowers::{
    compensation::CompensationEngine,
    fcs::FcsParser,
    gating::{BilinearGate, EllipseGate, Gate, Quadrant, QuadrantGate, RectangularGate},
    plot::{dot_plot_svg, histogram_svg, PlotStyle},
    statistics::{compute_profile, print_profile},
};
use std::fs;
use std::path::PathBuf;

/// `flowers` — an open-source FlowJo alternative for flow cytometry analysis.
#[derive(Parser)]
#[command(name = "flowers")]
#[command(version = "0.2.0")]
#[command(about = "Load FCS files, gate populations, compute statistics, and export plots.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse an FCS file and print a summary.
    Load {
        /// Path to the FCS file.
        fcs: PathBuf,
    },
    /// Apply a gate to an FCS file and report the number of events passing.
    Gate {
        /// Path to the FCS file.
        fcs: PathBuf,
        /// Name for the resulting population.
        population: String,
        /// Gate type: rect, ellipse, quadrant, bilinear.
        #[arg(value_enum)]
        gate: GateType,
        /// X parameter index.
        x: usize,
        /// Y parameter index.
        y: usize,
        /// X lower bound / split.
        min_x: f32,
        /// X upper bound / split.
        max_x: f32,
        /// Y lower bound.
        min_y: f32,
        /// Y upper bound.
        max_y: f32,
    },
    /// Gate a population and print its statistics (mean, median, SD, CV, geo-MFI...).
    Stats {
        /// Path to the FCS file.
        fcs: PathBuf,
        /// Name for the population.
        population: String,
        /// X parameter index.
        x: usize,
        /// Y parameter index.
        y: usize,
        /// X lower bound / split.
        min_x: f32,
        /// X upper bound / split.
        max_x: f32,
        /// Y lower bound.
        min_y: f32,
        /// Y upper bound.
        max_y: f32,
    },
    /// Render a 2D dot plot (SVG) of two parameters.
    Plot {
        /// Path to the FCS file.
        fcs: PathBuf,
        /// Output SVG path.
        out: PathBuf,
        /// X parameter index.
        x: usize,
        /// Y parameter index.
        y: usize,
    },
    /// Render a 1D histogram (SVG) of one parameter.
    Hist {
        /// Path to the FCS file.
        fcs: PathBuf,
        /// Output SVG path.
        out: PathBuf,
        /// Parameter index.
        p: usize,
    },
    /// Apply compensation (spillover correction) from a CSV matrix and print summary.
    Compensate {
        /// Path to the FCS file.
        fcs: PathBuf,
        /// Path to a CSV spillover matrix (rows = original channels).
        matrix: PathBuf,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum GateType {
    Rect,
    Ellipse,
    Quadrant,
    Bilinear,
}

fn main() {
    env_logger::init();
    let cli = Cli::parse();

    if let Err(e) = run(cli.command) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run(command: Commands) -> Result<(), anyhow::Error> {
    match command {
        Commands::Load { fcs } => {
            let bytes = fs::read(&fcs)?;
            let fcs_file = FcsParser::parse_bytes(&bytes)?;
            println!("FCS file: {}", fcs.display());
            println!("  version : {}", fcs_file.version);
            println!("  events  : {}", fcs_file.event_count);
            println!("  params  : {}", fcs_file.parameters.len());
            for p in &fcs_file.parameters {
                println!(
                    "    {} ({}) range={:.1} exp={:.1}",
                    p.name, p.description, p.range, p.exponent
                );
            }
            let (_, n_ch) = fcs_file.data.dim();
            println!("  data matrix: {} x {}", fcs_file.event_count, n_ch);
        }

        Commands::Gate {
            fcs,
            population,
            gate,
            x,
            y,
            min_x,
            max_x,
            min_y,
            max_y,
        } => {
            let fcs_file = load_fcs(&fcs)?;
            let gate: Box<dyn Gate> = match gate {
                GateType::Rect => Box::new(RectangularGate::new(x, y, min_x, max_x, min_y, max_y)),
                GateType::Ellipse => {
                    let cx = (min_x + max_x) / 2.0;
                    let cy = (min_y + max_y) / 2.0;
                    let rx = (max_x - min_x).abs() / 2.0;
                    let ry = (max_y - min_y).abs() / 2.0;
                    Box::new(EllipseGate::new(x, y, cx, cy, rx, ry))
                }
                GateType::Quadrant => {
                    Box::new(QuadrantGate::new(x, y, min_x, min_y, Quadrant::UpperRight))
                }
                GateType::Bilinear => {
                    let cx = (min_x + max_x) / 2.0;
                    let cy = (min_y + max_y) / 2.0;
                    let radius = ((max_x - min_x).abs().max((max_y - min_y).abs())) / 2.0;
                    Box::new(BilinearGate::new(
                        x,
                        y,
                        cx,
                        cy,
                        std::f32::consts::FRAC_PI_4,
                        3.0 * std::f32::consts::FRAC_PI_4,
                        radius,
                        true,
                        true,
                    ))
                }
            };
            let indices = gate.filter(&fcs_file.data);
            println!(
                "Gate '{}' on {}: {} / {} events pass ({:.1}%)",
                population,
                fcs.display(),
                indices.len(),
                fcs_file.event_count,
                (indices.len() as f64 / fcs_file.event_count as f64) * 100.0,
            );
        }

        Commands::Stats {
            fcs,
            population,
            x,
            y,
            min_x,
            max_x,
            min_y,
            max_y,
        } => {
            let fcs_file = load_fcs(&fcs)?;
            let gate = RectangularGate::new(x, y, min_x, max_x, min_y, max_y);
            let indices = gate.filter(&fcs_file.data);
            let param_names: Vec<String> = fcs_file
                .parameters
                .iter()
                .map(|p| p.name.clone())
                .collect();
            let profile = compute_profile(
                &population,
                &fcs_file.data,
                &indices,
                &param_names,
                None,
                Some(fcs_file.event_count),
            );
            print_profile(&profile);
        }

        Commands::Plot { fcs, out, x, y } => {
            let fcs_file = load_fcs(&fcs)?;
            let x_name = param_name(&fcs_file, x);
            let y_name = param_name(&fcs_file, y);
            let svg = dot_plot_svg(
                &fcs_file.data,
                x,
                y,
                &x_name,
                &y_name,
                &[],
                None,
                None,
                &[],
                &PlotStyle::default(),
            );
            fs::write(&out, &svg)?;
            println!(
                "Wrote dot plot {} ({} events) -> {}",
                out.display(),
                fcs_file.event_count,
                x_name
            );
        }

        Commands::Hist { fcs, out, p } => {
            let fcs_file = load_fcs(&fcs)?;
            let name = param_name(&fcs_file, p);
            let svg = histogram_svg(
                &fcs_file.data,
                p,
                &name,
                "Count",
                &[],
                None,
                30,
                &PlotStyle::default(),
            );
            fs::write(&out, &svg)?;
            println!("Wrote histogram {} ({} events) -> {}", out.display(), fcs_file.event_count, name);
        }

        Commands::Compensate { fcs, matrix } => {
            let fcs_file = load_fcs(&fcs)?;
            let matrix_text = fs::read_to_string(&matrix)?;
            let comp_matrix = parse_csv_matrix(&matrix_text)?;
            let engine = CompensationEngine::new(comp_matrix);
            let compensated = engine.compensate(&fcs_file.data)?;
            let (events, channels) = compensated.dim();
            println!(
                "Compensation applied: {} events x {} channels",
                events, channels
            );
            // Print corrected mean of first channel as a sanity check.
            let col: Vec<f32> = compensated.column(0).iter().copied().collect();
            let mean: f32 = col.iter().sum::<f32>() / col.len() as f32;
            println!("  corrected mean channel 0: {mean:.4}");
        }
    }
    Ok(())
}

fn load_fcs(path: &PathBuf) -> Result<flowers::fcs::FcsFile, anyhow::Error> {
    let bytes = fs::read(path)?;
    Ok(FcsParser::parse_bytes(&bytes)?)
}

fn param_name(fcs: &flowers::fcs::FcsFile, idx: usize) -> String {
    fcs.parameters
        .get(idx)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| format!("P{idx}"))
}

/// Parse a CSV matrix of f32 values, whitespace/trim tolerant.
fn parse_csv_matrix(text: &str) -> Result<ndarray::Array2<f32>, anyhow::Error> {
    let rows: Vec<Vec<f32>> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            l.split(',')
                .map(|s| s.trim().parse::<f32>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow::anyhow!("bad matrix value: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        anyhow::bail!("matrix is empty");
    }
    let ncols = rows[0].len();
    for r in &rows {
        if r.len() != ncols {
            anyhow::bail!("ragged matrix: expected {} columns", ncols);
        }
    }

    let nrows = rows.len();
    let flat: Vec<f32> = rows.into_iter().flatten().collect();
    let matrix = array2_from_flat(&flat, nrows, ncols)?;
    Ok(matrix)
}

/// Build an ndarray::Array2 from a flat row-major slice.
fn array2_from_flat(flat: &[f32], nrows: usize, ncols: usize) -> Result<ndarray::Array2<f32>, anyhow::Error> {
    if flat.len() != nrows * ncols {
        anyhow::bail!("matrix size mismatch: {} values for {}x{}", flat.len(), nrows, ncols);
    }
    // ndarray 0.15: construct from shape + iterator.
    Ok(ndarray::Array2::from_shape_vec((nrows, ncols), flat.to_vec())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_csv_matrix_works() {
        let m = parse_csv_matrix("1,0,0\n0,1,0\n0,0,1").unwrap();
        assert_eq!(m.dim(), (3, 3));
        assert!((m[[0, 0]] - 1.0).abs() < f32::EPSILON);
    }
}
