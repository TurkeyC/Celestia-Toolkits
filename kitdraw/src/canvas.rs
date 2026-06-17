use std::sync::OnceLock;

use ab_glyph::{Font, FontArc, Glyph, PxScale, ScaleFont, point};
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage, imageops::FilterType};

use crate::{terminal::TerminalMetrics, theme::ThemeMode};

#[derive(Debug, Clone)]
pub struct DrawingCanvas {
    metrics: TerminalMetrics,
    source: BaseSource,
    base: RgbaImage,
    fit_rect: FitRect,
    committed: RgbaImage,
    elements: Vec<DrawElement>,
    current: Option<DrawElement>,
    theme: ThemeMode,
}

#[derive(Debug, Clone)]
pub enum BaseSource {
    Blank,
    Image(DynamicImage),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingTool {
    Freehand,
    Rectangle,
    Ellipse,
    Arrow,
    Text,
    Highlighter,
    Redaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthPreset {
    Small,
    Medium,
    Large,
}

impl WidthPreset {
    pub fn previous(self) -> Self {
        match self {
            Self::Small => Self::Large,
            Self::Medium => Self::Small,
            Self::Large => Self::Medium,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Small => Self::Medium,
            Self::Medium => Self::Large,
            Self::Large => Self::Small,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    fn stroke_scale(self) -> f32 {
        match self {
            Self::Small => 0.65,
            Self::Medium => 1.0,
            Self::Large => 1.7,
        }
    }

    fn text_scale(self) -> f32 {
        match self {
            Self::Small => 0.85,
            Self::Medium => 1.1,
            Self::Large => 1.55,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawStyle {
    pub color: Rgba<u8>,
    pub width: WidthPreset,
    pub opacity: f32,
}

impl DrawStyle {
    pub fn new(color: Rgba<u8>, width: WidthPreset) -> Self {
        Self {
            color,
            width,
            opacity: 1.0,
        }
    }

    pub fn highlighter(color: Rgba<u8>, width: WidthPreset) -> Self {
        Self {
            color,
            width,
            opacity: 0.38,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ViewTransform {
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub rotation: u8,
}

impl ViewTransform {
    pub fn identity() -> Self {
        Self {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            rotation: 0,
        }
    }
}

/// Apply zoom and rotation to an image, returning the zoomed+rotated result.
/// The output dimensions are computed from the input and the zoom factor,
/// capped at 4096 pixels per side to prevent memory explosion.
///
/// When `rotation` is 0 this is a pure Triangle resize of the input.
/// When non-zero, the image is rotated 90°/180°/270° before resizing.
pub fn zoom_rotate_to_size(image: &RgbaImage, zoom: f32, rotation: u8) -> RgbaImage {
    let (w, h) = image.dimensions();
    let (rot_w, rot_h) = if rotation % 2 == 0 {
        (w, h)
    } else {
        (h, w)
    };
    let mut zoomed_w = ((rot_w as f32) * zoom).ceil().max(1.0) as u32;
    let mut zoomed_h = ((rot_h as f32) * zoom).ceil().max(1.0) as u32;

    // Cap zoomed dimensions to prevent massive memory allocation when zoom
    // is high. 8192 px per side is enough to allow meaningful zoom on 4K
    // displays at resolution_scale=0.5 (canvas width ~2048px, zoom up to 4x
    // stays at 8192px).
    // We cap the *larger* side and scale the other proportionally so the
    // aspect ratio is preserved.
    const MAX_ZOOMED_DIM: u32 = 8192;
    let max_dim = zoomed_w.max(zoomed_h);
    if max_dim > MAX_ZOOMED_DIM {
        let scale = MAX_ZOOMED_DIM as f32 / max_dim as f32;
        zoomed_w = (zoomed_w as f32 * scale).round().max(1.0) as u32;
        zoomed_h = (zoomed_h as f32 * scale).round().max(1.0) as u32;
    }

    if rotation == 0 {
        image::imageops::resize(image, zoomed_w, zoomed_h, image::imageops::FilterType::Triangle)
    } else {
        let rotated = match rotation {
            1 => image::imageops::rotate90(image),
            2 => image::imageops::rotate180(image),
            3 => image::imageops::rotate270(image),
            _ => unreachable!(),
        };
        image::imageops::resize(&rotated, zoomed_w, zoomed_h, image::imageops::FilterType::Triangle)
    }
}

/// Apply pan and letterboxing to a pre-zoomed image, producing an output of the
/// given `canvas_size`.  The zoomed image is cropped according to the pan offset
/// and centered on a black letterbox canvas.
///
/// `pan_x`/`pan_y` in `[-1, 1]` map to the full shift range:
///    0   → centred
///    1   → right/bottom edge visible
///   -1   → left/top edge visible
pub fn apply_pan(zoomed: &RgbaImage, pan_x: f32, pan_y: f32, canvas_size: (u32, u32)) -> RgbaImage {
    let (zw, zh) = zoomed.dimensions();
    let (w, h) = canvas_size;

    let shift_x = ((zw as f32 - w as f32) * (pan_x + 1.0) / 2.0)
        .clamp(0.0, (zw as f32 - w as f32).max(0.0));
    let shift_y = ((zh as f32 - h as f32) * (pan_y + 1.0) / 2.0)
        .clamp(0.0, (zh as f32 - h as f32).max(0.0));

    let crop_x = shift_x.round() as u32;
    let crop_y = shift_y.round() as u32;
    let crop_w = w.min(zw.saturating_sub(crop_x));
    let crop_h = h.min(zh.saturating_sub(crop_y));

    let mut result = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 255]));

    if crop_w > 0 && crop_h > 0 {
        let cropped = image::imageops::crop_imm(zoomed, crop_x, crop_y, crop_w, crop_h).to_image();
        let paste_x = ((w.saturating_sub(crop_w)) / 2) as i64;
        let paste_y = ((h.saturating_sub(crop_h)) / 2) as i64;
        image::imageops::overlay(&mut result, &cropped, paste_x, paste_y);
    }

    result
}

/// Apply a full view transform (zoom + pan + rotate) to an image, producing an
/// output of the same dimensions with letterboxing where the transformed image
/// does not fill the frame.
///
/// This is a convenience wrapper around [`zoom_rotate_to_size`] + [`apply_pan`].
/// For repeated pan-only updates prefer caching the result of
/// [`zoom_rotate_to_size`] and calling [`apply_pan`] directly.
pub fn apply_view_transform(image: &RgbaImage, transform: ViewTransform) -> RgbaImage {
    // Fast path: no transform at all — just clone
    if transform.zoom == 1.0
        && transform.pan_x == 0.0
        && transform.pan_y == 0.0
        && transform.rotation == 0
    {
        return image.clone();
    }

    let zoomed = zoom_rotate_to_size(image, transform.zoom, transform.rotation);
    apply_pan(&zoomed, transform.pan_x, transform.pan_y, image.dimensions())
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawElement {
    Freehand {
        points: Vec<Point>,
        style: DrawStyle,
    },
    Rectangle {
        start: Point,
        end: Point,
        style: DrawStyle,
    },
    Ellipse {
        start: Point,
        end: Point,
        style: DrawStyle,
    },
    Arrow {
        start: Point,
        end: Point,
        style: DrawStyle,
    },
    Text {
        position: Point,
        text: String,
        style: DrawStyle,
    },
    Highlighter {
        points: Vec<Point>,
        style: DrawStyle,
    },
    Redaction {
        start: Point,
        end: Point,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    x: f32,
    y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
        }
    }

    pub fn x(self) -> f32 {
        self.x
    }

    pub fn y(self) -> f32 {
        self.y
    }

    fn unclamped(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl FitRect {
    fn full() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }

    fn from_pixels(
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        canvas_width: u32,
        canvas_height: u32,
    ) -> Self {
        Self {
            x: x as f32 / canvas_width.max(1) as f32,
            y: y as f32 / canvas_height.max(1) as f32,
            width: width as f32 / canvas_width.max(1) as f32,
            height: height as f32 / canvas_height.max(1) as f32,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RenderSizing {
    pub stroke_radius_px: f32,
    pub text_size_px: f32,
}

impl RenderSizing {
    pub fn scaled(self, scale: f32) -> Self {
        Self {
            stroke_radius_px: self.stroke_radius_px * scale,
            text_size_px: self.text_size_px * scale,
        }
    }

    pub fn stroke_radius_for_style(self, style: DrawStyle) -> f32 {
        (self.stroke_radius_px * style.width.stroke_scale()).max(0.5)
    }

    pub fn text_size_for_style(self, style: DrawStyle) -> f32 {
        (self.text_size_px * style.width.text_scale()).max(4.0)
    }
}

impl DrawingCanvas {
    pub fn new(metrics: TerminalMetrics, source: BaseSource, theme: ThemeMode) -> Self {
        let (base, fit_rect) = render_base(
            &source,
            metrics.width_px,
            metrics.height_px,
            theme.background(),
        );
        let committed = base.clone();
        Self {
            metrics,
            source,
            base,
            fit_rect,
            committed,
            elements: Vec::new(),
            current: None,
            theme,
        }
    }

    pub fn blank(metrics: TerminalMetrics, theme: ThemeMode) -> Self {
        Self::new(metrics, BaseSource::Blank, theme)
    }

    pub fn resize(&mut self, metrics: TerminalMetrics) {
        self.metrics = metrics;
        let (base, fit_rect) = render_base(
            &self.source,
            metrics.width_px,
            metrics.height_px,
            self.theme.background(),
        );
        self.base = base;
        self.fit_rect = fit_rect;
        self.rebuild_committed();
    }

    pub fn begin_element(&mut self, tool: DrawingTool, point: Point, style: DrawStyle) {
        self.current = Some(match tool {
            DrawingTool::Freehand => DrawElement::Freehand {
                points: vec![point],
                style,
            },
            DrawingTool::Highlighter => DrawElement::Highlighter {
                points: vec![point],
                style,
            },
            DrawingTool::Rectangle => DrawElement::Rectangle {
                start: point,
                end: point,
                style,
            },
            DrawingTool::Ellipse => DrawElement::Ellipse {
                start: point,
                end: point,
                style,
            },
            DrawingTool::Arrow => DrawElement::Arrow {
                start: point,
                end: point,
                style,
            },
            DrawingTool::Redaction => DrawElement::Redaction {
                start: point,
                end: point,
            },
            DrawingTool::Text => {
                return;
            }
        });
    }

    pub fn extend_current(&mut self, point: Point) {
        match self.current.as_mut() {
            Some(
                DrawElement::Freehand { points, .. } | DrawElement::Highlighter { points, .. },
            ) => {
                if points.last().copied() != Some(point) {
                    points.push(point);
                }
            }
            Some(
                DrawElement::Rectangle { end, .. }
                | DrawElement::Ellipse { end, .. }
                | DrawElement::Arrow { end, .. }
                | DrawElement::Redaction { end, .. },
            ) => *end = point,
            Some(DrawElement::Text { .. }) => {}
            None => self.begin_stroke(point),
        }
    }

    pub fn finish_current(&mut self) -> bool {
        if let Some(element) = self.current.take() {
            if element.is_empty() {
                return false;
            }
            let sizing = self.preview_sizing();
            draw_element(&mut self.committed, &element, sizing);
            self.elements.push(element);
            return true;
        }
        false
    }

    pub fn cancel_current(&mut self) -> bool {
        self.current.take().is_some()
    }

    pub fn default_stroke_color(&self) -> Rgba<u8> {
        self.theme.stroke()
    }

    pub fn begin_stroke(&mut self, point: Point) {
        self.begin_element(
            DrawingTool::Freehand,
            point,
            DrawStyle::new(self.theme.stroke(), WidthPreset::Medium),
        );
    }

    pub fn add_text(&mut self, position: Point, text: String, style: DrawStyle) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        let element = DrawElement::Text {
            position,
            text,
            style,
        };
        let sizing = self.preview_sizing();
        draw_element(&mut self.committed, &element, sizing);
        self.elements.push(element);
        true
    }

    pub fn undo(&mut self) -> bool {
        self.current = None;
        let did_undo = self.elements.pop().is_some();
        if did_undo {
            self.rebuild_committed();
        }
        did_undo
    }

    pub fn clear(&mut self) -> bool {
        self.current = None;
        let had_strokes = !self.elements.is_empty();
        self.elements.clear();
        self.committed = self.base.clone();
        had_strokes
    }

    pub fn render(&self) -> RgbaImage {
        let mut image = self.committed.clone();
        if let Some(element) = &self.current {
            draw_element(&mut image, element, self.preview_sizing());
        }
        image
    }

    /// Render the canvas with an applied view transform (zoom, pan, rotate).
    /// The composited image is transformed and then letterboxed to canvas size.
    /// When the transform is identity this is equivalent to `render()`.
    pub fn render_transformed(&self, transform: ViewTransform) -> RgbaImage {
        let image = self.render();
        if transform.zoom == 1.0
            && transform.pan_x == 0.0
            && transform.pan_y == 0.0
            && transform.rotation == 0
        {
            image
        } else {
            apply_view_transform(&image, transform)
        }
    }

    /// Given a normalized point in the *displayed* (transformed) canvas,
    /// return the corresponding normalized point in the *original* image
    /// space. This is the inverse of the view transform.
    pub fn inverse_transform_point(
        &self,
        display_point: Point,
        transform: ViewTransform,
    ) -> Point {
        let (w, h) = (self.metrics.width_px as f32, self.metrics.height_px as f32);
        if w < 1.0 || h < 1.0 {
            return display_point;
        }

        // Convert normalized display → pixel in display space
        let dx = display_point.x() * w;
        let dy = display_point.y() * h;

        // Undo letterbox: shift so (0,0) is top-left of the visible image region
        let (rw, rh) = if transform.rotation % 2 == 0 {
            (w, h)
        } else {
            (h, w)
        };
        let zoomed_w = rw * transform.zoom;
        let zoomed_h = rh * transform.zoom;
        let vis_w = w.min(zoomed_w);
        let vis_h = h.min(zoomed_h);
        let letterbox_x = (w - vis_w) / 2.0;
        let letterbox_y = (h - vis_h) / 2.0;
        let lx = dx - letterbox_x;
        let ly = dy - letterbox_y;

        // Undo pan: shift from display space to zoomed-space center
        let shift_x = (zoomed_w - vis_w) * (transform.pan_x + 1.0) / 2.0;
        let shift_y = (zoomed_h - vis_h) * (transform.pan_y + 1.0) / 2.0;
        let zx = lx + shift_x;
        let zy = ly + shift_y;

        // Undo zoom
        let ux = zx / transform.zoom.max(0.01);
        let uy = zy / transform.zoom.max(0.01);

        // Undo rotation
        let (rx, ry) = match transform.rotation {
            1 => (rh - uy, ux),       // inverse of rotate90
            2 => (rw - ux, rh - uy),  // inverse of rotate180
            3 => (uy, rw - ux),       // inverse of rotate270
            _ => (ux, uy),
        };

        // Normalize to original canvas space
        Point::new(rx / rw.max(1.0), ry / rh.max(1.0))
    }

    pub fn render_canvas_export(&self) -> RgbaImage {
        self.render()
    }

    pub fn render_original_export(&self) -> RgbaImage {
        let Some((mut image, scale)) = self.original_export_base_and_scale() else {
            return self.render_canvas_export();
        };
        let sizing = self.preview_sizing().scaled(scale);
        for element in self.transformed_elements_for_original() {
            draw_element(&mut image, &element, sizing);
        }
        image
    }

    pub fn point_for_mouse_cell(&self, column: u16, row: u16) -> Point {
        let x_px = (f32::from(column) + 0.5) * self.metrics.cell_width_px;
        let y_px = (f32::from(row) + 0.5) * self.metrics.cell_height_px;
        self.point_for_pixel(x_px, y_px)
    }

    pub fn point_for_mouse_pixel(&self, column: u16, row: u16) -> Point {
        Point::new(
            f32::from(column) / self.metrics.display_width_px as f32,
            f32::from(row) / self.metrics.display_height_px as f32,
        )
    }

    fn point_for_pixel(&self, x_px: f32, y_px: f32) -> Point {
        Point::new(
            x_px / self.metrics.width_px as f32,
            y_px / self.metrics.height_px as f32,
        )
    }

    pub fn metrics(&self) -> TerminalMetrics {
        self.metrics
    }

    pub fn preview_sizing(&self) -> RenderSizing {
        RenderSizing {
            stroke_radius_px: self.metrics.brush_radius_px(),
            text_size_px: (self.metrics.cell_height_px * 1.1).max(6.0),
        }
    }

    pub fn elements(&self) -> &[DrawElement] {
        &self.elements
    }

    pub fn source(&self) -> &BaseSource {
        &self.source
    }

    pub fn canvas_base(&self) -> &RgbaImage {
        &self.base
    }

    pub fn original_base(&self) -> Option<RgbaImage> {
        match &self.source {
            BaseSource::Blank => None,
            BaseSource::Image(image) => Some(image.to_rgba8()),
        }
    }

    pub fn has_redactions(&self) -> bool {
        self.elements
            .iter()
            .any(|element| matches!(element, DrawElement::Redaction { .. }))
    }

    fn rebuild_committed(&mut self) {
        self.committed = self.base.clone();
        let sizing = self.preview_sizing();
        for element in &self.elements {
            draw_element(&mut self.committed, element, sizing);
        }
    }

    fn original_export_base_and_scale(&self) -> Option<(RgbaImage, f32)> {
        let BaseSource::Image(image) = &self.source else {
            return None;
        };
        let base = image.to_rgba8();
        let fit_width_px = self.fit_rect.width * self.metrics.width_px.max(1) as f32;
        let fit_height_px = self.fit_rect.height * self.metrics.height_px.max(1) as f32;
        let scale = (base.width() as f32 / fit_width_px.max(1.0))
            .min(base.height() as f32 / fit_height_px.max(1.0))
            .max(0.01);
        Some((base, scale))
    }

    pub fn original_export_scale(&self) -> Option<f32> {
        self.original_export_base_and_scale()
            .map(|(_, scale)| scale)
    }

    pub fn transformed_elements_for_original(&self) -> Vec<DrawElement> {
        self.elements
            .iter()
            .map(|element| transform_element_for_original(element, self.fit_rect))
            .collect()
    }

    #[cfg(test)]
    fn stroke_count(&self) -> usize {
        self.elements.len()
    }
}

impl DrawElement {
    fn is_empty(&self) -> bool {
        match self {
            Self::Freehand { points, .. } | Self::Highlighter { points, .. } => points.is_empty(),
            Self::Text { text, .. } => text.trim().is_empty(),
            Self::Rectangle { .. }
            | Self::Ellipse { .. }
            | Self::Arrow { .. }
            | Self::Redaction { .. } => false,
        }
    }
}

fn transform_element_for_original(element: &DrawElement, fit_rect: FitRect) -> DrawElement {
    match element {
        DrawElement::Freehand { points, style } => DrawElement::Freehand {
            points: points
                .iter()
                .map(|point| transform_point_for_original(*point, fit_rect))
                .collect(),
            style: *style,
        },
        DrawElement::Highlighter { points, style } => DrawElement::Highlighter {
            points: points
                .iter()
                .map(|point| transform_point_for_original(*point, fit_rect))
                .collect(),
            style: *style,
        },
        DrawElement::Rectangle { start, end, style } => DrawElement::Rectangle {
            start: transform_point_for_original(*start, fit_rect),
            end: transform_point_for_original(*end, fit_rect),
            style: *style,
        },
        DrawElement::Ellipse { start, end, style } => DrawElement::Ellipse {
            start: transform_point_for_original(*start, fit_rect),
            end: transform_point_for_original(*end, fit_rect),
            style: *style,
        },
        DrawElement::Arrow { start, end, style } => DrawElement::Arrow {
            start: transform_point_for_original(*start, fit_rect),
            end: transform_point_for_original(*end, fit_rect),
            style: *style,
        },
        DrawElement::Text {
            position,
            text,
            style,
        } => DrawElement::Text {
            position: transform_point_for_original(*position, fit_rect),
            text: text.clone(),
            style: *style,
        },
        DrawElement::Redaction { start, end } => DrawElement::Redaction {
            start: transform_point_for_original(*start, fit_rect),
            end: transform_point_for_original(*end, fit_rect),
        },
    }
}

fn transform_point_for_original(point: Point, fit_rect: FitRect) -> Point {
    Point::unclamped(
        (point.x - fit_rect.x) / fit_rect.width.max(f32::EPSILON),
        (point.y - fit_rect.y) / fit_rect.height.max(f32::EPSILON),
    )
}

fn render_base(
    source: &BaseSource,
    width: u32,
    height: u32,
    background: Rgba<u8>,
) -> (RgbaImage, FitRect) {
    let mut base = RgbaImage::from_pixel(width.max(1), height.max(1), background);
    let BaseSource::Image(image) = source else {
        return (base, FitRect::full());
    };
    let (fit_width, fit_height) = fit_dimensions(image.dimensions(), (base.width(), base.height()));
    let resized = image
        .resize_exact(fit_width, fit_height, FilterType::Lanczos3)
        .to_rgba8();
    let x = (base.width() - fit_width) / 2;
    let y = (base.height() - fit_height) / 2;
    overlay(&mut base, x as i32, y as i32, &resized);
    let fit_rect = FitRect::from_pixels(x, y, fit_width, fit_height, base.width(), base.height());
    (base, fit_rect)
}

fn fit_dimensions((src_w, src_h): (u32, u32), (dst_w, dst_h): (u32, u32)) -> (u32, u32) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return (1, 1);
    }
    let scale = (dst_w as f32 / src_w as f32).min(dst_h as f32 / src_h as f32);
    let width = (src_w as f32 * scale).round().clamp(1.0, dst_w as f32) as u32;
    let height = (src_h as f32 * scale).round().clamp(1.0, dst_h as f32) as u32;
    (width, height)
}

fn draw_element(image: &mut RgbaImage, element: &DrawElement, sizing: RenderSizing) {
    match element {
        DrawElement::Freehand { points, style } => draw_freehand(
            image,
            points,
            *style,
            stroke_radius_for_style(*style, sizing),
        ),
        DrawElement::Highlighter { points, style } => draw_freehand(
            image,
            points,
            *style,
            stroke_radius_for_style(*style, sizing) * 3.2,
        ),
        DrawElement::Rectangle { start, end, style } => draw_rectangle_outline(
            image,
            *start,
            *end,
            *style,
            stroke_radius_for_style(*style, sizing),
        ),
        DrawElement::Ellipse { start, end, style } => draw_ellipse_outline(
            image,
            *start,
            *end,
            *style,
            stroke_radius_for_style(*style, sizing),
        ),
        DrawElement::Arrow { start, end, style } => draw_arrow(
            image,
            *start,
            *end,
            *style,
            stroke_radius_for_style(*style, sizing),
        ),
        DrawElement::Text {
            position,
            text,
            style,
        } => {
            draw_text(
                image,
                *position,
                text,
                *style,
                text_size_for_style(*style, sizing),
            );
        }
        DrawElement::Redaction { start, end } => {
            fill_rectangle(image, *start, *end, Rgba([0, 0, 0, 255]), 1.0);
        }
    }
}

fn stroke_radius_for_style(style: DrawStyle, sizing: RenderSizing) -> f32 {
    sizing.stroke_radius_for_style(style)
}

fn text_size_for_style(style: DrawStyle, sizing: RenderSizing) -> f32 {
    sizing.text_size_for_style(style)
}

fn draw_freehand(image: &mut RgbaImage, stroke_points: &[Point], style: DrawStyle, radius: f32) {
    let points = curve_points(stroke_points, image.width(), image.height(), radius);
    let Some(first) = points.first().copied() else {
        return;
    };
    if points.len() == 1 {
        stamp_circle(image, first, style.color, style.opacity, radius);
        return;
    }
    for points in points.windows(2) {
        draw_segment(
            image,
            points[0],
            points[1],
            style.color,
            style.opacity,
            radius,
        );
    }
}

fn draw_rectangle_outline(
    image: &mut RgbaImage,
    start: Point,
    end: Point,
    style: DrawStyle,
    radius: f32,
) {
    let left = start.x.min(end.x);
    let right = start.x.max(end.x);
    let top = start.y.min(end.y);
    let bottom = start.y.max(end.y);
    let top_left = Point::new(left, top);
    let top_right = Point::new(right, top);
    let bottom_right = Point::new(right, bottom);
    let bottom_left = Point::new(left, bottom);

    draw_segment(
        image,
        top_left,
        top_right,
        style.color,
        style.opacity,
        radius,
    );
    draw_segment(
        image,
        top_right,
        bottom_right,
        style.color,
        style.opacity,
        radius,
    );
    draw_segment(
        image,
        bottom_right,
        bottom_left,
        style.color,
        style.opacity,
        radius,
    );
    draw_segment(
        image,
        bottom_left,
        top_left,
        style.color,
        style.opacity,
        radius,
    );
}

fn draw_ellipse_outline(
    image: &mut RgbaImage,
    start: Point,
    end: Point,
    style: DrawStyle,
    radius: f32,
) {
    let (start_x, start_y) = point_to_pixel(image, start);
    let (end_x, end_y) = point_to_pixel(image, end);
    let radius_x = (end_x - start_x).abs() * 0.5;
    let radius_y = (end_y - start_y).abs() * 0.5;
    if radius_x <= 0.5 || radius_y <= 0.5 {
        draw_segment(image, start, end, style.color, style.opacity, radius);
        return;
    }

    let center_x = (start_x + end_x) * 0.5;
    let center_y = (start_y + end_y) * 0.5;
    let circumference = std::f32::consts::PI
        * (3.0 * (radius_x + radius_y)
            - ((3.0 * radius_x + radius_y) * (radius_x + 3.0 * radius_y)).sqrt());
    let samples = (circumference / (radius * 0.65).max(1.0))
        .ceil()
        .clamp(16.0, 240.0) as u32;
    let mut previous = None;
    let mut first = None;

    for step in 0..samples {
        let theta = std::f32::consts::TAU * step as f32 / samples as f32;
        let point = point_from_dimensions_pixel(
            image.width(),
            image.height(),
            center_x + radius_x * theta.cos(),
            center_y + radius_y * theta.sin(),
        );
        if first.is_none() {
            first = Some(point);
        }
        if let Some(previous) = previous {
            draw_segment(image, previous, point, style.color, style.opacity, radius);
        }
        previous = Some(point);
    }

    if let (Some(previous), Some(first)) = (previous, first) {
        draw_segment(image, previous, first, style.color, style.opacity, radius);
    }
}

fn curve_points(points: &[Point], width: u32, height: u32, radius: f32) -> Vec<Point> {
    if points.len() <= 2 {
        return points.to_vec();
    }

    let mut curved = Vec::with_capacity(points.len() * 4);
    for idx in 0..points.len() - 1 {
        let p0 = point_to_dimensions_pixel(width, height, points[idx.saturating_sub(1)]);
        let p1 = point_to_dimensions_pixel(width, height, points[idx]);
        let p2 = point_to_dimensions_pixel(width, height, points[idx + 1]);
        let p3 = point_to_dimensions_pixel(width, height, points[(idx + 2).min(points.len() - 1)]);
        let distance = (p2.0 - p1.0).hypot(p2.1 - p1.1);
        let samples = (distance / (radius * 0.65).max(1.0))
            .ceil()
            .clamp(2.0, 32.0) as u32;
        let first_sample = if idx == 0 { 0 } else { 1 };
        for step in first_sample..=samples {
            let t = step as f32 / samples as f32;
            let (x, y) = catmull_rom(p0, p1, p2, p3, t);
            curved.push(point_from_dimensions_pixel(width, height, x, y));
        }
    }
    curved
}

fn catmull_rom(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    t: f32,
) -> (f32, f32) {
    let t2 = t * t;
    let t3 = t2 * t;
    (
        0.5 * ((2.0 * p1.0)
            + (-p0.0 + p2.0) * t
            + (2.0 * p0.0 - 5.0 * p1.0 + 4.0 * p2.0 - p3.0) * t2
            + (-p0.0 + 3.0 * p1.0 - 3.0 * p2.0 + p3.0) * t3),
        0.5 * ((2.0 * p1.1)
            + (-p0.1 + p2.1) * t
            + (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2
            + (-p0.1 + 3.0 * p1.1 - 3.0 * p2.1 + p3.1) * t3),
    )
}

fn draw_segment(
    image: &mut RgbaImage,
    start: Point,
    end: Point,
    color: Rgba<u8>,
    opacity: f32,
    radius: f32,
) {
    let (start_x, start_y) = point_to_pixel(image, start);
    let (end_x, end_y) = point_to_pixel(image, end);
    let dx = end_x - start_x;
    let dy = end_y - start_y;
    let distance = dx.hypot(dy);
    let steps = (distance / (radius * 0.5).max(1.0)).ceil().max(1.0) as u32;
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        stamp_circle_at(
            image,
            start_x + dx * t,
            start_y + dy * t,
            color,
            opacity,
            radius,
        );
    }
}

fn stamp_circle(image: &mut RgbaImage, point: Point, color: Rgba<u8>, opacity: f32, radius: f32) {
    let (x, y) = point_to_pixel(image, point);
    stamp_circle_at(image, x, y, color, opacity, radius);
}

fn stamp_circle_at(
    image: &mut RgbaImage,
    x: f32,
    y: f32,
    color: Rgba<u8>,
    opacity: f32,
    radius: f32,
) {
    let radius = radius.max(1.0);
    let min_x = (x - radius).floor() as i32;
    let max_x = (x + radius).ceil() as i32;
    let min_y = (y - radius).floor() as i32;
    let max_y = (y + radius).ceil() as i32;
    let radius_squared = radius * radius;

    for yy in min_y..=max_y {
        for xx in min_x..=max_x {
            if xx < 0 || yy < 0 || xx >= image.width() as i32 || yy >= image.height() as i32 {
                continue;
            }
            let px = xx as f32 + 0.5;
            let py = yy as f32 + 0.5;
            if (px - x).powi(2) + (py - y).powi(2) <= radius_squared {
                blend_pixel(image, xx as u32, yy as u32, color, opacity);
            }
        }
    }
}

fn draw_arrow(image: &mut RgbaImage, start: Point, end: Point, style: DrawStyle, radius: f32) {
    draw_segment(image, start, end, style.color, style.opacity, radius);

    let (start_x, start_y) = point_to_pixel(image, start);
    let (end_x, end_y) = point_to_pixel(image, end);
    let dx = end_x - start_x;
    let dy = end_y - start_y;
    let length = dx.hypot(dy);
    if length <= 0.5 {
        return;
    }
    let ux = dx / length;
    let uy = dy / length;
    let px = -uy;
    let py = ux;
    let head_len = (radius * 7.0).max(8.0);
    let head_width = (radius * 4.5).max(5.0);
    let base_x = end_x - ux * head_len;
    let base_y = end_y - uy * head_len;
    fill_triangle(
        image,
        (end_x, end_y),
        (base_x + px * head_width, base_y + py * head_width),
        (base_x - px * head_width, base_y - py * head_width),
        style.color,
        style.opacity,
    );
}

fn fill_triangle(
    image: &mut RgbaImage,
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    color: Rgba<u8>,
    opacity: f32,
) {
    let raw_min_x = a.0.min(b.0).min(c.0).floor();
    let raw_max_x = a.0.max(b.0).max(c.0).ceil();
    let raw_min_y = a.1.min(b.1).min(c.1).floor();
    let raw_max_y = a.1.max(b.1).max(c.1).ceil();
    if raw_max_x < 0.0
        || raw_max_y < 0.0
        || raw_min_x > image.width().saturating_sub(1) as f32
        || raw_min_y > image.height().saturating_sub(1) as f32
    {
        return;
    }
    let min_x = raw_min_x.max(0.0) as u32;
    let max_x = raw_max_x.min(image.width().saturating_sub(1) as f32) as u32;
    let min_y = raw_min_y.max(0.0) as u32;
    let max_y = raw_max_y.min(image.height().saturating_sub(1) as f32) as u32;
    let area = edge(a, b, c);
    if area.abs() <= f32::EPSILON {
        return;
    }

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let point = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = edge(b, c, point);
            let w1 = edge(c, a, point);
            let w2 = edge(a, b, point);
            if (area > 0.0 && w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0)
                || (area < 0.0 && w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0)
            {
                blend_pixel(image, x, y, color, opacity);
            }
        }
    }
}

fn edge(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (c.0 - a.0) * (b.1 - a.1) - (c.1 - a.1) * (b.0 - a.0)
}

fn fill_rectangle(image: &mut RgbaImage, start: Point, end: Point, color: Rgba<u8>, opacity: f32) {
    let (start_x, start_y) = point_to_pixel(image, start);
    let (end_x, end_y) = point_to_pixel(image, end);
    let raw_min_x = start_x.min(end_x).floor();
    let raw_max_x = start_x.max(end_x).ceil();
    let raw_min_y = start_y.min(end_y).floor();
    let raw_max_y = start_y.max(end_y).ceil();
    if raw_max_x < 0.0
        || raw_max_y < 0.0
        || raw_min_x > image.width().saturating_sub(1) as f32
        || raw_min_y > image.height().saturating_sub(1) as f32
    {
        return;
    }
    let min_x = raw_min_x.max(0.0) as u32;
    let max_x = raw_max_x.min(image.width().saturating_sub(1) as f32) as u32;
    let min_y = raw_min_y.max(0.0) as u32;
    let max_y = raw_max_y.min(image.height().saturating_sub(1) as f32) as u32;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            blend_pixel(image, x, y, color, opacity);
        }
    }
}

fn draw_text(image: &mut RgbaImage, position: Point, text: &str, style: DrawStyle, font_size: f32) {
    let Some(font) = annotation_font() else {
        return;
    };
    let (x, y) = point_to_pixel(image, position);
    let scale = PxScale::from(font_size);
    let scaled = font.as_scaled(scale);
    let mut caret = x;
    let baseline = y + scaled.ascent();

    for ch in text.chars() {
        let glyph_id = font.glyph_id(ch);
        let glyph: Glyph = glyph_id.with_scale_and_position(scale, point(caret, baseline));
        caret += scaled.h_advance(glyph_id);
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let x = bounds.min.x as i32 + gx as i32;
                let y = bounds.min.y as i32 + gy as i32;
                if x >= 0 && y >= 0 && x < image.width() as i32 && y < image.height() as i32 {
                    blend_pixel(
                        image,
                        x as u32,
                        y as u32,
                        style.color,
                        style.opacity * coverage.clamp(0.0, 1.0),
                    );
                }
            });
        }
    }
}

fn annotation_font() -> Option<&'static FontArc> {
    static FONT: OnceLock<Option<FontArc>> = OnceLock::new();
    FONT.get_or_init(|| {
        FontArc::try_from_slice(include_bytes!("../assets/NotoSans-Regular.ttf")).ok()
    })
    .as_ref()
}

