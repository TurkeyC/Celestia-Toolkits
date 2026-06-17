use std::{
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    cursor::MoveTo,
    event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    queue,
    style::{Color, Print, ResetColor, SetBackgroundColor},
    terminal::{Clear, ClearType},
};
use image::Rgba;

use image::RgbaImage;

use crate::{
    args::{CellPixels, ensure_output_path},
    canvas::{
        apply_pan, zoom_rotate_to_size, BaseSource, DrawStyle, DrawingCanvas, DrawingTool, Point,
        ViewTransform, WidthPreset,
    },
    export::{self, ExportFormat, ExportSize},
    kitty,
    terminal::{TerminalLayout, TerminalSession},
    theme::ThemeMode,
};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MIN_STATUS_HIT_SLOP_PX: u16 = 4;

pub struct AppConfig {
    pub input_image: Option<PathBuf>,
    pub output: PathBuf,
    pub output_format: ExportFormat,
    pub export_size: ExportSize,
    pub theme: ThemeMode,
    pub fallback_cell_px: CellPixels,
    pub resolution_scale: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Browse,
    Draw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SavePromptState {
    Active { triggered_by_quit: bool },
}

/// Cached result of [`zoom_rotate_to_size`] for browse-mode rendering.
/// When the user is only panning (zoom/rotation unchanged), we re-use this
/// cached image and only re-apply the pan/letterbox, which is much cheaper
/// than a full Triangle resize.
#[derive(Debug)]
struct BrowseCache {
    zoom: f32,
    rotation: u8,
    zoomed_image: RgbaImage,
}

impl Clone for BrowseCache {
    fn clone(&self) -> Self {
        Self {
            zoom: self.zoom,
            rotation: self.rotation,
            zoomed_image: self.zoomed_image.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct AppState {
    mode: AppMode,
    tool: DrawingTool,
    color: Rgba<u8>,
    width: WidthPreset,
    input_mode: InputMode,
    message: String,
    output_format: ExportFormat,
    export_size: ExportSize,
    view_transform: ViewTransform,
    save_prompt: Option<SavePromptState>,
    original_path: Option<PathBuf>,
    pan_drag_start: Option<(f32, f32)>,
    browse_cache: Option<BrowseCache>,
}

#[derive(Debug, Clone, PartialEq)]
enum InputMode {
    None,
    Color { buffer: String },
    Text { position: Point, buffer: String },
    SavePath { buffer: String },
}

impl AppState {
    fn new(
        color: Rgba<u8>,
        output_format: ExportFormat,
        export_size: ExportSize,
        original_path: Option<PathBuf>,
    ) -> Self {
        Self {
            mode: AppMode::Browse,
            tool: DrawingTool::Freehand,
            color,
            width: WidthPreset::Medium,
            input_mode: InputMode::None,
            message: String::from("Ready"),
            output_format,
            export_size,
            view_transform: ViewTransform::identity(),
            save_prompt: None,
            original_path,
            pan_drag_start: None,
            browse_cache: None,
        }
    }

    fn set_tool(&mut self, tool: DrawingTool) {
        self.tool = tool;
        self.message = format!("Tool: {}", tool_label(tool));
    }

    fn begin_color_input(&mut self) {
        self.input_mode = InputMode::Color {
            buffer: String::new(),
        };
        self.message = String::from("Enter color");
    }

    fn cancel_color_input(&mut self) {
        self.input_mode = InputMode::None;
        self.message = String::from("Color unchanged");
    }

    fn apply_color_input(&mut self) {
        let InputMode::Color { buffer } = &self.input_mode else {
            return;
        };
        match parse_color(buffer) {
            Some(color) => {
                self.color = color;
                self.input_mode = InputMode::None;
                self.message = format!("Color: {}", color_to_hex(color));
            }
            None => {
                let value = buffer.trim();
                self.message = if value.is_empty() {
                    String::from("Enter a color name or hex value")
                } else {
                    format!("Unknown color: {value}")
                };
            }
        }
    }

    fn set_color(&mut self, color: Rgba<u8>, name: &str) {
        self.color = color;
        self.input_mode = InputMode::None;
        self.message = format!("Color: {name}");
    }

    fn begin_text_input(&mut self, position: Point) {
        self.input_mode = InputMode::Text {
            position,
            buffer: String::new(),
        };
        self.message = String::from("Enter text");
    }

    fn cancel_text_input(&mut self) {
        self.input_mode = InputMode::None;
        self.message = String::from("Text canceled");
    }

    fn style(&self) -> DrawStyle {
        match self.tool {
            DrawingTool::Highlighter => DrawStyle::highlighter(self.color, self.width),
            DrawingTool::Redaction => DrawStyle::new(Rgba([0, 0, 0, 255]), self.width),
            _ => DrawStyle::new(self.color, self.width),
        }
    }

    fn cycle_width_previous(&mut self) {
        self.width = self.width.previous();
        self.message = format!("Size: {}", self.width.label());
    }

    fn cycle_width_next(&mut self) {
        self.width = self.width.next();
        self.message = format!("Size: {}", self.width.label());
    }
}

#[derive(Debug, Clone, Copy)]
struct PaletteColor {
    name: &'static str,
    color: Rgba<u8>,
}

const PALETTE: [PaletteColor; 9] = [
    PaletteColor {
        name: "black",
        color: Rgba([0, 0, 0, 255]),
    },
    PaletteColor {
        name: "white",
        color: Rgba([255, 255, 255, 255]),
    },
    PaletteColor {
        name: "red",
        color: Rgba([255, 0, 0, 255]),
    },
    PaletteColor {
        name: "orange",
        color: Rgba([255, 128, 0, 255]),
    },
    PaletteColor {
        name: "yellow",
        color: Rgba([255, 221, 0, 255]),
    },
    PaletteColor {
        name: "green",
        color: Rgba([0, 180, 80, 255]),
    },
    PaletteColor {
        name: "cyan",
        color: Rgba([0, 190, 220, 255]),
    },
    PaletteColor {
        name: "blue",
        color: Rgba([30, 100, 255, 255]),
    },
    PaletteColor {
        name: "purple",
        color: Rgba([160, 80, 220, 255]),
    },
];

pub fn run(config: AppConfig) -> Result<()> {
    ensure_output_path(&config.output)?;
    let source = match config.input_image.as_deref() {
        Some(path) => {
            if !path.exists() {
                anyhow::bail!("input image not found: {}", path.display());
            }
            BaseSource::Image(
                image::open(path)
                    .with_context(|| format!("failed to load image {}", path.display()))?,
            )
        }
        None => BaseSource::Blank,
    };

    let session = TerminalSession::enter()?;
    let layout = TerminalLayout::current(config.fallback_cell_px, config.resolution_scale);
    let mut canvas = match source {
        BaseSource::Blank => DrawingCanvas::blank(layout.canvas, config.theme),
        BaseSource::Image(image) => {
            DrawingCanvas::new(layout.canvas, BaseSource::Image(image), config.theme)
        }
    };
    let mut state = AppState::new(
        canvas.default_stroke_color(),
        config.output_format,
        config.export_size,
        config.input_image.clone(),
    );
    run_event_loop(
        &mut canvas,
        &mut state,
        layout,
        config.fallback_cell_px,
        config.resolution_scale,
    )?;
    drop(session);

    // No auto-save on exit — saving is handled by the user via save prompt
    // during the event loop. If the user did not save, any unsaved changes
    // are discarded on exit.
    Ok(())
}

fn run_event_loop(
    canvas: &mut DrawingCanvas,
    state: &mut AppState,
    mut layout: TerminalLayout,
    fallback_cell_px: CellPixels,
    resolution_scale: f32,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    render_to_terminal(&mut stdout, canvas, state, layout)?;
    let mut last_render = Instant::now();
    let mut dirty = false;
    let mut mouse_mapper = MouseMapper::new();

    loop {
        let wait = if dirty {
            FRAME_INTERVAL
                .checked_sub(last_render.elapsed())
                .unwrap_or(Duration::ZERO)
        } else {
            EVENT_POLL_INTERVAL
        };

        if event::poll(wait)? {
            loop {
                match event::read()? {
                    Event::Key(key)
                        if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                    {
                        if handle_key(key, canvas, state) {
                            return Ok(());
                        }
                        dirty = true;
                    }
                    Event::Mouse(mouse)
                        if handle_mouse(mouse, canvas, state, layout, &mut mouse_mapper) =>
                    {
                        dirty = true;
                    }
                    Event::Resize(cols, rows) => {
                        layout = TerminalLayout::from_cells(
                            cols,
                            rows,
                            fallback_cell_px,
                            resolution_scale,
                        );
                        canvas.resize(layout.canvas);
                        state.browse_cache = None;
                        mouse_mapper = MouseMapper::new();
                        dirty = true;
                    }
                    _ => {}
                }

                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }

        if dirty && last_render.elapsed() >= FRAME_INTERVAL {
            render_to_terminal(&mut stdout, canvas, state, layout)?;
            last_render = Instant::now();
            dirty = false;
        }
    }
}

fn handle_key(key: KeyEvent, canvas: &mut DrawingCanvas, state: &mut AppState) -> bool {
    // Save prompt takes priority
    if state.save_prompt.is_some() {
        return handle_save_prompt_key(key, canvas, state);
    }

    match &state.input_mode {
        InputMode::Color { .. } => return handle_color_input_key(key, state),
        InputMode::Text { .. } => return handle_text_input_key(key, canvas, state),
        InputMode::SavePath { .. } => return handle_save_path_input_key(key, canvas, state),
        InputMode::None => {}
    }

    // Browse mode quit shortcut: no elements → quit immediately
    if state.mode == AppMode::Browse && canvas.elements().is_empty() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
            _ => {}
        }
    }

    match state.mode {
        AppMode::Browse => handle_browse_key(key, state),
        AppMode::Draw => handle_draw_key(key, canvas, state),
    }
}

fn handle_browse_key(key: KeyEvent, state: &mut AppState) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            state.save_prompt = Some(SavePromptState::Active { triggered_by_quit: true });
            state.message = String::from("Save before quitting? (y)es / (n)o / s(a)ve as");
            false
        }
        KeyCode::Char('p') => {
            state.mode = AppMode::Draw;
            state.view_transform = ViewTransform::identity();
            state.browse_cache = None;
            state.message = String::from(
                "Draw mode | f=freehand r=rect e=ellipse a=arrow t=text h=highlight x=redact z=undo C=clear c=color [/]=size p=back",
            );
            false
        }
        KeyCode::Char('r') if key.modifiers.is_empty() => {
            state.view_transform.rotation = (state.view_transform.rotation + 1) % 4;
            state.message = format!("Rotation: {}°", state.view_transform.rotation as u32 * 90);
            false
        }
        KeyCode::Char('R') => {
            state.view_transform.rotation = (state.view_transform.rotation + 3) % 4;
            state.message = format!("Rotation: {}°", state.view_transform.rotation as u32 * 90);
            false
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            state.view_transform.zoom = (state.view_transform.zoom * 1.1).min(4.0);
            state.message = format!("Zoom: {:.0}%", state.view_transform.zoom * 100.0);
            false
        }
        KeyCode::Char('-') => {
            state.view_transform.zoom = (state.view_transform.zoom / 1.1).max(0.1);
            state.message = format!("Zoom: {:.0}%", state.view_transform.zoom * 100.0);
            false
        }
        KeyCode::Char('0') => {
            state.view_transform.zoom = 1.0;
            state.view_transform.pan_x = 0.0;
            state.view_transform.pan_y = 0.0;
            state.message = String::from("Zoom: 100%");
            false
        }
        KeyCode::Left => {
            state.view_transform.pan_x = (state.view_transform.pan_x - 0.05).max(-1.0);
            false
        }
        KeyCode::Right => {
            state.view_transform.pan_x = (state.view_transform.pan_x + 0.05).min(1.0);
            false
        }
        KeyCode::Up => {
            state.view_transform.pan_y = (state.view_transform.pan_y - 0.05).max(-1.0);
            false
        }
        KeyCode::Down => {
            state.view_transform.pan_y = (state.view_transform.pan_y + 0.05).min(1.0);
            false
        }
        _ => false,
    }
}

