mod audio_player;
mod config;
mod ffmpeg;
mod image_viewer;
mod kitty;
mod playback_control;
mod video_player;

use clap::Parser;
use config::Config;
use std::path::PathBuf;

fn main() {
    let config = Config::parse();

    for file in &config.files {
        let path = std::path::Path::new(file);
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Common video formats (including GIF, which uses the video player TUI)
        let is_video_ext = match ext.as_str() {
            "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" | "mpg" | "mpeg"
            | "3gp" | "gif" => true,
            _ => false,
        };

        // Common audio formats
        let is_audio_ext = match ext.as_str() {
            "mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" | "opus" | "wma" | "alac" | "aiff"
            | "aif" => true,
            _ => false,
        };

        // Known image formats — route to kitdraw TUI
        let is_image_ext = match ext.as_str() {
            "jpg" | "jpeg" | "png" | "bmp" | "tiff" | "tif" | "webp" | "ico" | "avif" => true,
            _ => false,
        };

        let result = if is_video_ext {
            // GIF files loop forever; regular videos play once and pause at end
            if ext == "gif" {
                let mut gif_config = config.clone();
                gif_config.loop_video = true;
                video_player::play(&gif_config, file)
            } else {
                video_player::play(&config, file)
            }
        } else if is_audio_ext {
            audio_player::play(&config, file)
        } else if is_image_ext {
            // Open in kitdraw TUI (browse mode with zoom/pan/rotate/draw)
            let kitdraw_result = kitdraw::run(kitdraw::app::AppConfig {
                input_image: Some(path.to_path_buf()),
                output: PathBuf::from(format!("{}-kitdraw.png", file)),
                output_format: kitdraw::export::ExportFormat::Png,
                export_size: kitdraw::export::ExportSize::Original,
                theme: kitdraw::theme::ThemeMode::Dark,
                fallback_cell_px: kitdraw::args::CellPixels { width: 10, height: 20 },
                resolution_scale: 0.5,
            });
            match kitdraw_result {
                Ok(()) => Ok(()),
                Err(e) => Err(format!("kitdraw: {}", e).into()),
            }
        } else {
            // Fallback: try image viewer, then FFmpeg video, then audio
            match image_viewer::view(&config, file) {
                Ok(_) => Ok(()),
                Err(_) => match video_player::play(&config, file) {
                    Ok(_) => Ok(()),
                    Err(_) => audio_player::play(&config, file),
                },
            }
        };

        if let Err(e) = result {
            eprintln!("Error processing {}: {}", file, e);
        }
    }
}
