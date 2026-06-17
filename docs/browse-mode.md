# Browse Mode — Technical Design

`kitdraw` ships with a dual-mode TUI: **Browse** (zoom, pan, rotate)
and **Draw** (annotation toolkit).  This document covers the browse-mode
rendering pipeline, the client-side cache, and the 1:1 sticky-pan formula.

---

## Architecture

```
source image ─→ DrawingCanvas (render) ─→ RgbaImage (canvas-sized)
                                              │
                                    zoom_rotate_to_size()
                                              │
                                   RgbaImage (zoomed, capped)
                                              │
                                       apply_pan()
                                              │
                                   RgbaImage (canvas-sized, cropped)
                                              │
                                    kitty::write_frame()
                                              │
                                      terminal display
```

### Resolution scale

The canvas is rendered at `resolution_scale` (default `0.5`) of the
terminal pixel dimensions.  On a 1920×1080 terminal the canvas is
960×540 px.  The Kitty protocol displays the canvas-sized image
across all available cells, so the terminal upscales the canvas to
full device resolution — browse-mode zooming operates on canvas
pixels and the terminal scales the result for display.

---

## Zoom Cache

Re-encoding the full RGBA frame through zlib + base64 for every
mouse-move during a pan is expensive (50–200 ms).  The cache avoids
re-running the Triangle resize when only the pan offset changed.

```rust
struct BrowseCache {
    zoom: f32,
    rotation: u8,
    zoomed_image: RgbaImage,
}
```

| Operation | Cache hit | Work done |
|-----------|-----------|-----------|
| Pan / drag | ✓ | `apply_pan` only (~0.1 ms) |
| Zoom in/out | ✗ | `zoom_rotate_to_size` + `apply_pan` |
| Rotate | ✗ | `zoom_rotate_to_size` + `apply_pan` |
| Resize | cleared | Full re-render |

---

## 1:1 Sticky-Pan Formula

The user places the cursor on a feature and drags.  The feature must
stay under the cursor throughout the drag, at any zoom level.

### Problem

The existing code normalised the mouse delta to cell columns:

```rust
// OLD — pan speed depends on zoom
pan_x -= delta_col / layout.cols / zoom;
```

This was a heuristic.  At zoom=2.0 the image moved at 0.25× cursor
speed; at zoom=4.0 at 0.375× — not 1:1.

### Derivation

The pan offset in `apply_pan` maps a normalised `pan_x ∈ [-1, 1]` to
a pixel shift in the zoomed image:

```
shift_x = (zw − w) · (pan_x + 1) / 2       [1]
```

where `zw` is the zoomed-image width and `w` the canvas width.

A mouse drag of Δfraction screen widths should move the image by the
same Δfraction of the canvas width on screen.  In canvas-pixel space:

```
Δshift = Δfraction · w                      [2]
```

Differentiating [1]:

```
Δshift = Δpan · (zw − w) / 2                [3]
```

Equating [2] and [3] and substituting `zw = w · zoom`:

```
Δpan = 2 · Δfraction / (zoom − 1)           [4]
```

### Implementation

```rust
// Detect pixel vs cell coordinate mode
let pixel_mode = mouse.column > layout.cols;

// Normalise mouse delta to screen fraction
let fraction = if pixel_mode {
    delta_col / layout.display_width_px
} else {
    delta_col / layout.cols
};

// Apply [4] with clamp near zoom=1.0
let z = view_transform.zoom.max(1.001);
let pan_delta = (2.0 * fraction / (z - 1.0)).clamp(-2.0, 2.0);
pan_x -= pan_delta;
```

At zoom → 1.0 the zoomed image is the same size as the canvas, so
there is nothing to pan.  The formula yields a large Δpan for tiny
screen fractions, but `apply_pan` clamps the actual shift to `[0,
zw−w]` which is near zero — the cursor stays on the feature because
the image barely shifts at all.  Once zoom > 1.05 the tracking is
visually perfect.

### Screen-fraction normalisation

The mouse subsystem enables SGR pixel mode (DECSET 1016).  Terminals
that support it report pixel-accurate coordinates; others fall back
to cell indices.  The mode is detected at runtime:

| Mode | Horizontal | Vertical |
|------|------------|----------|
| Pixel | `delta_col / display_width_px` | `delta_row / display_height_px` |
| Cell  | `delta_col / cols`             | `delta_row / rows`             |

Both normalise to a unit-less screen fraction so [4] is
resolution-independent.

---

## Proportional 4096 → 8192 Cap

`zoom_rotate_to_size` caps the larger zoomed dimension to prevent
OOM on extreme zoom.  The cap is **proportional** so aspect ratio
is preserved.

| Canvas width | Old cap (4096) | New cap (8192) |
|--------------|----------------|----------------|
| 960 (FHD, 0.5×) | zoom 4.27× | zoom 8.53× |
| 2048 (UHD, 0.5×) | zoom 2.00× | zoom 4.00× |

The increase from 4096 → 8192 ensures that at `resolution_scale = 0.5`
the cap is never reached before the software clamp at zoom 4.0× on any
common display size.

---

## Scroll‑wheel Zoom Scaling

| Version | Step | Perceived speed |
|---------|------|-----------------|
| Original | 1.10× | Too fast — one tick jumps 10 % |
| First fix | 1.03× | Too slow — 10 ticks = +34 %, imperceptible per-tick |
| Current | **1.05×** | One tick = 5 %, 10 ticks = +63 % |

---

## Keybindings

| Key | Browse mode |
|-----|-------------|
| `+` / `=` | Zoom in (1.1×) |
| `-` | Zoom out |
| Mouse wheel | Zoom in/out (1.05× per tick) |
| Drag left button | Pan (1:1 sticky) |
| Arrow keys | Pan by 5 % step |
| `r` | Rotate 90° CW |
| `R` | Rotate 90° CCW |
| `0` | Reset zoom & pan |
| `p` | Switch to Draw mode |
| `q` / Esc | Quit (prompts to save if annotated) |
