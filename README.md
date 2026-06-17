# Celestia-WallPaper

> 基于 wlroots 的 Wayland 合成器视频壁纸播放器 —— 用 Rust 完全重写的 [mpvpaper](https://github.com/GhostNaN/mpvpaper)。

本项目是原版 C 语言 mpvpaper 的 Rust 重写（更名为 **Celestia-WallPaper**），包含两个独立二进制文件：`celestia-wallpaper`（主程序）和 `celestia-wallpaper-holder`（占位程序）。功能与原版完全对齐，代码更加安全、可维护。

> **关于重命名**：原项目名 `mpvpaper` 继续在代码内部（如 crate 间的路径引用）使用，但对外发布名已更名为 Celestia-WallPaper。
> 如果你之前使用原版 C mpvpaper，本程序提供完全相同的命令行接口，二进制名改为 `celestia-wallpaper` 即可。

---

## 目录

- [项目概述](#项目概述)
- [与原版 C mpvpaper 的对比](#与原版-c-mpvpaper-的对比)
- [系统要求](#系统要求)
- [构建依赖](#构建依赖)
- [编译与安装](#编译与安装)
- [使用方法](#使用方法)
  - [celestia-wallpaper](#celestia-wallpaper)
  - [celestia-wallpaper-holder](#celestia-wallpaper-holder)
- [命令行参数详解](#命令行参数详解)
- [配置文件](#配置文件)
  - [pauselist（自动暂停列表）](#pauselist自动暂停列表)
  - [stoplist（自动停止列表）](#stoplist自动停止列表)
- [自动功能说明](#自动功能说明)
  - [自动暂停（-p）](#自动暂停-p)
  - [自动停止（-s）](#自动停止-s)
- [图层（Layer）说明](#图层layer说明)
- [架构概览](#架构概览)
  - [工作区结构](#工作区结构)
  - [模块说明](#模块说明)
  - [线程模型](#线程模型)
  - [主循环](#主循环)
- [技术栈](#技术栈)
- [常见问题](#常见问题)
- [许可证](#许可证)

---

## 项目概述

Celestia-WallPaper 是一个在 Wayland 桌面上播放视频壁纸的工具。它通过 `wlr-layer-shell` 协议将 mpv 的渲染输出直接绘制到桌面背景层，支持：

- 播放本地视频或网络流
- 多显示器支持
- 自动暂停/停止（当其他窗口遮挡壁纸时节省资源）
- 幻灯片模式（定时切换壁纸）
- 播放列表支持
- 可配置的图层（background / bottom / top / overlay）

---

## 与原版 C mpvpaper 的对比

| 特性 | C 原版 | Rust 重写（Celestia-WallPaper） |
|------|--------|-----------|
| 语言 | C | Rust |
| Wayland 绑定 | 手写 C 绑定 | `wayland-client` crate（类型安全） |
| EGL 加载 | 直接链接 | `khronos-egl` 动态加载（更灵活） |
| mpv 绑定 | 手写 FFI | `libmpv-sys` crate |
| 内存安全 | 手动管理 | Rust 所有权系统 |
| 线程安全 | 无保证 | `Send`/`Sync` 编译期检查 |
| CLI 解析 | getopt | clap derive（类型安全） |
| 错误处理 | 返回值检查 | `Result` + `?` 操作符 |
| 安全退出 | `execv`/`exit` 无清理 | 主循环 break + 所有 Drop 自动释放 |
| 进程检测 | `pidof` 子进程（每次 spawn） | 直接扫描 `/proc/*/comm` |
| 事件等待 | `wait_event(0)` + `sleep(10ms)` | `wait_event(0.05)` 单调用 |
| 帧同步 | 帧回调 Done 内直接 `render()` | 写入 `eventfd` 唤醒主循环再渲染 |

---

## 系统要求

- **操作系统**：Linux（基于 wlroots 的 Wayland 合成器，如 Sway、Hyprland、river 等）
- **Rust**：1.70+（推荐使用 `rustup` 安装）
- **mpv**：已安装并可在 PATH 中找到（运行时依赖，非编译依赖）

---

## 构建依赖

编译前需要安装以下系统包（以 Fedora 为例）：

```bash
sudo dnf install wayland-devel mesa-libEGL-devel mpv-devel luajit-devel
```

> `luajit-devel` 仅当需要使用 Spine Lua 脚本功能时才需要。如果没有安装，可退回到无 Lua 模式使用。

其他发行版对应的包名：

| 发行版 | 包名 |
|--------|------|
| Arch Linux | `wayland` `mesa` `mpv` `luajit` |
| Ubuntu/Debian | `libwayland-dev` `libegl-dev` `libmpv-dev` `libluajit-5.1-dev` |
| openSUSE | `wayland-devel` `Mesa-libEGL-devel` `mpv-devel` `luajit-devel` |

---

## 编译与安装

```bash
# 进入项目目录
cd mpvpaper-rust/

# 编译（Debug 模式）
cargo build

# 编译（Release 模式，推荐）
cargo build --release

# 安装到系统（可选）
sudo cp target/release/celestia-wallpaper /usr/local/bin/
sudo cp target/release/celestia-wallpaper-holder /usr/local/bin/
```

编译产物位于：
- `target/release/celestia-wallpaper` — 主程序
- `target/release/celestia-wallpaper-holder` — 占位程序

### RPM 构建（Fedora）

```bash
# 生成 tarball
git archive --prefix=celestia-wallpaper-1.8.0/ -o ~/rpmbuild/SOURCES/celestia-wallpaper-1.8.0.tar.gz HEAD

# 构建 SRPM
rpmbuild -bs celestia-wallpaper.spec

# 用 mock 构建 RPM
mock -r fedora-43-x86_64 --rebuild ~/rpmbuild/SRPMS/celestia-wallpaper-1.8.0-1.fc43.src.rpm
```

---

## 使用方法

### celestia-wallpaper

基本用法：

```bash
# 在指定显示器上播放视频
celestia-wallpaper <OUTPUT> <视频路径或URL>

# 示例：在 DP-1 上播放本地视频
celestia-wallpaper DP-1 /path/to/video.mp4

# 示例：在 HDMI-A-1 上播放网络视频
celestia-wallpaper HDMI-A-1 https://example.com/video.mp4

# 查看所有可用显示器
celestia-wallpaper -d

# 启用详细日志
celestia-wallpaper -vv DP-1 video.mp4

# 使用播放列表
celestia-wallpaper -o "--playlist=/path/to/playlist.m3u" DP-1

# 设置图层为 bottom
celestia-wallpaper -l bottom DP-1 video.mp4

# 播放 Spine 2D 骨架动画壁纸（直接使用json或者skel）
celestia-wallpaper DP-1 /path/to/model.json
celestia-wallpaper DP-1 /path/to/model.skel

# 播放 Spine 2D 骨架动画壁纸（TOML 配置）
celestia-wallpaper DP-1 /path/to/model.spine.toml

# 播放 Spine 壁纸（Lua 脚本配置，支持状态机逻辑）
celestia-wallpaper DP-1 /path/to/model.spine.lua

# 后台运行（fork 模式）
celestia-wallpaper -f DP-1 video.mp4
```

#### 使用项目中的示例文件快速体验

```bash
# 示例视频
./target/release/celestia-wallpaper -t video -s eDP-1 -l background -o "no-audio loop" ./asstes/video-example/Wanderer.mp4

# 示例图片
./target/release/celestia-wallpaper -t picture -s eDP-1 -l background ./asstes/picture-example/nature.png

# 示例spine2D
# 使用skel
./target/release/celestia-wallpaper -t spine -s eDP-1 -l background ./asstes/spine-example/Maki_home.skel
# 使用json
./target/release/celestia-wallpaper -t spine -s eDP-1 -l background ./asstes/spine-example/Maki_home.json
# 使用toml
./target/release/celestia-wallpaper -t spine -s eDP-1 -l background ./asstes/spine-example/Maki_home.spine.toml
# 使用lua
./target/release/celestia-wallpaper -t spine -s eDP-1 -l background ./asstes/spine-example/Maki_home.spine.lua
```

### celestia-wallpaper-holder

`celestia-wallpaper-holder` 通常不需要手动运行。它由 `celestia-wallpaper` 在自动停止模式下自动启动，用于在壁纸暂停时"占住"壁纸表面，防止合成器回收该图层。

```bash
# 手动运行（调试用）
celestia-wallpaper-holder <OUTPUT>
```

---

## 命令行参数详解

| 参数 | 长参数 | 说明 |
|------|--------|------|
| `-d` | `--help-output` | 列出所有可用的 Wayland 输出（显示器）并退出 |
| `-v` | `--verbose` | 增加日志详细程度，可叠加使用（`-v`、`-vv`、`-vvv`） |
| `-f` | `--fork` | 启动后 fork 到后台运行 |
| `-p` | `--auto-pause` | 启用自动暂停：当壁纸被其他窗口完全遮挡时暂停播放 |
| `-s` | `--auto-stop` | 启用自动停止：当壁纸被遮挡时停止并退出进程 |
| `-n <秒数>` | `--slideshow <秒数>` | 幻灯片模式：每隔指定秒数切换到播放列表中的下一个视频 |
| `-l <图层>` | `--layer <图层>` | 设置壁纸图层，可选：`background`（默认）、`bottom`、`top`、`overlay` |
| `-o <选项>` | `--mpv-options <选项>` | 传递额外选项给 mpv（用空格分隔，需要引号包裹） |
| — | `<OUTPUT>` | 目标显示器名称（如 `DP-1`、`HDMI-A-1`、`eDP-1`） |
| — | `<URL_OR_PATH>` | 视频文件路径或网络 URL |

**关于 `-o` 参数的示例：**

```bash
# 设置循环播放和静音
celestia-wallpaper -o "--loop --mute=yes" DP-1 video.mp4

# 使用播放列表文件
celestia-wallpaper -o "--playlist=~/videos/list.m3u" DP-1
```

---

## 配置文件

Celestia-WallPaper 支持两个配置文件，位于 `~/.config/celestia-wallpaper/` 目录下：

### pauselist（自动暂停列表）

文件路径：`~/.config/celestia-wallpaper/pauselist`

当启用 `-p`（自动暂停）时，celestia-wallpaper 会检查此列表中的进程名。如果列表中的任一进程正在运行，celestia-wallpaper 会暂停播放。

```
# ~/.config/celestia-wallpaper/pauselist
firefox
chromium
steam
```

### stoplist（自动停止列表）

文件路径：`~/.config/celestia-wallpaper/stoplist`

当启用 `-s`（自动停止）时，celestia-wallpaper 会检查此列表中的进程名。如果列表中的任一进程正在运行，celestia-wallpaper 会完全停止并启动 `celestia-wallpaper-holder` 占位。

```
# ~/.config/celestia-wallpaper/stoplist
firefox
steam
gamescope
```

> **注意**：列表文件的格式为每行一个进程名，通过扫描 `/proc/*/comm` 检测进程是否存在（不依赖外部命令）。

---

## Spine 2D 骨架动画壁纸

Celestia-WallPaper 支持播放 [Spine](https://esotericsoftware.com/) 2D 骨骼动画壁纸，支持 `.json`（JSON 格式骨架）和 `.skel`（二进制格式骨架）文件，配合同名的 `.atlas` 纹理图集文件使用。

### 配置文件格式

Spine 壁纸支持两种配置方式：

#### 1. TOML 线性序列（`.spine.toml`）

适用于简单的动画序列播放：

```toml
skeleton = "Maki_home.json"

[[anim]]
name = "Start_Idle_01"
duration = 11.3

[[anim]]
name = "Idle_01"
loop_anim = true
duration = 0

[display]
offset_x = 0.0
offset_y = 0.0
scale = 0.0
```

| 字段 | 说明 |
|------|------|
| `skeleton` | 骨架文件路径（相对配置文件或绝对路径） |
| `anim` | 动画列表，按顺序播放，循环往复 |
| `anim[].name` | 动画名称（需在骨架中存在） |
| `anim[].loop_anim` | 是否循环播放 |
| `anim[].duration` | 播放时长（秒），`0` 表示播到自然结束（循环动画会永远播下去） |
| `display.scale` | 缩放，`0` 为自动填充视口（cover 模式） |
| `display.offset_x/y` | 偏移（骨架坐标） |

#### 2. Lua 脚本状态机（`.spine.lua`）

适用于需要复杂动画逻辑的场景，如概率跳转、多轨道叠加、事件驱动的状态机。需要安装 `luajit-devel` 编译。

```lua
skeleton = "Maki_home.json"
scale = 0.0
offset_x = 0.0
offset_y = 0.0

-- 状态机示例：开场 → idle → 随机说话（轨道叠加）
is_idle = false
state = "init"

function on_init(anim_table)
    if has_animation("Start_Idle_01") then
        state = "start"
        play(0, "Start_Idle_01", false)
    else
        is_idle = true
        state = "idle"
        play(0, "Idle_01", true)
    end
end

function on_complete(track, name)
    -- 开场播完 → idle
    if state == "start" and name == "Start_Idle_01" then
        is_idle = true
        state = "idle"
        play(0, "Idle_01", true)
    end
    -- 说话播完 → 淡出并回到 idle
    if state == "talking" and (track == 1 or track == 2) then
        empty(1, 0.2)
        empty(2, 0.2)
        is_idle = true
        state = "idle"
    end
end
```

### Lua API 参考

| 函数 | 说明 |
|------|------|
| `play(track, name, looping)` | 在指定轨道上播放动画 |
| `add(track, name, looping, delay)` | 排队动画（当前播完后播，delay=0 立即接） |
| `clear_track(track)` | 立即清空轨道 |
| `empty(track, mix_duration)` | 淡出轨道到无动画 |
| `animations()` | 返回所有动画名列表（1-indexed 表） |
| `has_animation(name)` | 检查动画是否存在 |
| `random_from({...})` | 从表中随机选一项 |

| 引擎回调 | 说明 |
|----------|------|
| `on_init(anim_table)` | 骨架加载完成后调用 |
| `on_update(dt)` | 每帧调用，`dt` 为帧间隔秒数 |
| `on_complete(track, name)` | 非循环动画完成时调用 |

可用的 Lua 标准库：`math.*`、`string.*`、`table.*`（安全的纯计算子集，不含 io、os、debug、ffi）。

### 轨道模型

Spine 支持多个动画轨道同时播放，实现动作叠加：

```
track 0: Idle_01（循环，一直播）
track 1: Talk_01_M（说话身体，叠加）    ← 同时播放
track 2: Talk_01_A（说话手臂，叠加）    ← 同时播放
```

轨道 0 播放基础动画（如 idle），轨道 1/2 播放叠加动画（如说话），说话结束后淡出 1/2，露出底层的 idle。

### 编译支持

Lua 脚本功能默认启用。如果不使用 Lua 功能，可通过以下方式关闭：

```bash
cargo build --no-default-features
```

这会在编译时减少一个依赖（mlua），生成的二进制不包含 Lua 解释器。

---

## 自动功能说明

### 自动暂停（-p）

工作原理：
1. 启动一个后台线程，每 2 秒检查一次壁纸是否可见
2. 如果壁纸被完全遮挡（无帧回调）且 mpv 未暂停，则发送暂停命令
3. 当壁纸重新可见时，自动恢复播放

适用场景：全屏游戏或应用时节省 CPU/GPU 资源。

### 自动停止（-s）

工作原理：
1. 与自动暂停类似，但检测到遮挡后会完全停止 celestia-wallpaper
2. 主循环退出，释放所有资源（EGL 表面、Wayland 连接、mpv 实例）
3. 进程自然退出

适用场景：长时间不需要壁纸时彻底释放资源。

---

## 图层（Layer）说明

`wlr-layer-shell` 协议定义了 4 个图层，从下到上依次为：

| 图层 | 说明 | 典型用途 |
|------|------|----------|
| `background` | 最底层，在所有窗口之下 | **壁纸（默认）** |
| `bottom` | 在 background 之上，窗口之下 | 桌面小组件 |
| `top` | 在窗口之上 | 面板、状态栏 |
| `overlay` | 最顶层，覆盖所有窗口 | 通知、锁屏 |

对于壁纸用途，通常使用 `background`（默认值）即可。

---

## 架构概览

### 工作区结构

```
mpvpaper-rust/
├── Cargo.toml                                  # 工作区根
├── proto/
│   └── wlr-layer-shell-unstable-v1.xml         # wlr-layer-shell 协议定义
├── celestia-wallpaper/                         # 主程序
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                             # 入口、信号处理、主循环
│       ├── cli.rs                              # clap CLI 参数解析
│       ├── wayland.rs                          # Wayland 连接、输出管理、图层表面
│       ├── egl.rs                              # EGL 显示/上下文/表面管理
│       ├── mpv_ctx.rs                          # mpv 创建、初始化、渲染上下文
│       ├── monitor.rs                          # 自动暂停/停止线程、监控列表
│       ├── renderers/                          # 壁纸渲染器
│       │   ├── mod.rs                          # 模块注册
│       │   ├── image.rs                        # 静态图片渲染
│       │   ├── mpv.rs                          # 视频渲染（mpv）
│       │   ├── spine.rs                        # Spine 2D 骨骼动画渲染
│       │   └── spine_lua.rs                    # Lua 脚本引擎（可选）
│       └── log.rs                              # 彩色终端日志宏
└── celestia-wallpaper-holder/                  # 占位程序
    ├── Cargo.toml
    └── src/
        └── main.rs                             # SHM 虚拟缓冲区、恢复逻辑
```

### 模块说明

#### `main.rs`
程序入口。负责：
- 解析命令行参数
- 初始化 Wayland 连接和 EGL
- 创建 mpv 实例和渲染上下文
- 设置 `eventfd` 用于 mpv 渲染唤醒
- 运行 `poll()` 主循环，处理 Wayland 事件和 mpv 渲染更新

#### `cli.rs`
使用 `clap` derive API 定义所有命令行参数，包含参数验证和图层枚举转换。

#### `wayland.rs`
管理 Wayland 连接和协议交互：
- `WaylandState`：持有 display、compositor、layer_shell 和输出列表
- `DisplayOutput`：每个显示器的状态（尺寸、缩放、EGL 窗口/表面、帧回调）
- 实现 `Dispatch` trait 处理 Wayland 事件

#### `egl.rs`
EGL 动态加载和上下文管理：
- 使用 `khronos-egl` 的 `DynamicInstance<EGL1_5>` 动态加载 `libEGL.so`
- 使用过滤属性（`SURFACE_TYPE | WINDOW_BIT | RGBA8 | OPENGL_BIT`）精确选择 EGL config
- 按 OpenGL → GLES → fallback 顺序尝试创建上下文，避免无过滤遍历
- 管理每个输出的 EGL 表面

#### `mpv_ctx.rs`
libmpv FFI 的安全封装：
- `MpvContext`：mpv 实例的创建、配置、初始化
- `MpvRenderContext`：OpenGL 渲染上下文的创建和渲染
- 命令发送、属性观察、事件等待

#### `monitor.rs`
后台监控线程：
- `spawn_mpv_event_thread`：处理 mpv 事件（关闭、属性变化、幻灯片计时），使用 `wait_event(0.05)` 事件驱动等待，不忙等
- `spawn_auto_pause_thread`：自动暂停/恢复逻辑
- `spawn_auto_stop_thread`：自动停止逻辑，通过 `stop_render_loop` 标志通知主循环退出
- `spawn_pauselist_thread` / `spawn_stoplist_thread`：扫描 `/proc/*/comm` 检测进程（不依赖 `pidof` 子进程）

#### `log.rs`
彩色终端日志宏：
- `[+]` 绿色：成功信息
- `[-]` 红色：错误信息
- `[!]` 黄色：警告信息
- `[*]` 蓝色：普通信息

#### `renderers/` 模块

| 文件 | 说明 |
|------|------|
| `mod.rs` | 模块注册，导出所有渲染器 |
| `image.rs` | 静态图片渲染器：加载 PNG/JPG/BMP/WebP 并以贴图形式渲染 |
| `mpv.rs` | 视频渲染器：通过 libmpv 渲染视频帧到 EGL 表面 |
| `spine.rs` | Spine 2D 骨骼动画渲染器：支持 `.json`/`.skel` 格式 + TOML 配置 |
| `spine_lua.rs` | Lua 脚本引擎：加载 `.spine.lua` 提供可编程的动画状态机（可选） |

Spine 渲染器通过 `rusty_spine` crate 加载骨架和纹理，用自定义 OpenGL shader 渲染。Lua 脚本引擎使用 `mlua`（LuaJIT 后端）提供安全的沙箱环境，仅暴露 `math`、`string`、`table` 标准库和 Spine API。

### 线程模型

```
主线程
  └── poll() 主循环（Wayland 事件 + mpv 渲染）

spawn_mpv_event_thread 线程
  └── 等待 mpv 事件（wait_event 50ms timeout），写入 eventfd 唤醒主线程

spawn_auto_pause_thread 线程（-p 模式）
  └── 检测可见性，暂停/恢复 mpv

spawn_auto_stop_thread 线程（-s 模式）
  └── 检测可见性，设置 stop_render_loop 标志，通知主循环退出

spawn_pauselist_thread 线程（-p 模式）
  └── 扫描 /proc 监控 pauselist 中的进程

spawn_stoplist_thread 线程（-s 模式）
  └── 扫描 /proc 监控 stoplist 中的进程
```

线程间通过 `Arc<HaltInfo>` 共享状态，使用 `AtomicBool` / `AtomicI32` 进行无锁同步。

### 主循环

```
loop {
    1. event_queue.prepare_read()        // 准备读取 Wayland 事件
    2. event_queue.flush()               // 刷新输出缓冲区
    3. poll([wayland_fd, wakeup_fd], timeout=16ms)
    4. 如果 wayland_fd 可读：ReadEventsGuard::read() + dispatch_pending()
    5. 检查 stop_render_loop 标志
    6. 如果 wakeup_fd 可读：
       - mpv_render_context_update()
       - 对每个输出：渲染（有帧回调可用）或设置 redraw_needed（等待帧回调）
}
```

- `wakeup_fd`（eventfd）由 mpv 的 `render_update_callback` 写入，通知主线程新帧就绪
- 帧回调 `Done` 事件处理中如果发现 `redraw_needed`，立即写入 wakeup_fd 触发重新渲染（避免丢失帧）
- `stop_render_loop` 由 `spawn_auto_stop_thread` 或 `spawn_mpv_event_thread`（收到 `MPV_EVENT_SHUTDOWN`）设置

---

## 性能优化

### 已完成的优化

| 优化项 | 说明 | 效果 |
|--------|------|------|
| 移除冗余 FFI 查询 | 事件线程不再每 50ms 额外调用 `get_property_flag("pause")`，pause 状态由事件驱动更新 | 消除每秒 20 次不必要的 FFI + 堆分配 |
| EGL config 过滤 | `choose_config` 使用 `WINDOW_BIT \| RGBA8 \| OPENGL_BIT` 过滤属性 | 避免遍历数百个无关 config |
| 内存顺序修正 | `SeqCst` → `Relaxed`/`Acquire`/`Release` | 消除 x86 `mfence` 指令 |
| `/proc` 替代 `pidof` | 直接扫描 `/proc/*/comm` 而非 spawn 子进程 | 消除每秒的进程创建开销 |
| `eventfd` 生命周期 | `OnceLock<EventFd>` 替代 `mem::forget` | 修复资源泄漏 |
| `Arc<MpvContext>` | 替代 raw pointer + `mem::forget` | 线程安全退出时自动调用 `mpv_terminate_destroy` |
| `wait_event(0.05)` | 替代 `wait_event(0) + sleep(10ms)` | 轮询频率从 100Hz 降至 20Hz |
| auto-pause 忙等间隔 | 内部循环从 10ms 改为 100ms | 隐藏时 CPU 占用降低 90%+ |
| auto-stop 修复 | 设置 `stop_render_loop` 通知主循环退出 | 修复进程空转直至段错误的 bug |
| `format_args!` | 日志宏改用 `format_args!` 避免中间 `String` | 减少非必要堆分配 |
| `swap_remove` | 替换 `Vec::remove` | 输出移除 O(n) → O(1) |

### 性能分析（heaptrack）

使用 [heaptrack](https://github.com/KDE/heaptrack) 对运行中的 celestia-wallpaper 进行实时堆分析：

| 指标 | 值 |
|------|-----|
| 峰值堆内存 | ~395 MB |
| 分配调用总次数 | ~365 万次（7,850/s） |
| 临时分配 | ~118 万次 |
| 泄漏 | ~81 MB（来自 mpv/FFmpeg 内部 Lua/ASS 路径） |

**热点分布：**
- `av_malloc` (FFmpeg 解码器): 84 万次/267 MB 峰值 — **h.264 参考帧池**
- `ta_alloc_size` (libmpv 内部): 119 万次/1.2 MB 峰值 — **mpv 字符串/配置分配**
- Rust 代码: 几乎为 0 — **所有主要分配均在 C 库中**

> 结论：峰值 ~395 MB 中约 99% 由 C 库（FFmpeg、libmpv、NVIDIA 驱动）分配，Rust 封装层占用 <2 MB。性能已与 C 原版对齐。

### 编译优化

默认以 Cargo Release 模式编译时启用 LTO 和优化。如需进一步减小体积：

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

---

## 技术栈

| 组件 | 依赖 | 版本 | 用途 |
|------|------|------|------|
| Wayland 客户端 | `wayland-client` | 0.31 | Wayland 协议交互 |
| Wayland 协议 | `wayland-protocols` | 0.31 | 标准协议（xdg-shell 等） |
| WLR 协议 | `wayland-protocols-wlr` | 0.3 | wlr-layer-shell 协议 |
| EGL | `khronos-egl` | 6 | 动态 EGL 加载 |
| OpenGL | `gl` | 0.14 | OpenGL 函数绑定 |
| mpv | `libmpv-sys` | 3 | libmpv FFI 绑定 |
| CLI | `clap` | 4 | 命令行参数解析 |
| 日志 | `log` + `env_logger` | 0.4 / 0.11 | 日志框架 |
| 序列化 | `serde` + `toml` | 1 / 0.8 | TOML 配置解析 |
| 图像加载 | `image` | 0.25 | Spine 纹理加载 |
| Spine | `rusty_spine` | git | Spine 3.8 骨骼动画运行时 |
| Lua 脚本 | `mlua` | 0.11 | LuaJIT 脚本引擎（可选） |
| 系统调用 | `nix` | 0.29 | eventfd、signal、poll、shm |
| FFI | `libc` | 0.2 | C FFI 类型 |

---

## 常见问题

### Q: 编译时报错找不到 `wayland-client.pc`

A: 需要安装 `wayland-devel` 包（Fedora）或 `libwayland-dev`（Debian/Ubuntu）。

### Q: 运行时显示 `EGL 初始化失败`

A: 确保已安装 `mesa-libEGL-devel`（Fedora）或 `libegl-dev`（Debian/Ubuntu），并且显卡驱动正确安装。

### Q: 视频无法播放，mpv 报错

A: 确保系统已安装 mpv 及其解码器。某些发行版需要额外安装 `ffmpeg` 或 `gstreamer` 插件。

### Q: 自动暂停/停止不工作

A: 检查 `~/.config/celestia-wallpaper/pauselist` 或 `stoplist` 文件是否存在且格式正确（每行一个进程名）。自动功能依赖 Wayland 合成器的帧回调行为，某些合成器可能不完全支持。

### Q: 多显示器下只显示一个

A: 使用 `celestia-wallpaper -d` 查看所有可用输出名称，为每个显示器分别启动一个 celestia-wallpaper 实例。

### Q: 如何从原版 mpvpaper 迁移？

A: 直接将 `mpvpaper` 命令替换为 `celestia-wallpaper`，`mpvpaper-holder` 替换为 `celestia-wallpaper-holder`，并将配置文件从 `~/.config/mpvpaper/` 移到 `~/.config/celestia-wallpaper/` 即可。

---

## 许可证

本项目遵循与原版 mpvpaper 相同的许可证（GPLv3）。