pub fn annotation_font_bytes() -> &'static [u8] {
    include_bytes!("../assets/NotoSans-Regular.ttf")
}

fn point_to_pixel(image: &RgbaImage, point: Point) -> (f32, f32) {
    point_to_dimensions_pixel(image.width(), image.height(), point)
}

fn point_to_dimensions_pixel(width: u32, height: u32, point: Point) -> (f32, f32) {
    (
        point.x * width.saturating_sub(1) as f32,
        point.y * height.saturating_sub(1) as f32,
    )
}

fn point_from_dimensions_pixel(width: u32, height: u32, x: f32, y: f32) -> Point {
    Point::new(
        x / width.saturating_sub(1).max(1) as f32,
        y / height.saturating_sub(1).max(1) as f32,
    )
}

fn overlay(dst: &mut RgbaImage, x: i32, y: i32, src: &RgbaImage) {
    for sy in 0..src.height() {
        for sx in 0..src.width() {
            let dx = x + sx as i32;
            let dy = y + sy as i32;
            if dx >= 0 && dy >= 0 && dx < dst.width() as i32 && dy < dst.height() as i32 {
                blend_pixel(dst, dx as u32, dy as u32, *src.get_pixel(sx, sy), 1.0);
            }
        }
    }
}

fn blend_pixel(dst: &mut RgbaImage, x: u32, y: u32, src: Rgba<u8>, opacity: f32) {
    let alpha = (src[3] as f32 / 255.0) * opacity.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    if alpha >= 1.0 {
        dst.put_pixel(x, y, src);
        return;
    }
    let mut out = *dst.get_pixel(x, y);
    for channel in 0..3 {
        out[channel] =
            ((src[channel] as f32 * alpha) + (out[channel] as f32 * (1.0 - alpha))).round() as u8;
    }
    out[3] = 255;
    dst.put_pixel(x, y, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> TerminalMetrics {
        TerminalMetrics::from_dimensions(10, 5, 100, 50)
    }

    fn style(color: Rgba<u8>) -> DrawStyle {
        DrawStyle::new(color, WidthPreset::Medium)
    }

    #[test]
    fn blank_canvas_uses_theme_background() {
        let canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        let image = canvas.render();
        assert_eq!(*image.get_pixel(0, 0), Rgba([0, 0, 0, 255]));

        let canvas = DrawingCanvas::blank(metrics(), ThemeMode::Light);
        let image = canvas.render();
        assert_eq!(*image.get_pixel(0, 0), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn fit_dimensions_preserve_aspect() {
        assert_eq!(fit_dimensions((400, 200), (100, 100)), (100, 50));
        assert_eq!(fit_dimensions((200, 400), (100, 100)), (50, 100));
    }

    #[test]
    fn draws_continuous_stroke() {
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        canvas.begin_stroke(Point::new(0.1, 0.5));
        canvas.extend_current(Point::new(0.9, 0.5));
        canvas.finish_current();
        let image = canvas.render();

        for x in 15..85 {
            assert_eq!(*image.get_pixel(x, 25), Rgba([255, 255, 255, 255]));
        }
    }

    #[test]
    fn draws_rectangle_outline_without_filling_center() {
        let red = Rgba([255, 0, 0, 255]);
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        canvas.begin_element(DrawingTool::Rectangle, Point::new(0.2, 0.2), style(red));
        canvas.extend_current(Point::new(0.8, 0.8));
        canvas.finish_current();
        let image = canvas.render();

        assert_eq!(*image.get_pixel(50, 10), red);
        assert_eq!(*image.get_pixel(20, 25), red);
        assert_eq!(*image.get_pixel(50, 25), Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn draws_ellipse_outline_without_filling_center() {
        let green = Rgba([0, 180, 80, 255]);
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        canvas.begin_element(DrawingTool::Ellipse, Point::new(0.2, 0.2), style(green));
        canvas.extend_current(Point::new(0.8, 0.8));
        canvas.finish_current();
        let image = canvas.render();

        assert_eq!(*image.get_pixel(50, 10), green);
        assert_eq!(*image.get_pixel(50, 25), Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn draws_arrow_with_head() {
        let red = Rgba([255, 0, 0, 255]);
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        canvas.begin_element(DrawingTool::Arrow, Point::new(0.2, 0.5), style(red));
        canvas.extend_current(Point::new(0.8, 0.5));
        canvas.finish_current();
        let image = canvas.render();

        assert_eq!(*image.get_pixel(80, 25), red);
        assert_eq!(*image.get_pixel(72, 22), red);
    }

    #[test]
    fn highlighter_alpha_blends_with_background() {
        let yellow = Rgba([255, 221, 0, 255]);
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        canvas.begin_element(
            DrawingTool::Highlighter,
            Point::new(0.5, 0.5),
            DrawStyle::highlighter(yellow, WidthPreset::Medium),
        );
        canvas.finish_current();
        let image = canvas.render();

        assert_eq!(*image.get_pixel(50, 25), Rgba([97, 84, 0, 255]));
    }

    #[test]
    fn redaction_fills_opaque_black_rectangle() {
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Light);
        canvas.begin_element(
            DrawingTool::Redaction,
            Point::new(0.2, 0.2),
            style(Rgba([255, 0, 0, 255])),
        );
        canvas.extend_current(Point::new(0.8, 0.8));
        canvas.finish_current();
        let image = canvas.render();

        assert_eq!(*image.get_pixel(50, 25), Rgba([0, 0, 0, 255]));
        assert_eq!(*image.get_pixel(5, 5), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn text_renders_with_embedded_font() {
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        assert!(canvas.add_text(
            Point::new(0.1, 0.1),
            String::from("Hi"),
            style(Rgba([255, 255, 255, 255])),
        ));
        let image = canvas.render();

        assert!(image.pixels().any(|pixel| *pixel != Rgba([0, 0, 0, 255])));
    }

    #[test]
    fn text_renders_at_clicked_position() {
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        assert!(canvas.add_text(
            Point::new(0.5, 0.5),
            String::from("Hi"),
            style(Rgba([255, 255, 255, 255])),
        ));
        let image = canvas.render();
        let (min_x, min_y) = non_background_bounds(&image, Rgba([0, 0, 0, 255])).unwrap();

        assert!(min_x >= 45, "text started too far left: {min_x}");
        assert!(min_y >= 23, "text started too high: {min_y}");
    }

    #[test]
    fn committed_elements_keep_their_original_colors_after_resize() {
        let red = Rgba([255, 0, 0, 255]);
        let blue = Rgba([30, 100, 255, 255]);
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        canvas.begin_element(DrawingTool::Freehand, Point::new(0.1, 0.2), style(red));
        canvas.extend_current(Point::new(0.4, 0.2));
        canvas.finish_current();
        canvas.begin_element(DrawingTool::Rectangle, Point::new(0.6, 0.6), style(blue));
        canvas.extend_current(Point::new(0.9, 0.9));
        canvas.finish_current();

        canvas.resize(metrics());
        let image = canvas.render();

        assert_eq!(*image.get_pixel(20, 10), red);
        assert_eq!(*image.get_pixel(60, 30), blue);
    }

    #[test]
    fn undo_removes_completed_strokes_many_times() {
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        for x in [0.2, 0.4, 0.6] {
            canvas.begin_stroke(Point::new(x, 0.5));
            canvas.finish_current();
        }

        assert_eq!(canvas.stroke_count(), 3);
        assert!(canvas.undo());
        assert!(canvas.undo());
        assert!(canvas.undo());
        assert!(!canvas.undo());
        assert_eq!(canvas.stroke_count(), 0);
    }

    #[test]
    fn clear_removes_strokes_and_preserves_base() {
        let mut canvas = DrawingCanvas::blank(metrics(), ThemeMode::Light);
        canvas.begin_stroke(Point::new(0.5, 0.5));
        canvas.finish_current();
        assert!(canvas.clear());
        assert!(!canvas.undo());

        let image = canvas.render();
        assert_eq!(*image.get_pixel(50, 25), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn mouse_cells_map_to_normalized_points() {
        let canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        assert_eq!(canvas.point_for_mouse_cell(0, 0), Point::new(0.05, 0.1));
        assert_eq!(canvas.point_for_mouse_cell(9, 4), Point::new(0.95, 0.9));
    }

    #[test]
    fn mouse_pixels_map_to_normalized_points() {
        let canvas = DrawingCanvas::blank(metrics(), ThemeMode::Dark);
        assert_eq!(canvas.point_for_mouse_pixel(0, 0), Point::new(0.0, 0.0));
        assert_eq!(canvas.point_for_mouse_pixel(50, 25), Point::new(0.5, 0.5));
        assert_eq!(canvas.point_for_mouse_pixel(100, 50), Point::new(1.0, 1.0));
    }

    #[test]
    fn curve_points_preserve_cursor_trail_points() {
        let points = [
            Point::new(0.0, 0.0),
            Point::new(0.1, 0.0),
            Point::new(0.1, 0.1),
            Point::new(0.2, 0.1),
            Point::new(0.2, 0.2),
            Point::new(0.3, 0.2),
            Point::new(0.3, 0.3),
        ];
        let curved = curve_points(&points, 100, 100, 2.0);
        assert!(curved.len() > points.len());
        assert!(points_are_close(
            curved.first().copied().unwrap(),
            points[0]
        ));
        assert!(points_are_close(
            curved.last().copied().unwrap(),
            points[points.len() - 1]
        ));
        for point in points {
            assert!(curved.iter().any(|curved| points_are_close(*curved, point)));
        }
    }

    fn points_are_close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 0.0001 && (a.y - b.y).abs() < 0.0001
    }

    fn non_background_bounds(image: &RgbaImage, background: Rgba<u8>) -> Option<(u32, u32)> {
        let mut min_x = image.width();
        let mut min_y = image.height();
        for (x, y, pixel) in image.enumerate_pixels() {
            if *pixel != background {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
            }
        }
        (min_x < image.width()).then_some((min_x, min_y))
    }
}
