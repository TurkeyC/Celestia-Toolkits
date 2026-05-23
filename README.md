# kitim

**Turn your terminal into a fast, focused media viewer for images, audio, and videos without leaving the command line.**

(kitim only works in a Kitty-graphics-compatible terminal such as **Kitty**, **Ghostty**, **cmux**, **WezTerm**)

## Install

    cargo install kitim

(Audio and video playback require `ffmpeg` installed, `brew install ffmpeg` on MacOS)

---

## Why

- Stop bouncing from terminal to Finder, Preview, or a browser just to inspect a file.
- Preview images, animated GIFs, videos, and audio in the same place you already work.
- Keep media checks scriptable, keyboard-first, and fast enough for real terminal workflows.

---

<!-- ## Show, Don't Tell

![kitim demo placeholder](./assets/demo.gif)
-->

## Key Capabilities

- **Inline visual previews** powered by the Kitty graphics protocol, so media appears directly in your terminal.
- **Video playback with synced audio** using FFmpeg decoding, CPAL output, and a render thread tuned for smooth playback.
- **Audio-only playback** for common formats such as MP3, M4A, AAC, WAV, FLAC, OGG, and Opus.
- **Tiny controls, useful defaults**: zoom output, override playback FPS, and loop videos, GIFs, or audio when you need to inspect media.

---

## Usage

```bash
kitim screenshot.png
kitim song.mp3
kitim -z 0.7 clip.webm
kitim --loop --frame-rate 24 animation.gif
```

---

## How It Works

```text
file path -> image/GIF decoder or FFmpeg -> RGBA frames -> Kitty graphics chunks -> terminal
                                      audio -> resampler -> CPAL output
                                      video audio -> resampler -> CPAL output -> sync clock
```

`kitim` keeps the rendering path direct: decode into RGBA, resize to terminal cell geometry, chunk through the Kitty protocol, and use audio timing as the playback clock when a video has sound.