fn handle_draw_key(key: KeyEvent, canvas: &mut DrawingCanvas, state: &mut AppState) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            if !canvas.elements().is_empty() {
                state.save_prompt = Some(SavePromptState::Active { triggered_by_quit: true });
                state.message = String::from("Save changes? (y)es / (n)o / s(a)ve as");
                false
            } else {
                true
            }
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if !canvas.elements().is_empty() {
                state.save_prompt = Some(SavePromptState::Active { triggered_by_quit: true });
                state.message = String::from("Save changes? (y)es / (n)o / s(a)ve as");
                false
            } else {
                true
            }
        }
        KeyCode::Char('p') => {
            if !canvas.elements().is_empty() {
                state.save_prompt = Some(SavePromptState::Active { triggered_by_quit: false });
                state.message = String::from("Save changes? (y)es / (n)o / s(a)ve as");
                false
            } else {
                state.mode = AppMode::Browse;
                state.message = String::from("Browse mode");
                false
            }
        }
        KeyCode::Char('z') => {
            canvas.undo();
            false
        }
        KeyCode::Char('f') => {
            canvas.cancel_current();
            state.set_tool(DrawingTool::Freehand);
            false
        }
        KeyCode::Char('r') => {
            canvas.cancel_current();
            state.set_tool(DrawingTool::Rectangle);
            false
        }
        KeyCode::Char('e') => {
            canvas.cancel_current();
            state.set_tool(DrawingTool::Ellipse);
            false
        }
        KeyCode::Char('a') => {
            canvas.cancel_current();
            state.set_tool(DrawingTool::Arrow);
            false
        }
        KeyCode::Char('t') => {
            canvas.cancel_current();
            state.set_tool(DrawingTool::Text);
            false
        }
        KeyCode::Char('h') => {
            canvas.cancel_current();
            state.set_tool(DrawingTool::Highlighter);
            false
        }
        KeyCode::Char('x') => {
            canvas.cancel_current();
            state.set_tool(DrawingTool::Redaction);
            false
        }
        KeyCode::Char('c') => {
            state.begin_color_input();
            false
        }
        KeyCode::Char('C') => {
            canvas.clear();
            state.message = String::from("Drawing layer cleared");
            false
        }
        KeyCode::Char('[') => {
            state.cycle_width_previous();
            false
        }
        KeyCode::Char(']') => {
            state.cycle_width_next();
            false
        }
        _ => false,
    }
}

