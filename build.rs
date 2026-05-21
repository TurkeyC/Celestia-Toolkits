fn main() {
    // Add macOS homebrew paths for dynamic linking
    println!("cargo:rustc-link-search=native=/opt/homebrew/lib");
    println!("cargo:rustc-link-search=native=/opt/homebrew/Cellar/ffmpeg/8.1/lib");

    // Link required FFmpeg libraries
    println!("cargo:rustc-link-lib=avcodec");
    println!("cargo:rustc-link-lib=avformat");
    println!("cargo:rustc-link-lib=avutil");
    println!("cargo:rustc-link-lib=swscale");
    println!("cargo:rustc-link-lib=swresample");
}
