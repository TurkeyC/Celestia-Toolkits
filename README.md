<h1>
<p align="center">
  📑
  <br>Celestia-PDF-Reader
</h1>
  <p align="center">
    PDF viewer for terminals using the Kitty image protocol
    <br />
  </p>
</p>

> **Note:** This project is a modified fork of [freref/fancy-cat](https://github.com/freref/fancy-cat).

## Usage

```sh
celestia-pdf-reader <path-to-pdf> [page]
```

Run `celestia-pdf-reader --help` for the full list of options, keybindings, and commands.

### Commands

Celestia-PDF-Reader uses a modal interface similar to Neovim. There are two modes: view mode and command mode. To enter command mode you type `:` by default (this can be changed in the config file).

Documentation on the available commands can be found [here](./docs/commands.md).

### Configuration

Celestia-PDF-Reader can be configured through a JSON configuration file located in one of several locations (primary `$XDG_CONFIG_HOME/fancy-cat/config.json`, fallback `$HOME/.config/fancy-cat/config.json`, legacy `$HOME/.fancy-cat`). An empty configuration file is automatically created in the primary or fallback location on the first run.

An example `config.json` and documentation can be found [here](./docs/config.md).

## Build Instructions

### Requirements

- Zig version `0.15.2`
- Terminal emulator with the Kitty image protocol (e.g. Kitty, Ghostty, WezTerm, etc.)

### Build

1. Fetch submodules:

```
git submodule update --init --recursive
```

2. Build the project:

```sh
zig build --release=small
```

> [!NOTE]
> There is a [known issue](https://github.com/freref/fancy-cat/issues/18) with some processors; if the build fails on step 7/10 with the error `LLVM ERROR: Do not know how to expand the result of this operator!` then try the command below instead:
>
> ```sh
> zig build -Dcpu="skylake" --release=small
> ```

3. Install:

```sh
# Add to your PATH
# Linux
mv zig-out/bin/fancy-cat ~/.local/bin/

# macOS
mv zig-out/bin/fancy-cat /usr/local/bin/
```

### Run

```sh
zig build run -- <path-to-pdf> [page]
```

## Features

- ✅ Mouse interaction (scroll, drag, zoom)
- ✅ Filewatch (hot-reload)
- ✅ Runtime config
- ✅ Custom keymappings
- ✅ Modal interface
- ✅ Commands
- ✅ Colorize mode (dark-mode)
- ✅ Status bar
- ✅ Page navigation (zoom, prev, next, etc.)

## License

[AGPL-3.0-or-later](https://spdx.org/licenses/AGPL-3.0-or-later.html)

## Contributing

Contributions are welcome.