fn handle_save_prompt_key(key: KeyEvent, canvas: &mut DrawingCanvas, state: &mut AppState) -> bool {
    let triggered_by_quit = matches!(
        state.save_prompt,
        Some(SavePromptState::Active {
            triggered_by_quit: true
        })
    );

    match key.code {
        KeyCode::Char('y') => {
            let path = state.original_path.clone();
            if let Some(path) = path {
                let fmt = if path.extension().map_or(false, |e| e == "svg") {
                    ExportFormat::Svg
                } else {
                    ExportFormat::Png
                };
                if export::save(&path, fmt, state.export_size, canvas).is_ok() {
                    state.save_prompt = None;
                    state.mode = AppMode::Browse;
                    state.message = format!("Saved to {}", path.display());
                } else {
                    state.message = String::from("Save failed");
                }
                if triggered_by_quit {
                    return true; // quit after save
                }
                false
            } else {
                // No original file — fall through to save-as
                state.input_mode = InputMode::SavePath {
                    buffer: String::new(),
                };
                state.save_prompt = None;
                state.message = String::from("Enter save path:");
                false
            }
        }
        KeyCode::Char('n') => {
            state.save_prompt = None;
            state.mode = AppMode::Browse;
            state.message = String::from("Changes discarded");
            if triggered_by_quit {
                true // quit was requested
            } else {
                false
            }
        }
        KeyCode::Char('a') => {
            state.input_mode = InputMode::SavePath {
                buffer: String::new(),
            };
            state.save_prompt = None;
            state.message = String::from("Save as path:");
            false
        }
        KeyCode::Esc => {
            state.save_prompt = None;
            if triggered_by_quit || state.mode == AppMode::Browse {
                state.mode = AppMode::Browse;
            } else {
                state.mode = AppMode::Draw;
            }
            state.message = String::from("Canceled");
            false
        }
        _ => false,
    }
}

