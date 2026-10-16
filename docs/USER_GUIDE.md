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
   - [Bilinear / Line Gate (Region Under a Line)](#bilinear--line-gate-region-under-a-line)
5. [Gate Hierarchy & Tree](#gate-hierarchy--tree)
6. [Detached Population Windows](#detached-population-windows)
7. [Themes](#themes)
8. [Undo / Redo](#undo--redo)
9. [Applying Gates Across Files](#applying-gates-across-files)
10. [Export](#export)
    - [SVG Export](#svg-export)
    - [Raster Image Export (PNG/JPEG/TIFF)](#raster-image-export-pngjpegtiff)
    - [CSV Export](#csv-export)
11. [Layout Editor](#layout-editor)
    - [Axis Label Styling](#axis-label-styling)
12. [Workspaces](#workspaces)
13. [Command-Line Interface](#command-line-interface)

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
3. When a file loads, the X and Y axes are chosen automatically:
   - **X = FSC-H**, **Y = SSC-H** when those channels are present.
   - Otherwise the first FSC/SSC channel (FSC-A/SSC-A) is used, falling back to
     the first two parameters.
4. You can override the axes at any time with the **X** and **Y** dropdowns in
   the left sidebar. Human-readable names (e.g. "FITC-H", "PE-A") are shown where
   available.

The scatter plot appears in the central panel as soon as a file is loaded.

If gates already exist when you load a new file, a prompt asks whether to apply
the current gating strategy to that file — see
[Applying Gates Across Files](#applying-gates-across-files).

---

## The Plotting Window

### Axes, Scales & Ranges

Open **Axis options** in the left sidebar:

- **X scale / Y scale**: Choose **Linear** or **Log** (base-10).
- **X range / Y range**: Set the visible window.
  - Large values are shown in **scientific notation** (e.g. `5.300871e6`) so
    10⁷-scale numbers stay short and readable.
  - **Drag** a value to change it — the drag speed scales with the axis width,
    so a 10⁷-wide axis can be swept in a single drag instead of one unit per
    pixel.
  - **Click** a value to type an exact number. Both plain (`250000`) and
    scientific (`2.5e5`, `1e7`) input are accepted.
- **Auto**: Click **Auto** next to an axis to fit it to the data.

Axis tick labels use compact scientific notation for large values (e.g. `7e4`
instead of `70000`) so they do not collide with the axis title.

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
   - For **Bilinear**, the drag *is* the line: the gate is the region under the
     line you draw (see below).
3. For **Quadrant** and **Bilinear** gates you can also **click** on the plot to
   place the gate at that position (a quadrant crosshair, or a horizontal line).
4. Click **+ Add Gate** to create a gate at the centre of the current view.
5. Gates appear in the **Gate tree** on the right side. Select a gate to edit
   its name, colour, geometry, and parent.

While dragging, a preview shows the shape being drawn (a rectangle, ellipse,
polygon, or — for Bilinear — the dividing line).

### Gate Types

| Gate Type | Description |
|-----------|-------------|
| **Rectangle** | Axis-aligned box (can be rotated). |
| **Ellipse** | Oval gate (can be rotated and stretched per axis). |
| **Polygon** | Free-form polygon with 3+ vertices. |
| **Quadrant** | Four-way split by vertical and horizontal lines. |
| **Bilinear** | Line gate: selects the events **under** a line you draw, within the line's width. |

### Manipulating Gates

After drawing a gate, select it in the gate tree or click on it in the plot.
You can manipulate it in two ways:

**Drag handles on the plot** (shown when a gate is selected):

- **Corner handles** (white squares on rectangles; on the ellipse edge for ellipses; vertices for polygons): drag to resize/scale.
- **Edge handles** (ellipses only, at left/top/right/bottom): drag to stretch one axis.
- **Diamond handle** (rotate): drag to rotate about the centre.
- **Centre handle** (quadrant and bilinear): drag to move the split point / line.
- **Line-end handles** (bilinear): drag either endpoint to reshape the line.
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

### Bilinear / Line Gate (Region Under a Line)

The bilinear gate is a **line gate**. Instead of splitting the whole window, a
single gate is created from the line you draw:

- **Drag** across the plot to draw a line. The gate is the region **under** that
  line — a shape whose top edge follows the line, whose width matches the
  horizontal extent of the line you drew, and whose sides drop down to the
  bottom of the Y axis (so it looks like a full-height rectangle when the line
  is horizontal).
- **Click** on the plot to place a horizontal threshold line at that Y value.
- **+ Add Gate** places a horizontal threshold across the current view.

Because the gate hugs the drawn line, a diagonal line selects a diagonal band of
events. Draw a nearly-vertical line to make a left/right split (the two sides are
then treated as a threshold at the line's midpoint).

In the gate editor you can:

- Edit the line's **Start X/Y** and **End X/Y** numerically.
- Toggle **"Select points below the line"** — uncheck it to select the region
  **above** the line instead.

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
- An **Add this plot to layout** button that sends the detached view straight
  into the [Layout Editor](#layout-editor)

Close the window to return to the main view.

---

## Themes

Switch between **Dark** and **Light** themes:

- **Settings ▸ Theme** from the top menu bar, or
- The theme selector that was previously in the top bar.

The entire application (plot, panels, text) updates immediately.

---

## Undo / Redo

Every edit to the analysis can be undone and redone:

- **Undo**: `Ctrl + Z` (⌘Z on macOS), or **Edit ▸ Undo**.
- **Redo**: `Ctrl + Shift + Z` or `Ctrl + Y` (⌘⇧Z / ⌘Y on macOS), or
  **Edit ▸ Redo**.

Undo covers the **gating strategy**, the **view settings** (axes, scales,
ranges, colours, view mode) and the **layout editor** contents. Loaded FCS files
are inputs and are *not* undone — undoing never unloads a file.

A whole drag (drawing a gate, moving a handle, dragging an axis value) collapses
into a **single** undo step, so one `Ctrl + Z` reverses the entire gesture. The
history keeps the most recent 200 steps.

---

## Applying Gates Across Files

Gates can be shared across every file in your experiment, but **you decide** how
they are applied — nothing is forced.

### The per-file prompt

When you load a new FCS file and gates already exist, a prompt appears:

> **Apply the existing N gate(s) to this file?**

- **Apply gates** — the current gating strategy is evaluated for the new file.
- **Don't apply** — the new file keeps its own (empty) gate set for now.

**How a gate is applied.** A gate is always evaluated for the sample it was
*created* on. For any *other* sample it is evaluated only when the gate's
**"Apply to all samples"** checkbox is on *and* that sample has gating enabled.

### The per-sample checkbox

Every entry in the **Loaded samples** list has a small **"gates"** checkbox:

- **Checked**: the shared gate set applies to this file.
- **Unchecked**: shared gates are skipped for this file (gates created on it
  still apply).

Toggle it at any time to add or remove a file from the gating strategy. Hover
the checkbox for a reminder of what it does.

### The per-gate checkbox

For fine control, select a gate in the tree and use **"Apply to all samples"** in
the gate editor:

- **Checked** (default): the gate is evaluated for every file whose **gates**
  checkbox is on.
- **Unchecked**: the gate applies only to the sample it was created on (shown as
  the "owner sample" next to the checkbox).

The cross-sample table and CSV export respect both switches.

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
2. Add panels:
   - **Add main plot**: adds the main window's plot as a panel.
   - **Add detached plot**: adds the open detached population window's view.
   - **Add this plot to layout** (inside a detached window): adds that
     population's view without leaving the window.
3. Drag panels to move them; drag the corner handle to resize.
4. Use **Tile** to arrange panels in a grid, **Bring to front** to reorder, and
   **Delete** to remove a panel. **Columns** controls how many columns **Tile**
   uses.
5. Export as **SVG…** or **Image…** (PNG/JPEG/TIFF).

Each panel shows:

- The plot title.
- X and Y axis labels.
- A preview of the data points.

**Export notes.** Panels are written as inlined content inside a single SVG
document (rather than nested `<svg>` viewports), which keeps every panel visible
across SVG viewers. Panel titles carry an explicit font family, so text in
PNG/JPEG/TIFF exports is rendered in a normal font instead of a substituted one.

### Axis Label Styling

Select a panel, then use the **Axis label style** controls below the canvas to
style its X and Y labels independently:

| Control | Effect |
|---------|--------|
| **X text / Y text** | Replace the label text. Leave blank to use the parameter name. |
| **size** | Font size of the label. |
| **rotate** | Rotation of the label, in degrees (−180…180). The Y label defaults to −90°. |
| **font** | Font family: **Sans**, **Serif**, or **Mono**. |
| **colour swatch** | Label colour. |

The canvas preview updates immediately so you can see the result before
exporting. The same styling is applied to the exported SVG and raster images.

The `(linear)` / `(log10)` scale suffix is kept on each label; only the parameter
name portion is replaced by your custom text.

---

## Workspaces

A **workspace** (`.wsp`) saves your analysis so you can pick it up later. It
stores the list of FCS file paths plus the gating strategy, view settings and
layout editor contents — but **not** the raw event data (FCS files are large and
usually live next to the workspace). The file is human-readable JSON.

Use the **File** menu:

| Item | Effect |
|------|--------|
| **New Workspace** | Clears all samples, gates and layout, and resets the view. The theme is kept. |
| **Load Workspace…** | Opens a `.wsp` file and re-reads the referenced FCS files. |
| **Save Workspace…** | Writes the current analysis to a `.wsp` file. |

**Loading** restores the file list, gates (including hierarchy, colours and the
per-gate "apply to all" flag), per-file "gates" checkboxes, axes/scales/ranges,
dot colour, view mode, and every layout panel with its axis-label styling. If a
referenced FCS file has moved or been deleted, it is skipped and reported in the
status line; the rest of the workspace still loads.

After loading, undo/redo starts fresh from the loaded state.

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
│   ├── project.rs      # Workspace (.wsp) file format
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
