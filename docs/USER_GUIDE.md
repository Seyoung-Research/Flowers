# Flowers User Guide

`Flowers` is an open-source FlowJo alternative for flow cytometry analysis,
written in Rust. This guide explains how to use the graphical user interface
(`flowers-gui`) and all of its features.

## Table of Contents

1. [Installation & Launch](#installation--launch)
2. [Loading FCS Files](#loading-fcs-files)
3. [The Plotting Window](#the-plotting-window)
   - [Axes, Scales & Ranges](#axes-scales--ranges)
   - [Dot Colour, Gradient & View](#dot-colour-gradient--view)
   - [Histogram / Counts View](#histogram--counts-view)
4. [Gating](#gating)
   - [Creating Gates](#creating-gates)
   - [Gate Types](#gate-types)
   - [Manipulating Gates](#manipulating-gates)
   - [Polygon Vertices](#polygon-vertices)
   - [Bilinear Gates (Horizontal Threshold)](#bilinear-gates-horizontal-threshold)
5. [Gate Hierarchy & Tree](#gate-hierarchy--tree)
6. [Detached Population Windows](#detached-population-windows)
7. [Themes](#themes)
8. [Apply Gates to All Files](#apply-gates-to-all-files)
9. [Export](#export)
   - [SVG Export](#svg-export)
   - [Raster Image Export (PNG/JPEG/TIFF)](#raster-image-export-pngjpegtiff)
   - [CSV Export](#csv-export)
10. [Layout Editor](#layout-editor)
11. [Command-Line Interface](#command-line-interface)

---

## Installation & Launch

```bash
git clone https://github.com/your-username/flowers.git
cd flowers
cargo build --release
```

Launch the GUI:

```bash
cargo run --bin flowers-gui --release
```

Example FCS files are provided in `data/`:

- `data/D1_WT.fcs`
- `data/D2_WT + Recom.fcs`

---

## Loading FCS Files

1. Click **"Load FCS File"** in the left sidebar, or use **File ▸ Load FCS** from
   the top menu bar.
2. Select one or more `.fcs` files (FCS 2.0 / 3.0 / 3.1).
3. After loading, select **X** and **Y** parameters from the dropdowns in the
   left sidebar. Human-readable names (e.g. "FITC-H", "PE-A") are shown where
   available.

The scatter plot appears in the central panel once X and Y are selected.

---

## The Plotting Window

### Axes, Scales & Ranges

Open **Axis options** in the left sidebar:

- **X scale / Y scale**: Choose **Linear** or **Log** (base-10).
- **X range / Y range**: Drag the min/max values to set the visible window.
- **Auto**: Click **Auto** next to an axis to fit it to the data.

Axis tick labels use scientific notation for large values (e.g. `7e4` instead of
`70000`) so they do not collide with the axis title.

### Dot Colour, Gradient & View

Open **Point & view options** in the left sidebar:

- **Dot colour**: Click the colour swatch to pick a colour.
- **Density gradient layering** (checked by default): colours points by local
  density (blue → cyan → yellow → red) where points overlap.
- **View**: Choose **Dots**, **Contour**, or **Histogram**.
  - **Contour**: topographic contour lines instead of individual points.
  - **Histogram**: 1D histogram of the X-axis channel, with counts on the Y axis.

### Histogram / Counts View

Select **Histogram** from the **View** options. The plot shows a histogram of
the currently selected X-axis channel:

- X-axis: the selected parameter.
- Y-axis: event count per bin.
- 50 bins span the X-axis range.

This view is available in both the main window and detached population windows.

---

## Gating

### Creating Gates

1. Choose a **New gate type** from the radio buttons in the left sidebar:
   Rectangle, Ellipse, Polygon, Quadrant, or Bilinear.
2. **Drag on the scatter plot** to draw the gate.
3. For **Quadrant** and **Bilinear** gates, you can also **click** on the plot to
   place the gate at that position.
4. Click **+ Add Gate** to create a gate at the centre of the current view.
5. Gates appear in the **Gate tree** on the right side. Select a gate to edit
   its name, colour, geometry, and parent.

### Gate Types

| Gate Type | Description |
|-----------|-------------|
| **Rectangle** | Axis-aligned box (can be rotated). |
| **Ellipse** | Oval gate (can be rotated and stretched per axis). |
| **Polygon** | Free-form polygon with 3+ vertices. |
| **Quadrant** | Four-way split by vertical and horizontal lines. |
| **Bilinear** | Horizontal threshold line creating "Below" and "Above" gates. |

### Manipulating Gates

After drawing a gate, select it in the gate tree or click on it in the plot.
You can manipulate it in two ways:

**Drag handles on the plot** (shown when a gate is selected):

- **Corner handles** (white squares on rectangles; on the ellipse edge for ellipses; vertices for polygons): drag to resize/scale.
- **Edge handles** (ellipses only, at left/top/right/bottom): drag to stretch one axis.
- **Diamond handle** (rotate): drag to rotate about the centre.
- **Cross handle** (centre, bilinear only): drag to move the split point.
- **Body**: drag anywhere inside the gate to move it.

The mouse cursor changes to indicate the handle type (resize arrows, grab hand,
etc.) when hovering over a handle.

**Transform buttons** (in the gate editor at the bottom):

- ⬅ ➡ ⬆ ⬇: Move left / right / up / down.
- ⟲ ⟳: Rotate -5° / +5°.
- ➕ ➖: Enlarge / shrink (scale about centre).

### Polygon Vertices

For polygon gates, you can add or remove vertices:

- **Drag a vertex handle** on the plot to reshape the polygon.
- **+ Add vertex**: inserts a new vertex at the midpoint of the longest edge.
- **− Remove vertex**: removes the last vertex (minimum 3 vertices enforced).

### Bilinear Gates (Horizontal Threshold)

Bilinear gates now draw a **horizontal threshold line**. Dragging horizontally
creates two gates:

- **Below**: events with Y value below the line.
- **Above**: events with Y value above the line.

Clicking on the plot places the horizontal line at that Y position. You can
rotate or move the threshold line using the handles or transform buttons.

---

## Gate Hierarchy & Tree

Gates are organised in a tree (like FlowJo) shown in the right panel:

- Each gate can have a **parent** gate. Child gates are applied to the parent's
  population (hierarchical gating).
- Select a gate in the tree to view and edit its details in the bottom panel.
- Use the **Parent** dropdown to re-parent a gate (only earlier gates are
  offered, keeping the hierarchy acyclic).
- Click **◧** to open the gate's population in a detached window.
- Click **✖** to delete a gate (children are re-parented to the deleted gate's
  parent).

Counts and percentages are shown for each gate relative to its parent.

---

## Detached Population Windows

1. Select a gate in the tree.
2. Click **Open detached window** (or click **◧** next to the gate in the tree).
3. A new window opens showing only that population's events.

The detached window has the full plotting and gating toolkit:

- X/Y parameter selection
- Linear/log scales and editable ranges
- Auto range
- Dot colour and gradient
- Dots / Contour / Histogram view
- Gate drawing and manipulation on the population
- All transform buttons

Close the window to return to the main view.

---

## Themes

Switch between **Dark** and **Light** themes:

- **Settings ▸ Theme** from the top menu bar, or
- The theme selector that was previously in the top bar.

The entire application (plot, panels, text) updates immediately.

---

## Apply Gates to All Files

By default, gates apply to **all loaded FCS files**. You can control this per
gate:

1. Select a gate in the tree.
2. In the bottom gate editor, toggle **"Apply to all samples"**.
   - **Checked** (default): the gate is evaluated for every loaded sample.
   - **Unchecked**: the gate applies only to the sample it was created on
     (shown as the "owner sample" next to the checkbox).
3. The cross-sample table and CSV export only include gates that apply to each
   sample.

---

## Export

### SVG Export

- **File ▸ Export SVG**: exports the current plot (with gates) as SVG.
- In the Layout Editor: click **SVG…** to export the composed layout.

### Raster Image Export (PNG / JPEG / TIFF)

- **File ▸ Export Image…**: exports the current plot as PNG, JPEG, or TIFF.
- Set the **scale** (1×–4×) for supersampling in the export dialog.
- In the Layout Editor: click **Image…** to export the composed layout.

### CSV Export

- **File ▸ Export CSV**: exports per-population counts and percentages across
  all loaded samples.
- Columns: Sample, Population, Parent, Count, Total, PercentOfTotal,
  PercentOfParent, GateSummary.
- Also available from the right panel: **Populations across samples ▸ Export CSV…**.

---

## Layout Editor

The layout editor lets you arrange plots on a free-form canvas (like
Photoshop) before exporting a multi-panel figure.

1. Click **Layout Editor** from the top menu bar.
2. **Add current plot**: adds the main window's plot as a panel.
3. **Add detached plot**: adds the current detached population window's view as
   a panel (only available when a detached window is open).
4. Drag panels to move them; drag the corner handle to resize.
5. Use **Tile** to arrange panels in a grid, **Bring to front** to reorder, and
   **Delete** to remove a panel.
6. Export as **SVG…** or **Image…** (PNG/JPEG/TIFF).

Each panel shows:
- The plot title (Y-axis parameter name).
- X and Y axis labels.
- A preview of the data points.

---

## Command-Line Interface

`flowers` also provides a CLI for batch workflows:

```bash
# Inspect an FCS file
cargo run -- load data/D1_WT.fcs

# Gate a population and report event counts
cargo run -- gate data/D1_WT.fcs Live rect 0 1 0.0 1000000.0 0.0 1000000.0
cargo run -- gate data/D1_WT.fcs Wedge bilinear 0 1 0.0 1000000.0 0.0 1000000.0

# Compute statistics
cargo run -- stats data/D1_WT.fcs Live 0 1 0.0 1000000.0 0.0 1000000.0

# Export an SVG dot plot
cargo run -- plot data/D1_WT.fcs plot.svg 0 1

# Export an SVG histogram
cargo run -- hist data/D1_WT.fcs hist.svg 0

# Apply spillover compensation
cargo run -- compensate data/D1_WT.fcs matrix.csv
```

CLI subcommands: `load`, `gate`, `stats`, `plot`, `hist`, `compensate`.

---

## Project Structure

```
flowers/
├── Cargo.toml          # Manifest & dependencies
├── README.md           # Project overview
├── FEATURES.md         # Feature checklist
├── docs/
│   └── USER_GUIDE.md   # This file
├── src/
│   ├── main.rs         # CLI binary
│   ├── gui_main.rs     # GUI binary entry point
│   ├── gui.rs          # egui desktop GUI
│   ├── fcs.rs          # FCS parser
│   ├── gating.rs       # Gate implementations
│   ├── plot.rs         # SVG plots, density, contours
│   ├── layout.rs       # Layout editor composition
│   ├── raster.rs       # SVG → PNG/JPEG/TIFF rasterisation
│   ├── export.rs       # CSV reports
│   ├── statistics.rs   # Population statistics
│   ├── compensation.rs # Spillover compensation
│   └── workspace.rs    # Experiment workspace
├── data/               # Example FCS files
└── examples/           # Example programs
```

---

## Requirements

- Rust 1.99 or later
- No GPU required (egui with software rendering)
- Raster export uses `resvg` and `image`; on Linux, installing `fontconfig`
  and a system font (e.g. DejaVu) improves text rendering in raster exports.
