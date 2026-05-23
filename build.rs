use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-check-cfg=cfg(ffmpeg_old_channel_layout)");

    // FF_API_OLD_CHANNEL_LAYOUT is gated on libavutil major < 59 (i.e. FFmpeg 6.x and
    // older). With it active the AVCodecParameters and AVFrame layouts include the
    // deprecated channel_layout/channels (plus several other) fields, so we need a
    // different Rust mirror of those structs. FFmpeg 7.x removed them and shifted the
    // remaining fields.
    if let Some(major) = pkg_config_major("libavutil") {
        if major < 59 {
            println!("cargo:rustc-cfg=ffmpeg_old_channel_layout");
        }
    }

    // Prefer pkg-config — it picks up the right library path and link names on both
    // Linux distros and Homebrew-on-macOS without hardcoding paths.
    if emit_pkg_config_libs() {
        return;
    }

    // Fallback: hand-rolled paths for environments without pkg-config (e.g. macOS where
    // brew's pkg-config isn't on PATH). Keep the original Homebrew hints.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("apple-darwin") {
        println!("cargo:rustc-link-search=native=/opt/homebrew/lib");
        println!("cargo:rustc-link-search=native=/usr/local/lib");
    }
    for lib in ["avcodec", "avformat", "avutil", "swscale", "swresample"] {
        println!("cargo:rustc-link-lib={}", lib);
    }
}

fn emit_pkg_config_libs() -> bool {
    let output = Command::new("pkg-config")
        .args([
            "--libs",
            "libavformat",
            "libavcodec",
            "libavutil",
            "libswscale",
            "libswresample",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for arg in stdout.split_whitespace() {
        if let Some(path) = arg.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={}", path);
        } else if let Some(name) = arg.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={}", name);
        }
    }
    true
}

fn pkg_config_major(lib: &str) -> Option<u32> {
    let output = Command::new("pkg-config")
        .args(["--modversion", lib])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout);
    version.split('.').next()?.trim().parse().ok()
}
