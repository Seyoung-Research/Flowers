# Flowers - A FlowJo Alternative in Rust

[![Rust](https://img.shields.io/badge/rust-1.99+-orange.svg)](https://rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`Flowers` is an open-source FlowJo alternative for flow cytometry analysis, written in Rust. It provides FCS file parsing, compensation, multi-gate gating (rectangular, elliptical, polygon, quadrant, boolean), population statistics, SVG plotting, and a desktop GUI.

## Features

| Feature | Description |
|---------|-------------|
| **FCS parsing** | FCS 2.0 / 3.0 / 3.1 listmode files (32-bit float and 64-bit double data, little/big-endian) |
| **Compensation** | Spillover-matrix compensation from CSV matrices |
| **Gating** | Rectangular, elliptical, polygonal, quadrant, and boolean (AND/OR/NOT) gates |
| **Drag-to-create gates** | Draw gates directly on the scatter plot in the GUI |
| **Statistics** | Count, %, mean, median, SD, CV, geometric MFI, min, max per population |
| **Visualization** | SVG dot plots and 1D histograms |
| **Desktop GUI** | egui-based application: load files, gate populations, export plots |
| **CLI** | Scriptable command-line interface for batch workflows |

## Installation

```bash
git clone https://github.com/your-username/flowers.git
cd flowers
cargo build --release
```

### Binaries

| Binary | Purpose |
|--------|---------|
| `flowers` | Command-line interface (load, gate, stats, plot, hist, compensate) |
| `flowers-gui` | Desktop GUI application (egui) |

## Quick Start

### GUI

```bash
cargo run --bin flowers-gui --release
```

1. Click **Load FCS File** (or use the **Load FCS** menu).
2. Select **X** and **Y** parameters from the dropdowns (human-readable names like "FITC-H", "PE-A").
3. **Drag on the scatter plot** to create a gate, or click **+ Add Gate**.
4. Adjust gate bounds with the DragValue controls, or switch between Rect/Ellipse shapes.
5. Click **Save Population** to record the gated population.
6. Click **Export SVG** to save a publication-ready dot plot.

### CLI

```bash
# Inspect an FCS file
cargo run -- load data/D1_WT.fcs

# Gate a population and report event counts
cargo run -- gate data/D1_WT.fcs Live rect 0 1 0.0 1000000.0 0.0 1000000.0

# Compute statistics (mean, median, SD, CV, geo-MFI...)
cargo run -- stats data/D1_WT.fcs Live 0 1 0.0 1000000.0 0.0 1000000.0

# Export an SVG dot plot
cargo run -- plot data/D1_WT.fcs plot.svg 0 1

# Export an SVG histogram
cargo run -- hist data/D1_WT.fcs hist.svg 0

# Apply spillover compensation from a CSV matrix
cargo run -- compensate data/D1_WT.fcs matrix.csv
```

## Example Workflow (CLI)

```bash
# 1. Load and inspect
cargo run -- load data/D1_WT.fcs

# 2. Create a "Live" gate on FSC-H vs SSC-H (params 0 and 1)
cargo run -- gate data/D1_WT.fcs Live rect 0 1 0.0 1000000.0 0.0 1000000.0

# 3. Get full statistics for the gated population
cargo run -- stats data/D1_WT.fcs Live 0 1 0.0 1000000.0 0.0 1000000.0

# 4. Visualize
cargo run -- plot data/D1_WT.fcs live_plot.svg 0 1
```

## Project Structure

```
flowers/
├── Cargo.toml          # Manifest & dependencies
├── README.md           # This file
├── LICENSE
├── src/
│   ├── lib.rs          # Library crate root (module exports)
│   ├── main.rs         # CLI binary (`flowers`)
│   ├── gui_main.rs     # GUI binary (`flowers-gui`)
│   ├── fcs.rs          # FCS 2.0/3.0/3.1 parser
│   ├── gating.rs       # Gates: rect, ellipse, polygon, quadrant, boolean
│   ├── compensation.rs # Spillover-matrix compensation
│   ├── statistics.rs   # Population statistics (count, %, mean, SD, CV, geo-MFI)
│   ├── plot.rs         # SVG dot plots & histograms
│   ├── workspace.rs    # Experiment workspace & population trees
│   └── gui.rs          # egui desktop GUI
├── data/               # Example FCS files
│   ├── D1_WT.fcs
│   └── D2_WT + Recom.fcs
├── examples/           # Example programs
│   └── gen_fcs.rs      # Generate synthetic FCS files for testing
└── target/             # Build output (gitignored)
```

## Library Usage

`flowers` is also a library. Add to your `Cargo.toml`:

```toml
[dependencies]
flowers = { path = "../flowers" }
```

```rust
use flowers::fcs::FcsParser;
use flowers::gating::{RectangularGate, Gate};
use flowers::statistics::{compute_profile, print_profile};
use ndarray::array2;

// 1. Parse an FCS file
let fcs = FcsParser::parse_bytes(&fc_bytes)?;

// 2. Gate a population
let gate = RectangularGate::new(0, 1, 0.0, 1000000.0, 0.0, 1000000.0);
let indices = gate.filter(&fcs.data);

// 3. Compute statistics
let param_names: Vec<String> = fcs.parameters.iter().map(|p| p.name.clone()).collect();
let profile = compute_profile("Live", &fcs.data, &indices, &param_names, None, Some(fcs.event_count));
print_profile(&profile);
```

## Supported FCS Features

- Versions: FCS 2.0, 3.0, 3.1
- Data types: `$DATATYPE/F` (32-bit float), `$DATATYPE/D` (64-bit double)
- Byte order: `$BYTEORD/1,2,3,4` (little-endian), `$BYTEORD/4,3,2,1` (big-endian)
- Segment pointers: 0-indexed byte offsets (real-world FCS 3.1 compatible)

## Requirements

- Rust 1.99 or later
- No GPU required for the GUI (egui with software/GL backend)

## Contributing

Contributions are welcome! Please open an issue or pull request.

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/your-feature`)
3. Commit your changes (`git commit -m 'Add some feature'`)
4. Push to the branch (`git push origin feature/your-feature`)
5. Open a Pull Request

## License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.

## Acknowledgments

Inspired by FlowJo (BD) and the open-source flow cytometry ecosystem. Built with [egui](https://github.com/emilk/egui), [ndarray](https://github.com/rust-ndarray/ndarray), [rfd](https://github.com/RustFileDialog/rfd), and [thiserror](https://github.com/dtolnay/thiserror).
