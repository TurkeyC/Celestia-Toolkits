use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "celestia-wallpaper",
    about = "Video wallpaper player using mpv for wlroots-based Wayland compositors",
    after_help = "* The auto options might not work as intended\nSee the man page for more details"
)]
pub struct Args {
    #[arg(short = 'd', long = "help-output")]
    pub help_output: bool,

    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[arg(short, long)]
    pub fork: bool,

    #[arg(short = 'p', long = "auto-pause")]
    pub auto_pause: bool,

    #[arg(short = 's', long = "auto-stop")]
    pub auto_stop: bool,

    #[arg(short = 'n', long)]
    pub slideshow: Option<u32>,

    #[arg(short, long)]
    pub layer: Option<String>,

    #[arg(short = 'o', long = "mpv-options")]
    pub mpv_options: Option<String>,

    #[arg(short = 'Z', hide = true)]
    pub save_info: Option<String>,

    pub output: Option<String>,

    pub url_or_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceLayer {
    Background,
    Bottom,
    Top,
    Overlay,
}

impl SurfaceLayer {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "background" => Some(Self::Background),
            "overlay" => Some(Self::Overlay),
            _ => None,
        }
    }

    pub fn to_wlr_layer(self) -> wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer {
        use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
        match self {
            Self::Background => Layer::Background,
            Self::Bottom => Layer::Bottom,
            Self::Top => Layer::Top,
            Self::Overlay => Layer::Overlay,
        }
    }
}

pub fn parse_layer(args: &Args) -> SurfaceLayer {
    match &args.layer {
        Some(name) => match SurfaceLayer::from_str_opt(name) {
            Some(layer) => layer,
            None => {
                log_error!(
                    "{} is not a shell surface layer\nYour options are: top, bottom, background and overlay",
                    name
                );
                std::process::exit(1);
            }
        },
        None => SurfaceLayer::Background,
    }
}

pub fn validate_args(args: &Args) -> (String, String) {
    let mpv_opts = args.mpv_options.as_deref().unwrap_or("");

    let playlist_in_opts = mpv_opts.contains("--playlist=");

    if playlist_in_opts {
        if args.output.is_none() {
            log_error!("Not enough args passed\nPlease set output");
            std::process::exit(1);
        }
        let output = args.output.clone().unwrap();
        let video_path = extract_playlist_path(mpv_opts);
        (output, video_path)
    } else {
        if args.output.is_none() || args.url_or_path.is_none() {
            log_error!("Not enough args passed\nPlease set output and url|path filename");
            std::process::exit(1);
        }
        (
            args.output.clone().unwrap(),
            args.url_or_path.clone().unwrap(),
        )
    }
}

fn extract_playlist_path(mpv_opts: &str) -> String {
    let opts_with_newlines = mpv_opts.replace(' ', "\n");
    for line in opts_with_newlines.lines() {
        if let Some(path) = line.strip_prefix("--playlist=") {
            return path.to_string();
        }
    }
    unreachable!("playlist_in_opts was true but no --playlist= found")
}

pub fn mpv_options_to_config(mpv_opts: &str) -> String {
    mpv_opts.replace(' ', "\n")
}
