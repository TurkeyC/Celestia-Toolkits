# kitim

**Turn your terminal into a fast, focused media viewer for images, audio, and videos without leaving the command line.**

(Part of kit* series of graphic terminal apps:
[kitim](https://github.com/wensheng/kitim)
[kitmd](https://github.com/wensheng/kitmd)
[kitpdf](https://github.com/wensheng/kitpdf)
[kitdraw](https://github.com/wensheng/kitdraw)
[kitDOOM](https://github.com/wensheng/kitdoom)) 

***kitim runs on terminals that supports the Kitty graphics protocol:
[**Ghostty**](https://ghostty.org/),
[**Kitty**](https://sw.kovidgoyal.net/kitty/),
[**WezTerm**](https://wezterm.net/),
[**cmux**](https://github.com/manaflow-ai/cmux),
[**Warp**](https://warp.dev/),
[**iTerm2**](https://www.iterm2.com/) (see notes below for iTerm2).***

[![demo]](https://github.com/user-attachments/assets/acd301e1-b749-446a-ab40-67845a13a6bb)

## Install

    cargo install kitim

(Audio and video playback require `ffmpeg` installed, `brew install ffmpeg` on MacOS)

---

## Why

- Stop bouncing from terminal to Finder, Preview, or a browser just to inspect a file.
- Preview images, animated GIFs, videos, and audio in the same place you already work.
- Keep media checks scriptable, keyboard-first, and fast enough for real terminal workflows.

---

## Key Capabilities

- **Inline visual previews** powered by the Kitty graphics protocol, so media appears directly in your terminal.
- **Image TUI** with dual Browse/Draw modes — zoom, pan, rotate, and annotate images interactively.
- **Video playback with synced audio** using FFmpeg decoding, CPAL output, and a render thread tuned for smooth playback.
- **Audio-only playback** for common formats such as MP3, M4A, AAC, WAV, FLAC, OGG, and Opus.
- **Tiny controls, useful defaults**: zoom output, override playback FPS, and loop videos, GIFs, or audio when you need to inspect media.

---

## Image TUI — Browse & Draw Modes

When you open an image (`kitim photo.png`), `kitim` launches the
[kitdraw](https://github.com/wensheng/kitdraw) TUI with two modes.

### Browse mode — zoom, pan, rotate

| Key / Mouse | Action |
|-------------|--------|
| `+` / `=`  | Zoom in  (1.1×) |
| `-`         | Zoom out |
| Scroll wheel | Zoom in/out (1.05× per tick) |
| Drag left button | Pan — **1:1 sticky tracking** at any zoom |
| Arrow keys | Pan by 5 % step |
| `r` / `R` | Rotate 90° CW / CCW |
| `0` | Reset to 100 %, centred |
| `p` | Switch to Draw mode |
| `q` / Esc | Quit |

### Draw mode — annotate

| Key | Tool |
|-----|------|
| `f` | Freehand pen |
| `r` | Rectangle outline |
| `e` | Ellipse outline |
| `a` | Arrow |
| `t` | Text (click to place, type, Enter) |
| `h` | Highlighter (semi-transparent) |
| `x` | Redaction (opaque black) |
| `c` | Pick stroke colour |
| `[` / `]` | Decrease / increase stroke width |
| `z` | Undo last stroke |
| `C` | Clear all annotations |
| `p` | Back to Browse (prompts to save) |
| `q` / Esc | Quit (prompts to save) |

### Performance notes

Pan operations are cached client-side so that only the cheap
`apply_pan` letterbox runs per frame (no Triangle resize).
Full zoom/rotate operations still re-encode the frame (50–200 ms).

For a deep dive into the rendering pipeline and the 1:1 sticky-pan
formula see [`docs/browse-mode.md`](docs/browse-mode.md).

---

## Usage

```bash
kitim screenshot.png          # opens in interactive Browse TUI
kitim song.mp3                # audio-only playback
kitim -z 1.5 clip.webm        # video with 1.5× zoom
kitim --loop --frame-rate 24 animation.gif   # GIF: loop, 24 FPS
```

`-z`/`--zoom` is a multiplier on the original media size. Media displays at
original size by default, and shrinks to fit the terminal with a small margin
when it would otherwise exceed the available screen area.

---

## How It Works

```text
Images → kitdraw TUI (Browse: zoom/pan/rotate + Draw: annotate)
GIFs   → video player (loop, overlay controls)
Videos → FFmpeg decode → RGBA frames → Kitty chunks → terminal
Audio  → decoder      → resampler    → CPAL playback
```

`kitim` keeps the rendering path direct: decode into RGBA, resize to terminal cell geometry, chunk through the Kitty protocol, and use audio timing as the playback clock when a video has sound.

## Notes

For iTerm2, set:

```bash
export TERMINAL_KITTEN_GRAPHICS=1
```

in your `.zshrc` or `.bashrc`.