fn handle_save_path_input_key(
    key: KeyEvent,
    canvas: &mut DrawingCanvas,
    state: &mut AppState,
) -> bool {
    match key.code {
        KeyCode::Esc => {
            state.input_mode = InputMode::None;
            state.mode = AppMode::Draw;
            state.message = String::from("Save canceled");
            false
        }
        KeyCode::Enter => {
            let InputMode::SavePath { buffer } =
                std::mem::replace(&mut state.input_mode, InputMode::None)
            else {
                return false;
            };
            let path = PathBuf::from(buffer.trim());
            let fmt = if path.extension().map_or(false, |e| e == "svg") {
                ExportFormat::Svg
            } else {
                ExportFormat::Png
            };
            if export::save(&path, fmt, state.export_size, canvas).is_ok() {
                state.save_prompt = None;
                state.mode = AppMode::Browse;
                state.original_path = Some(path.clone());
                state.message = format!("Saved to {}", path.display());
            } else {
                state.message = String::from("Save failed");
                state.mode = AppMode::Draw;
            }
            false
        }
        KeyCode::Backspace => {
            if let InputMode::SavePath { buffer } = &mut state.input_mode {
                buffer.pop();
            }
            false
        }
        KeyCode::Char(ch) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            if let InputMode::SavePath { buffer } = &mut state.input_mode {
                buffer.push(ch);
            }
            false
        }
        _ => false,
    }
}

fn handle_color_input_key(key: KeyEvent, state: &mut AppState) -> bool {
    if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }

    match key.code {
        KeyCode::Esc => {
            state.cancel_color_input();
            false
        }
        KeyCode::Enter => {
            state.apply_color_input();
            false
        }
        KeyCode::Backspace => {
            if let InputMode::Color { buffer } = &mut state.input_mode {
                buffer.pop();
            }
            false
        }
        KeyCode::Char(ch) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            if let InputMode::Color { buffer } = &mut state.input_mode {
                buffer.push(ch);
            }
            false
        }
        _ => false,
    }
}

fn handle_text_input_key(key: KeyEvent, canvas: &mut DrawingCanvas, state: &mut AppState) -> bool {
    if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }

    match key.code {
        KeyCode::Esc => {
            state.cancel_text_input();
            false
        }
        KeyCode::Enter => {
            let InputMode::Text { position, buffer } =
                std::mem::replace(&mut state.input_mode, InputMode::None)
            else {
                return false;
            };
            if canvas.add_text(position, buffer, state.style()) {
                state.message = String::from("Text added");
            } else {
                state.message = String::from("Text skipped");
            }
            false
        }
        KeyCode::Backspace => {
            if let InputMode::Text { buffer, .. } = &mut state.input_mode {
                buffer.pop();
            }
            false
        }
        KeyCode::Char(ch) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            if let InputMode::Text { buffer, .. } = &mut state.input_mode {
                buffer.push(ch);
            }
            false
        }
        _ => false,
    }
}

fn handle_mouse(
    mouse: MouseEvent,
    canvas: &mut DrawingCanvas,
    state: &mut AppState,
    layout: TerminalLayout,
    mouse_mapper: &mut MouseMapper,
) -> bool {
    if !matches!(state.input_mode, InputMode::None) || state.save_prompt.is_some() {
        return false;
    }

    match state.mode {
        AppMode::Browse => handle_browse_mouse(mouse, state, layout),
        AppMode::Draw => handle_draw_mouse(mouse, canvas, state, layout, mouse_mapper),
    }
}

fn handle_browse_mouse(
    mouse: MouseEvent,
    state: &mut AppState,
    layout: TerminalLayout,
) -> bool {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Check if click is on canvas area (not status/input rows)
            let row = mouse.row;
            if Some(row) == layout.status_row || Some(row) == layout.input_row {
                return false;
            }
            state.pan_drag_start = Some((f32::from(mouse.column), f32::from(mouse.row)));
            true
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some((start_col, start_row)) = state.pan_drag_start {
                let delta_col = f32::from(mouse.column) - start_col;
                let delta_row = f32::from(mouse.row) - start_row;

                // Detect coordinate mode.  With SGR pixel mouse (DECSET 1016,
                // which we enable) the terminal reports coordinates in pixels.
                // In cell mode they are character-cell indices.
                let pixel_mode =
                    mouse.column > layout.cols || mouse.row > layout.rows;

                // Normalise the mouse delta to a fraction of the screen
                // dimension — this way it's resolution-independent.
                let screen_frac_x = if pixel_mode {
                    delta_col / layout.display_width_px.max(1) as f32
                } else {
                    delta_col / layout.cols.max(1) as f32
                };
                let screen_frac_y = if pixel_mode {
                    delta_row / layout.display_height_px.max(1) as f32
                } else {
                    delta_row / layout.rows.max(1) as f32
                };

                // 1:1 sticky-pan formula:
                //
                //   Δpan = 2 · Δscreen_fraction / (zoom - 1)
                //
                // Derivation: in apply_pan the shift (in canvas pixels) is
                //
                //   shift_x = (zw - w) · (pan_x + 1) / 2
                //
                // so the change in shift per unit pan_x is (zw-w)/2.
                // We want the display-pixel shift to exactly match the
                // mouse-drag distance:
                //
                //   Δpan · (zw-w)/2 · (display_w / w) = Δfraction · display_w
                //
                //  → Δpan · (zw-w)/2 = Δfraction · w
                //  → Δpan = 2 · Δfraction · w / (zw-w)
                //         = 2 · Δfraction / (zoom - 1)   [since zw = w·zoom]
                //
                // At zoom ≤ 1.0 the zoomed image fits inside the canvas and
                // panning has no visual effect (apply_pan clamps shift to 0),
                // so we clamp zoom to just above 1.0 to keep the math stable.
                let z = state.view_transform.zoom.max(1.001);
                let scale = 2.0 / (z - 1.0);
                let pan_delta_x = (screen_frac_x * scale).clamp(-2.0, 2.0);
                let pan_delta_y = (screen_frac_y * scale).clamp(-2.0, 2.0);

                state.view_transform.pan_x = (state.view_transform.pan_x
                    - pan_delta_x)
                    .clamp(-1.0, 1.0);
                state.view_transform.pan_y = (state.view_transform.pan_y
                    - pan_delta_y)
                    .clamp(-1.0, 1.0);
                state.pan_drag_start = Some((f32::from(mouse.column), f32::from(mouse.row)));
                true
            } else {
                false
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            state.pan_drag_start = None;
            false
        }
        MouseEventKind::ScrollDown => {
            state.view_transform.zoom = (state.view_transform.zoom / 1.05).max(0.1);
            state.message = format!("Zoom: {:.0}%", state.view_transform.zoom * 100.0);
            true
        }
        MouseEventKind::ScrollUp => {
            state.view_transform.zoom = (state.view_transform.zoom * 1.05).min(4.0);
            state.message = format!("Zoom: {:.0}%", state.view_transform.zoom * 100.0);
            true
        }
        _ => false,
    }
}

