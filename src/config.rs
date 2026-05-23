use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "viewim",
    version,
    about = "Kitty graphics-based terminal image, audio, and video player"
)]
pub struct Config {
    /// The image, audio, or video files to display/play
    #[arg(required = true)]
    pub files: Vec<String>,

    /// Zoom level multiplier to resize the output (default: 0.4)
    #[arg(short = 'z', long = "zoom", default_value_t = 0.4)]
    pub zoom: f32,

    /// Optional frame rate override for video playback (fps)
    #[arg(short = 'f', long = "frame-rate")]
    pub fps: Option<f32>,

    /// Loop video, GIF, or audio playback continuously
    #[arg(short = 'l', long = "loop")]
    pub loop_video: bool,
}
