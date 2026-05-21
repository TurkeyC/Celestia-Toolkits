# kitim

**Turn your terminal into a fast, focused media viewer for images and videos without leaving the command line.**

(kitim only works in a Kitty-graphics-compatible terminal such as **Kitty**, **Ghostty**, **cmux**, **WezTerm**)

## Install

    cargo install kitim

(Video playing requires `ffmpeg` installed, `brew install ffmpeg` on MacOS)

---

## Why

- Stop bouncing from terminal to Finder, Preview, or a browser just to inspect a file.
- Preview images, animated GIFs, and videos in the same place you already work.
- Keep media checks scriptable, keyboard-first, and fast enough for real terminal workflows.

---

<!-- ## Show, Don't Tell

![kitim demo placeholder](./assets/demo.gif)
-->

## Key Capabilities

- **Inline visual previews** powered by the Kitty graphics protocol, so media appears directly in your terminal.
- **Video playback with synced audio** using FFmpeg decoding, CPAL output, and a render thread tuned for smooth playback.
- **Tiny controls, useful defaults**: zoom output, override playback FPS, and loop videos or GIFs when you need to inspect motion.

---

## Usage

```bash
kitim screenshot.png
kitim -z 0.7 clip.webm
kitim --loop --frame-rate 24 animation.gif
```

---

## How It Works

```text
file path -> image/GIF decoder or FFmpeg -> RGBA frames -> Kitty graphics chunks -> terminal
                                      video audio -> resampler -> CPAL output -> sync clock
```

`kitim` keeps the rendering path direct: decode into RGBA, resize to terminal cell geometry, chunk through the Kitty protocol, and use audio timing as the playback clock when a video has sound.

