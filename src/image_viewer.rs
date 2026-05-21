use crate::config::Config;
use crate::kitty;
use image::{AnimationDecoder, GenericImageView};
use std::fs::File;
use std::time::Duration;

pub fn view(config: &Config, file_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Query terminal dimensions (columns, rows)
    let (term_cols, _term_rows) = crossterm::terminal::size().unwrap_or((80, 24));

    // Load static image first (to determine dimensions/format)
    let img = image::open(file_path)?;
    let (img_w, img_h) = img.dimensions();

    // Calculate display dimensions in cells
    let target_cols = ((term_cols as f32) * config.zoom) as u32;
    let target_cols = std::cmp::max(1, target_cols);

    // Aspect ratio preservation (assuming terminal cell width:height ratio of 1:2)
    let target_rows = (((target_cols as f32 * img_h as f32) / img_w as f32) * 0.5) as u32;
    let target_rows = std::cmp::max(1, target_rows);

    // High resolution target pixel dimensions for the terminal cells
    let target_w_px = target_cols * 10;
    let target_h_px = target_rows * 20;

    // Check if the file is an animated GIF
    let file = File::open(file_path)?;
    let is_gif = image::guess_format(&std::fs::read(file_path)?)? == image::ImageFormat::Gif;

    if is_gif {
        let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(file))?;
        let frames: Vec<_> = decoder.into_frames().collect_frames()?;

        // Pre-allocate terminal rows to prevent scrolling drift during playback
        for _ in 0..target_rows {
            println!();
        }
        kitty::move_up_robust(target_rows as u16)?;

        'outer: loop {
            for frame in &frames {
                let buffer: &image::RgbaImage = frame.buffer();
                let frame_img = image::DynamicImage::ImageRgba8(buffer.clone());
                let resized = frame_img.resize_exact(target_w_px, target_h_px, image::imageops::FilterType::Nearest);
                let rgba = resized.to_rgba8();

                // Draw using the Kitty protocol with prevent_cursor_move = true
                kitty::write_rgba_frame(&rgba, target_w_px, target_h_px, target_cols, target_rows, true)?;

                let delay = match config.fps {
                    Some(fps) => Duration::from_secs_f32(1.0 / fps),
                    None => Duration::from(frame.delay()),
                };
                std::thread::sleep(delay);
            }

            if !config.loop_video {
                break 'outer;
            }
        }

        // Move cursor to bottom-left after GIF ends
        let mut stdout = std::io::stdout().lock();
        let mut buf = Vec::new();
        let _ = crossterm::queue!(
            buf,
            crossterm::cursor::MoveDown(target_rows as u16),
            crossterm::cursor::MoveToColumn(0)
        );
        let _ = kitty::write_all_robust(&mut stdout, &buf);
        let _ = kitty::flush_robust(&mut stdout);
        println!();
    } else {
        // Static image
        let resized = img.resize_exact(target_w_px, target_h_px, image::imageops::FilterType::Triangle);
        let rgba = resized.to_rgba8();

        kitty::write_rgba_frame(&rgba, target_w_px, target_h_px, target_cols, target_rows, false)?;
        println!(); // Add trailing newline for static output
    }

    Ok(())
}
