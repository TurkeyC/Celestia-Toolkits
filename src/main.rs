mod config;
mod ffmpeg;
mod image_viewer;
mod kitty;
mod video_player;

use clap::Parser;
use config::Config;
use std::path::Path;

fn main() {
    let config = Config::parse();

    for file in &config.files {
        let path = Path::new(file);
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Common video formats
        let is_video_ext = match ext.as_str() {
            "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" | "mpg" | "mpeg" | "3gp" => true,
            _ => false,
        };

        let result = if is_video_ext {
            video_player::play(&config, file)
        } else {
            // Try as image, fallback to video if it fails
            match image_viewer::view(&config, file) {
                Ok(_) => Ok(()),
                Err(_) => {
                    // Try fallback to video decoder
                    video_player::play(&config, file)
                }
            }
        };

        if let Err(e) = result {
            eprintln!("Error processing {}: {}", file, e);
        }
    }
}