fn handle_draw_mouse(
    mouse: MouseEvent,
    canvas: &mut DrawingCanvas,
    state: &mut AppState,
    layout: TerminalLayout,
    mouse_mapper: &mut MouseMapper,
) -> bool {
    // Draw mode renders at 1:1 (no view transform), so mouse coordinates
    // map directly to canvas coordinates without inverse transformation.
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            match mouse_mapper.target_for_mouse(mouse, layout, canvas) {
                MouseTarget::Canvas(point) => {
                    if state.tool == DrawingTool::Text {
                        state.begin_text_input(point);
                    } else {
                        canvas.begin_element(state.tool, point, state.style());
                    }
                    true
                }
                MouseTarget::Status { column } => {
                    if let Some(palette_color) = palette_color_at_column(column, state, layout.cols)
                    {
                        state.set_color(palette_color.color, palette_color.name);
                        true
                    } else {
                        false
                    }
                }
                MouseTarget::Input | MouseTarget::None => false,
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let MouseTarget::Canvas(point) = mouse_mapper.target_for_mouse(mouse, layout, canvas)
            {
                canvas.extend_current(point);
                true
            } else {
                false
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let MouseTarget::Canvas(point) = mouse_mapper.target_for_mouse(mouse, layout, canvas)
            {
                canvas.extend_current(point);
            }
            canvas.finish_current()
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseCoordinateMode {
    Pixel,
    Cell,
}

#[derive(Debug, Clone, Copy)]
struct MouseMapper {
    mode: MouseCoordinateMode,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum MouseTarget {
    Canvas(Point),
    Status { column: u16 },
    Input,
    None,
}

impl MouseMapper {
    fn new() -> Self {
        Self {
            mode: if prefers_pixel_mouse() {
                MouseCoordinateMode::Pixel
            } else {
                MouseCoordinateMode::Cell
            },
        }
    }

    fn target_for_mouse(
        &mut self,
        mouse: MouseEvent,
        layout: TerminalLayout,
        canvas: &DrawingCanvas,
    ) -> MouseTarget {
        if mouse.column >= layout.cols || mouse.row >= layout.rows {
            self.mode = MouseCoordinateMode::Pixel;
        }
        match self.mode {
            MouseCoordinateMode::Pixel => {
                let column = layout.column_for_pixel(mouse.column);
                let row = layout.row_for_pixel(mouse.row);
                if Some(row) == layout.status_row || is_near_status_row(mouse.row, layout) {
                    return MouseTarget::Status { column };
                }
                if Some(row) == layout.input_row {
                    return MouseTarget::Input;
                }
                if u32::from(mouse.row) < canvas.metrics().display_height_px {
                    return MouseTarget::Canvas(
                        canvas.point_for_mouse_pixel(mouse.column, mouse.row),
                    );
                }
                MouseTarget::None
            }
            MouseCoordinateMode::Cell => {
                if mouse.row < canvas.metrics().rows {
                    MouseTarget::Canvas(canvas.point_for_mouse_cell(mouse.column, mouse.row))
                } else if Some(mouse.row) == layout.status_row {
                    MouseTarget::Status {
                        column: mouse.column,
                    }
                } else if Some(mouse.row) == layout.input_row {
                    MouseTarget::Input
                } else {
                    MouseTarget::None
                }
            }
        }
    }
}

fn is_near_status_row(pixel_y: u16, layout: TerminalLayout) -> bool {
    let Some(status_row) = layout.status_row else {
        return false;
    };
    let status_start = (f32::from(status_row) * layout.cell_height_px).floor() as u16;
    let slop = status_hit_slop_px(layout);
    pixel_y >= status_start.saturating_sub(slop) && pixel_y < status_start
}

fn status_hit_slop_px(layout: TerminalLayout) -> u16 {
    (layout.cell_height_px * 0.25)
        .ceil()
        .clamp(f32::from(MIN_STATUS_HIT_SLOP_PX), 10.0) as u16
}

fn prefers_pixel_mouse() -> bool {
    std::env::var("KITTY_WINDOW_ID").is_ok()
        || std::env::var("TERM_PROGRAM")
            .map(|value| value.to_ascii_lowercase().contains("ghostty"))
            .unwrap_or(false)
        || std::env::var("TERM")
            .map(|value| {
                let value = value.to_ascii_lowercase();
                value.contains("kitty") || value.contains("ghostty")
            })
            .unwrap_or(false)
}

fn render_to_terminal<W: Write>(
    writer: &mut W,
    canvas: &DrawingCanvas,
    state: &mut AppState,
    layout: TerminalLayout,
) -> Result<()> {
    // Always reset cursor to top-left before rendering the image.
    queue!(writer, MoveTo(0, 0))?;

    match state.mode {
        AppMode::Browse => {
            render_browse(writer, canvas, state)?;
        }
        AppMode::Draw => {
            let image = canvas.render();
            let metrics = canvas.metrics();
            kitty::write_frame(writer, &image, metrics.cols, metrics.rows, true)?;
        }
    }
    render_ui(writer, state, layout)?;
    Ok(())
}

/// Render the canvas for Browse mode with client-side zoom cache.
///
/// When the view transform is identity we skip the cache entirely.
/// When zoom/rotation is unchanged we reuse the cached zoomed+rotated image
/// and apply only the pan/letterbox — avoiding a slow Triangle resize on
/// every pan update.  The resulting image is always written to the terminal
/// via `kitty::write_frame` (full RGBA encode).  In a future iteration this
/// could use Kitty's `a=c` compose to skip the encode as well.
fn render_browse<W: Write>(
    writer: &mut W,
    canvas: &DrawingCanvas,
    state: &mut AppState,
) -> Result<()> {
    let v = state.view_transform;
    let metrics = canvas.metrics();

    // Identity transform — render directly, no cache.
    if v.zoom == 1.0 && v.pan_x == 0.0 && v.pan_y == 0.0 && v.rotation == 0 {
        state.browse_cache = None;
        let image = canvas.render();
        kitty::write_frame(writer, &image, metrics.cols, metrics.rows, true)?;
        return Ok(());
    }

    // Check the client-side zoom cache.
    let cache_hit = state
        .browse_cache
        .as_ref()
        .map(|c| c.zoom == v.zoom && c.rotation == v.rotation)
        .unwrap_or(false);

    let image = if cache_hit {
        // Only pan changed — reuse the cached zoomed image.
        apply_pan(
            &state.browse_cache.as_ref().unwrap().zoomed_image,
            v.pan_x,
            v.pan_y,
            (metrics.width_px, metrics.height_px),
        )
    } else {
        // Zoom or rotation changed — re-render the zoomed image.
        let img = canvas.render();
        let zoomed = zoom_rotate_to_size(&img, v.zoom, v.rotation);
        let result = apply_pan(&zoomed, v.pan_x, v.pan_y, (img.width(), img.height()));
        state.browse_cache = Some(BrowseCache {
            zoom: v.zoom,
            rotation: v.rotation,
            zoomed_image: zoomed,
        });
        result
    };

    kitty::write_frame(writer, &image, metrics.cols, metrics.rows, true)?;
    Ok(())
}

fn render_ui<W: Write>(writer: &mut W, state: &AppState, layout: TerminalLayout) -> Result<()> {
    if let Some(row) = layout.status_row {
        queue!(writer, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
        write_status_row(writer, state, layout.cols)?;
    }
    if let Some(row) = layout.input_row {
        queue!(writer, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
        let text = input_row_text(state);
        queue!(writer, Print(truncate_to_cols(&text, layout.cols)))?;
    }
    writer.flush()?;
    Ok(())
}

fn write_status_row<W: Write>(writer: &mut W, state: &AppState, cols: u16) -> Result<()> {
    let prefix = status_prefix(state);
    let mut used_cols = prefix.chars().count() as u16;
    queue!(writer, Print(truncate_to_cols(&prefix, cols)))?;
    if used_cols >= cols {
        return Ok(());
    }

    // Only show palette in Draw mode
    if state.mode == AppMode::Draw {
        for palette_color in PALETTE {
            let swatch_width = palette_swatch_width(palette_color);
            if used_cols.saturating_add(swatch_width) > cols {
                break;
            }
            queue!(
                writer,
                Print(" "),
                SetBackgroundColor(terminal_color(palette_color.color)),
                Print("  "),
                ResetColor,
                Print(format!(" {}", palette_color.name))
            )?;
            used_cols += swatch_width;
        }
    }
    queue!(writer, ResetColor)?;
    Ok(())
}

fn status_prefix(state: &AppState) -> String {
    match state.mode {
        AppMode::Browse => format!(
            "Browse | Zoom {:.0}% | Rot {}° | {} {}",
            state.view_transform.zoom * 100.0,
            state.view_transform.rotation as u32 * 90,
            state.output_format,
            state.export_size
        ),
        AppMode::Draw => format!(
            "Tool {}:{} | Size {} | Color {} | {} {} | Palette",
            tool_shortcut(state.tool),
            tool_label(state.tool),
            state.width.label(),
            color_to_hex(state.color),
            state.output_format,
            state.export_size
        ),
    }
}

fn input_row_text(state: &AppState) -> String {
    if state.save_prompt.is_some() {
        return String::from("Save changes? (y)es / (n)o / s(a)ve as");
    }
    match &state.input_mode {
        InputMode::Color { buffer } => {
            format!("Color> {buffer}  Enter apply, Esc cancel")
        }
        InputMode::Text { buffer, .. } => {
            format!("Text> {buffer}  Enter apply, Esc cancel")
        }
        InputMode::SavePath { buffer } => {
            format!("Save as path> {buffer}  Enter save, Esc cancel")
        }
        InputMode::None => match state.mode {
            AppMode::Browse => format!(
                "{} | p=draw +/-=zoom arrows=pan r=rot R=rot-ccw 0=reset q=quit",
                state.message
            ),
            AppMode::Draw => format!(
                "{} | f freehand r rectangle e ellipse a arrow t text h highlight x redact [ ] size c color C clear z undo p back q quit",
                state.message
            ),
        },
    }
}

fn palette_color_at_column(column: u16, state: &AppState, cols: u16) -> Option<PaletteColor> {
    let mut start = status_prefix(state).chars().count() as u16;
    if start >= cols {
        return None;
    }

    for palette_color in PALETTE {
        let width = palette_swatch_width(palette_color);
        let end = start.saturating_add(width);
        if end > cols {
            return None;
        }
        if column >= start && column < end {
            return Some(palette_color);
        }
        start = end;
    }

    None
}

fn palette_swatch_width(palette_color: PaletteColor) -> u16 {
    palette_color.name.len() as u16 + 4
}

fn tool_label(tool: DrawingTool) -> &'static str {
    match tool {
        DrawingTool::Freehand => "freehand",
        DrawingTool::Rectangle => "rectangle",
        DrawingTool::Ellipse => "ellipse",
        DrawingTool::Arrow => "arrow",
        DrawingTool::Text => "text",
        DrawingTool::Highlighter => "highlight",
        DrawingTool::Redaction => "redact",
    }
}

fn tool_shortcut(tool: DrawingTool) -> char {
    match tool {
        DrawingTool::Freehand => 'f',
        DrawingTool::Rectangle => 'r',
        DrawingTool::Ellipse => 'e',
        DrawingTool::Arrow => 'a',
        DrawingTool::Text => 't',
        DrawingTool::Highlighter => 'h',
        DrawingTool::Redaction => 'x',
    }
}

fn parse_color(value: &str) -> Option<Rgba<u8>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    match value.to_ascii_lowercase().as_str() {
        "black" => Some(Rgba([0, 0, 0, 255])),
        "white" => Some(Rgba([255, 255, 255, 255])),
        "red" => Some(Rgba([255, 0, 0, 255])),
        "orange" => Some(Rgba([255, 128, 0, 255])),
        "yellow" => Some(Rgba([255, 221, 0, 255])),
        "green" => Some(Rgba([0, 180, 80, 255])),
        "cyan" => Some(Rgba([0, 190, 220, 255])),
        "blue" => Some(Rgba([30, 100, 255, 255])),
        "purple" => Some(Rgba([160, 80, 220, 255])),
        "pink" => Some(Rgba([255, 96, 170, 255])),
        "magenta" => Some(Rgba([220, 0, 220, 255])),
        "gray" | "grey" => Some(Rgba([128, 128, 128, 255])),
        _ => None,
    }
}

fn parse_hex_color(hex: &str) -> Option<Rgba<u8>> {
    if !hex.is_ascii() {
        return None;
    }
    match hex.len() {
        3 => {
            let mut chars = hex.chars();
            let red = parse_hex_nibble(chars.next()?)?;
            let green = parse_hex_nibble(chars.next()?)?;
            let blue = parse_hex_nibble(chars.next()?)?;
            Some(Rgba([red * 17, green * 17, blue * 17, 255]))
        }
        6 => {
            let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Rgba([red, green, blue, 255]))
        }
        _ => None,
    }
}

fn parse_hex_nibble(ch: char) -> Option<u8> {
    ch.to_digit(16).map(|value| value as u8)
}

fn color_to_hex(color: Rgba<u8>) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

fn terminal_color(color: Rgba<u8>) -> Color {
    Color::Rgb {
        r: color[0],
        g: color[1],
        b: color[2],
    }
}

fn truncate_to_cols(text: &str, cols: u16) -> String {
    text.chars().take(cols as usize).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{canvas::Point, terminal::TerminalMetrics, theme::ThemeMode};

    fn canvas() -> DrawingCanvas {
        DrawingCanvas::blank(
            TerminalMetrics::from_dimensions(10, 5, 100, 50),
            ThemeMode::Dark,
        )
    }

    fn state() -> AppState {
        AppState::new(
            Rgba([255, 255, 255, 255]),
            ExportFormat::Png,
            ExportSize::Canvas,
            None,
        )
    }

    fn layout() -> TerminalLayout {
        TerminalLayout::from_display_dimensions(10, 7, 100, 70, 1.0)
    }

    #[test]
    fn planned_shortcuts_update_canvas() {
        let mut canvas = canvas();
        let mut state = state();
        // Switch to Draw mode first, since Browse mode intercepts 'r' for rotation
        state.mode = AppMode::Draw;
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert_eq!(state.tool, DrawingTool::Rectangle);
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert_eq!(state.tool, DrawingTool::Ellipse);
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert_eq!(state.tool, DrawingTool::Freehand);
        for (shortcut, tool) in [
            ('a', DrawingTool::Arrow),
            ('t', DrawingTool::Text),
            ('h', DrawingTool::Highlighter),
            ('x', DrawingTool::Redaction),
        ] {
            assert!(!handle_key(
                KeyEvent::new(KeyCode::Char(shortcut), KeyModifiers::NONE),
                &mut canvas,
                &mut state
            ));
            assert_eq!(state.tool, tool);
        }
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert_eq!(state.width, WidthPreset::Large);
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert_eq!(state.width, WidthPreset::Medium);
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert!(matches!(state.input_mode, InputMode::Color { .. }));
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert_eq!(state.input_mode, InputMode::None);
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT),
            &mut canvas,
            &mut state
        ));
        assert!(handle_key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert!(handle_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert!(handle_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &mut canvas,
            &mut state
        ));
    }

    #[test]
    fn mouse_events_create_completed_stroke() {
        let mut canvas = canvas();
        let mut state = state();
        state.mode = AppMode::Draw;
        let layout = layout();
        let mut mouse_mapper = MouseMapper {
            mode: MouseCoordinateMode::Cell,
        };
        assert!(handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
            &mut canvas,
            &mut state,
            layout,
            &mut mouse_mapper,
        ));
        assert!(handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: 8,
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
            &mut canvas,
            &mut state,
            layout,
            &mut mouse_mapper,
        ));
        assert!(handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 8,
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
            &mut canvas,
            &mut state,
            layout,
            &mut mouse_mapper,
        ));
        assert!(canvas.undo());
        let image = canvas.render();
        assert_eq!(*image.get_pixel(50, 10), image::Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn mouse_mapper_uses_pixel_mode_for_large_coordinates() {
        let canvas = canvas();
        let layout = layout();
        let mut mapper = MouseMapper {
            mode: MouseCoordinateMode::Cell,
        };
        let target = mapper.target_for_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 50,
                row: 25,
                modifiers: KeyModifiers::NONE,
            },
            layout,
            &canvas,
        );
        assert_eq!(target, MouseTarget::Canvas(Point::new(0.5, 0.5)));
        assert_eq!(mapper.mode, MouseCoordinateMode::Pixel);
    }

    #[test]
    fn direct_canvas_points_are_available_for_tests() {
        let mut canvas = canvas();
        canvas.begin_stroke(Point::new(0.0, 0.0));
        canvas.finish_current();
        assert!(canvas.undo());
    }

    #[test]
    fn color_input_accepts_names_and_hex_values() {
        assert_eq!(parse_color("red"), Some(Rgba([255, 0, 0, 255])));
        assert_eq!(parse_color("BLUE"), Some(Rgba([30, 100, 255, 255])));
        assert_eq!(parse_color("#0f0"), Some(Rgba([0, 255, 0, 255])));
        assert_eq!(parse_color("#112233"), Some(Rgba([17, 34, 51, 255])));
        assert_eq!(parse_color("not-a-color"), None);
    }

    #[test]
    fn color_prompt_applies_valid_color_and_keeps_invalid_input_open() {
        let mut canvas = canvas();
        let mut state = state();
        state.mode = AppMode::Draw;
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        for ch in "bad".chars() {
            assert!(!handle_key(
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                &mut canvas,
                &mut state
            ));
        }
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert!(matches!(state.input_mode, InputMode::Color { .. }));
        assert_eq!(state.color, Rgba([255, 255, 255, 255]));
        assert!(state.message.contains("Unknown color"));

        state.input_mode = InputMode::Color {
            buffer: String::new(),
        };
        for ch in "#123456".chars() {
            assert!(!handle_key(
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                &mut canvas,
                &mut state
            ));
        }
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert_eq!(state.input_mode, InputMode::None);
        assert_eq!(state.color, Rgba([18, 52, 86, 255]));
    }

    #[test]
    fn text_prompt_commits_text_element() {
        let mut canvas = canvas();
        let mut state = state();
        state.set_tool(DrawingTool::Text);
        state.begin_text_input(Point::new(0.2, 0.2));
        for ch in "Hi".chars() {
            assert!(!handle_key(
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                &mut canvas,
                &mut state
            ));
        }
        assert!(!handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut canvas,
            &mut state
        ));
        assert_eq!(state.input_mode, InputMode::None);
        assert!(canvas.undo());
    }

    #[test]
    fn status_palette_click_changes_color_without_drawing() {
        let mut canvas = canvas();
        let mut state = state();
        state.mode = AppMode::Draw;
        let layout = TerminalLayout::from_display_dimensions(120, 7, 1200, 70, 1.0);
        let red_column = status_prefix(&state).chars().count() as u16
            + palette_swatch_width(PALETTE[0])
            + palette_swatch_width(PALETTE[1]);
        let mut mouse_mapper = MouseMapper {
            mode: MouseCoordinateMode::Cell,
        };

        assert!(handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: red_column,
                row: layout.status_row.unwrap(),
                modifiers: KeyModifiers::NONE,
            },
            &mut canvas,
            &mut state,
            layout,
            &mut mouse_mapper,
        ));

        assert_eq!(state.color, Rgba([255, 0, 0, 255]));
        assert!(!canvas.undo());
    }

    #[test]
    fn pixel_click_near_status_top_still_hits_palette() {
        let mut canvas = canvas();
        let mut state = state();
        state.mode = AppMode::Draw;
        let layout = TerminalLayout::from_display_dimensions(120, 7, 1200, 70, 1.0);
        let red_column = status_prefix(&state).chars().count() as u16
            + palette_swatch_width(PALETTE[0])
            + palette_swatch_width(PALETTE[1]);
        let pixel_column = red_column * 10 + 1;
        let status_top_pixel =
            (f32::from(layout.status_row.unwrap()) * layout.cell_height_px).floor() as u16;
        let mut mouse_mapper = MouseMapper {
            mode: MouseCoordinateMode::Pixel,
        };

        assert!(handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: pixel_column,
                row: status_top_pixel - 1,
                modifiers: KeyModifiers::NONE,
            },
            &mut canvas,
            &mut state,
            layout,
            &mut mouse_mapper,
        ));

        assert_eq!(state.color, Rgba([255, 0, 0, 255]));
        assert!(!canvas.undo());
    }
}
