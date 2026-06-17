# mpvpaper-rust 重构优化方案

## 概述

Rust 重构版 mpvpaper 相比原版 C 性能退化的根本原因分析及优化方案。
优化按 P0（关键修复）、P1（重要优化）、P2（增量优化）三级优先级排列。

---

## 详细分析

### 问题 1：热路径上冗余的 FFI 属性查询（最大性能杀手）

**文件：** `mpvpaper/src/monitor.rs:81-85`

```rust
// 每次 10ms 循环都多一次 FFI 调用
let is_paused = halt_info.is_paused.load(Ordering::SeqCst);
let mpv_paused = mpv.get_property_flag("pause").unwrap_or(0); // ← 冗余
if is_paused == 0 && mpv_paused != 0 {
    mpv.command_async(&["set", "pause", "no"]);
}
```

**C 版**（`main.c:382-383`）：`mpv_paused` 在 `MPV_EVENT_PROPERTY_CHANGE` 事件处理中已经拿到，事件没来就不查。

```c
// C 版：只从事件里拿，不需要额外查询
if (!halt_info.is_paused && mpv_paused)
    mpv_command_async(mpv, 0, (const char *[]){"set", "pause", "no", NULL});
```

Rust 版每 10ms 额外调一次 `get_property_flag`，内部包含：`CString` 分配 → `mpv_get_property` FFI → 返回值处理。每秒 100 次堆分配，C 版完全没有这个开销。

### 问题 2：每个 mpv 命令都堆分配 CString 数组

**文件：** `mpvpaper/src/mpv_ctx.rs:69-83`

```rust
pub fn command(&self, args: &[&str]) -> i32 {
    let c_args: Vec<CString> = args.iter().map(|a| CString::new(*a).unwrap()).collect();
    let mut ptrs: Vec<*const i8> = c_args.iter().map(|a| a.as_ptr()).collect();
    ptrs.push(ptr::null());
    unsafe { mpv_command(self.handle, ptrs.as_mut_ptr()) }
}
```

每次 `command()`/`command_async()` 调用产生：
- `Vec<CString>` 堆分配（N 个元素）
- `Vec<*const i8>` 堆分配（N+1 个元素）
- 每个元素 `CString::new()` 堆分配

**C 版**：直接传栈上字符串字面量数组，0 分配。
```c
mpv_command_async(mpv, 0, (const char *[]){"set", "pause", "yes", NULL});
```

### 问题 3：EGL config 无过滤条件导致巨量 context 创建尝试

**文件：** `mpvpaper/src/egl.rs:98-99, 174-209`

**Rust 版**：
```rust
// 第一次：无过滤拿所有 config
let mut cfgs = Vec::with_capacity(64);
instance.choose_config(d, &[egl::NONE], &mut cfgs);
// 第二次：再拿一次
let mut configs = Vec::with_capacity(64);
let _ = instance.choose_config(display, &[egl::NONE], &mut configs);
// 遍历所有 config × 11 GL 版本 × 2 API × 4 GLES 版本
for (i, &cfg) in configs.iter().enumerate() {
    for api in &[egl::OPENGL_API, egl::OPENGL_ES_API] {
        for &(major, minor) in versions { // 最多 15 个版本
            instance.create_context(display, cfg, None, &ctx_attribs);
        }
    }
}
```

**C 版**（`main.c:572-587`）：带过滤属性的 `eglChooseConfig`，只匹配 1 个 config，最多 11 次 context 创建。
```c
const EGLint win_attrib[] = {
    EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
    EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
    EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8,
    EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
    EGL_NONE
};
eglChooseConfig(egl_display, win_attrib, &egl_config, 1, &num_config);
```

Rust 版可能尝试几千次 `eglCreateContext`，C 版最多 11 次。

### 问题 4：SeqCst 滥用

**文件：** 全局

所有 `Atomic*` 操作统一 `Ordering::SeqCst`，在 x86 上产生 `mfence`/`lock` 前缀指令。大多数场景不需要这么强的保证：

| 变量 | 使用场景 | 正确语义 |
|------|---------|---------|
| `stop_render_loop` | 一次性终止标志 | `Relaxed` |
| `frame_ready` | 生产者（frame callback）→ 消费者（auto_pause） | `Release`/`Acquire` |
| `is_paused` | 引用计数式计数器 | `Relaxed` |
| `auto_pause`/`auto_stop` | 启动后只读 | `Relaxed` |

### 问题 5：资源泄漏

**文件：** `mpvpaper/src/main.rs:86`, `monitor.rs:65,91,121,173`

```rust
// main.rs:86 — 泄漏 eventfd
std::mem::forget(wakeup_fd_owned);
// monitor.rs:65 — 泄漏 mpv handle，跳过 mpv_terminate_destroy
std::mem::forget(mpv);
std::process::exit(0); // 跳过所有 Drop
```

C 版有 `exit_cleanup()` 函数（`main.c:92-119`）在信号处理中释放所有资源，Rust 版用 `process::exit()` 粗暴退出。

### 问题 6：线程更多，且有忙等循环

**文件：** `mpvpaper/src/monitor.rs:114-116`

```rust
// auto_pause 被触发后，100Hz 忙等
while !halt_info.frame_ready.load(Ordering::SeqCst) {
    thread::sleep(Duration::from_millis(10));
}
```

C 版同样的 10ms 循环（`main.c:322-324`），但 C 版可以用 `pthread_cancel` 终止线程，Rust 版只能让线程自行退出或用全局标志。

### 问题 7：Log 宏每次都堆分配

**文件：** `mpvpaper/src/log.rs`

```rust
// format! 每次都堆分配 String
println!("{}[+] {}{}", GREEN, format!($($arg)*), RESET);
// 即使 verbose == 0 也会执行（调用方在调用前检查了级别，但宏本身仍会分配）
```

### 问题 8：Vec::remove O(n) 开销

**文件：** `mpvpaper/src/wayland.rs:206, 276, 346`

`outputs.remove(idx)` 是 O(n) 内存移动操作。对于 1-4 个输出影响不大，但仍是多余的。

---

## 优化方案

### P0 — 关键修复

#### 0.1 移除冗余 get_property_flag 查询

**目标：** 消除热路径上每秒 100 次不必要的 FFI 调用 + 堆分配

**做法：**

`mpvpaper/src/monitor.rs:39-93` 中：

1. 在事件线程的栈上维护 `mpv_paused` 局部变量，初始为 0
2. 在 `MPV_EVENT_PROPERTY_CHANGE` 事件处理中更新 `mpv_paused`
3. 删除循环末尾的 `get_property_flag("pause")` 调用
4. 改用局部 `mpv_paused` 做判断

```rust
pub fn spawn_mpv_event_thread(...) {
    thread::spawn(move || {
        let mut mpv_paused = 0; // 局部变量，替代每次 FFI 查询
        mpv.observe_property(observe_pause, "pause", MPV_FORMAT_FLAG);

        while !halt_info.stop_render_loop.load(Relaxed) {
            // slideshow 判断...
            if slideshow_time > 0 && last_slideshow.elapsed() >= Duration::from_secs(...) {
                mpv.command_async(&["playlist-next"]);
                last_slideshow = Instant::now();
            }

            let event = mpv.wait_event(0.05);

            if !event.is_null() {
                let event = unsafe { &*event };
                if event.event_id == MPV_EVENT_SHUTDOWN {
                    halt_info.stop_render_loop.store(true, Relaxed);
                    return; // 正常返回，不再 process::exit
                } else if event.event_id == MPV_EVENT_PROPERTY_CHANGE
                    && event.reply_userdata == observe_pause
                {
                    // 从事件数据中读取 pause 状态
                    if let Some(paused) = mpv.get_property_flag("pause") {
                        mpv_paused = paused;
                        // ... 更新 halt_info.is_paused ...
                    }
                }
            }

            // 直接用 mpv_paused，不再调 get_property_flag
            let is_paused = halt_info.is_paused.load(Relaxed);
            if is_paused == 0 && mpv_paused != 0 {
                mpv.command_async(&["set", "pause", "no"]);
            }

            // 不再需要 thread::sleep(10ms)，wait_event(0.05) 自带阻塞
        }

        mpv.unobserve_property(observe_pause);
    })
}
```

**注意事项：** `wait_event(0.05)` 的 50ms timeout 代替了原来的 `wait_event(0) + sleep(10ms)`。slideshow 精度由 10ms 降为 50ms，对于秒级的 slideshow 间隔无影响。

#### 0.2 EGL config 加过滤条件

**目标：** 将 config 候选集从几百个降到个位数

**文件：** `mpvpaper/src/egl.rs:98-99, 174`

```rust
// 改前
instance.choose_config(d, &[egl::NONE], &mut cfgs);

// 改后
let attribs = [
    egl::SURFACE_TYPE, egl::WINDOW_BIT as i32,
    egl::RENDERABLE_TYPE, egl::OPENGL_BIT as i32,
    egl::RED_SIZE, 8,
    egl::GREEN_SIZE, 8,
    egl::BLUE_SIZE, 8,
    egl::ALPHA_SIZE, 8,
    egl::NONE as i32, egl::NONE as i32, // 终止
];
// 注意 khronos-egl 的 choose_config 签名：
// choose_config(&self, display, attribs: &[i32], configs: &mut Vec<Config>)
// 需要将 egl 枚举转为 i32
instance.choose_config(d, &attribs, &mut cfgs);
```

第二次 `choose_config`（`egl.rs:174-175`）同样加过滤。

**简化 context 创建：** 在找到 config 后，只尝试一种 API（先 `egl::OPENGL_API`，不行再 `egl::OPENGL_ES_API`），不再双层遍历所有 API。

#### 0.3 Vec::remove 换 swap_remove

**文件：** `mpvpaper/src/wayland.rs:206, 276, 346`

```rust
// 改前
state.outputs.remove(idx);

// 改后
state.outputs.swap_remove(idx);
```

前提：输出顺序不重要（当前没有依赖顺序的逻辑）。

---

### P1 — 重要优化

#### 1.1 降低轮询频率 + 事件驱动

**文件：** `mpvpaper/src/monitor.rs`

**spawn_mpv_event_thread（行 39-93）：**

将 `wait_event(0.0) + sleep(10ms)` → `wait_event(0.05)`（50ms timeout）：
- 轮询频率从 100Hz 降到 20Hz
- `wait_event` 本身会等待，不需要额外 sleep
- slideshow 精度从 10ms 变为 50ms，对秒级 slideshow 无影响

**spawn_auto_pause_thread（行 114-116）：**

```rust
// 改前
while !halt_info.frame_ready.load(Ordering::SeqCst) {
    thread::sleep(Duration::from_millis(10));
}

// 改后
while !halt_info.frame_ready.load(Relaxed) {
    thread::sleep(Duration::from_millis(100)); // 100ms = 10Hz
    // 实际上 frame_ready 由帧同步（~60Hz）设置，100ms 足够了
}
```

更优方案：使用 `std::sync::Condvar`，在 frame callback 中通知：
```rust
// 思路（需要较大改动）：
// 共享 (Mutex<bool>, Condvar)
// frame callback 中：*lock = true; condvar.notify_one();
// auto_pause 中：condvar.wait(lock) 或 wait_timeout
```

#### 1.2 修复资源泄漏

**文件：** `mpvpaper/src/main.rs:86`, `monitor.rs:65,91,121,173`

**核心原则：** 不要用 `mem::forget` 和 `process::exit` 跳过 Drop。

**wakeup_fd：**

```rust
// 改前
let wakeup_fd = wakeup_fd_owned.as_raw_fd();
std::mem::forget(wakeup_fd_owned); // 泄漏
WAKEUP_FD.set(wakeup_fd).unwrap();

// 改后 — 用 ManuallyDrop 确保生命周期可控
let wakeup_fd = wakeup_fd_owned.as_raw_fd();
let wakeup_fd_owned = std::mem::ManuallyDrop::new(wakeup_fd_owned);
// 或直接把 OwnedFd 存到 OnceLock
static WAKEUP_FD_OWNED: OnceLock<OwnedFd> = OnceLock::new();
WAKEUP_FD_OWNED.set(wakeup_fd_owned).unwrap();
WAKEUP_FD.set(wakeup_fd_owned.as_raw_fd()).unwrap(); // wakeup_fd_owned 已被 move
// 程序退出时 OwnedFd::Drop 自动 close(fd)
```

**mpv handle 泄漏：**

```rust
// 改前：传 usize 给线程，mem::forget 跳过了 Drop
let handle_addr = mpv_handle as usize;
thread::spawn(move || {
    let mpv = MpvContext { handle: handle_addr as *mut ... };
    // ...
    std::mem::forget(mpv); // 跳过 mpv_terminate_destroy
});

// 改后：用 Arc<MpvContext>
let mpv_ctx = Arc::new(mpv);
let mpv_ctx_clone = mpv_ctx.clone();
thread::spawn(move || {
    // 使用 mpv_ctx_clone
    // 线程退出时 Arc Drop 自动触发 MpvContext::Drop
});
// 主线程也持有引用，所有线程退出后自动释放
```

**process::exit 替换：**

```rust
// 改前
if event.event_id == MPV_EVENT_SHUTDOWN {
    halt_info.stop_render_loop.store(true, Ordering::SeqCst);
    std::mem::forget(mpv);
    std::process::exit(0);
}

// 改后
if event.event_id == MPV_EVENT_SHUTDOWN {
    halt_info.stop_render_loop.store(true, Relaxed);
    return; // 让主循环自然退出，走 join + Drop
}
```

主循环退出后调用 `event_queue.dispatch_pending()` 处理剩余事件，然后等待所有线程 join，最后释放所有资源。

#### 1.3 修正内存顺序

**文件：** 全局

**替换规则：**

| 文件 | 行号 | 改前 | 改后 |
|------|------|------|------|
| `main.rs` | 71-72 | `SeqCst` | `Relaxed` |
| `main.rs` | 316 | `SeqCst` | `Relaxed` |
| `main.rs` | 317 | `SeqCst` | `Relaxed` |
| `wayland.rs` | 372 | `SeqCst` | `Release` — store |
| `monitor.rs:53` | 53 | `SeqCst` | `Relaxed` |
| `monitor.rs:64` | 64 | `SeqCst` | `Relaxed` |
| `monitor.rs:71` | 71 | `SeqCst` | `Relaxed` |
| `monitor.rs:72` | 72 | `SeqCst` | `Relaxed` |
| `monitor.rs:75` | 75 | `SeqCst` | `Relaxed` |
| `monitor.rs:81` | 81 | `SeqCst` | `Relaxed` |
| `monitor.rs:83` | 83 | `SeqCst` | `Relaxed` |
| `monitor.rs:104` | 104 | `SeqCst` | `Relaxed` |
| `monitor.rs:107` | 107 | `SeqCst` | `Acquire` — load |
| `monitor.rs:108` | 108 | `SeqCst` | `Relaxed` |
| `monitor.rs:114` | 114 | `SeqCst` | `Relaxed` |
| `monitor.rs:117` | 117 | `SeqCst` | `Relaxed` |
| `monitor.rs:130` | 130 | `SeqCst` | `Relaxed` |
| `monitor.rs:134` | 134 | `SeqCst` | `Relaxed` |
| `monitor.rs:155` | 155 | `SeqCst` | `Relaxed` |
| `monitor.rs:159` | 159 | `SeqCst` | `Relaxed` |
| `monitor.rs:163` | 163 | `SeqCst` | `Relaxed` |
| `monitor.rs:165` | 165 | `SeqCst` | `Relaxed` |

**关键原则：**
- `frame_ready` 的 `store(true)` 用 `Release`，`load()` 用 `Acquire`（生产者-消费者模式）
- 其余计数器和标志：`Relaxed`
- 如果 `is_paused` 被多个线程递增/递减，用 `fetch_add(1, Relaxed)` 即可，不需要 SeqCst

#### 1.4 预缓存常用 CString

**目标：** 消除热路径上每次 mpv 命令的堆分配

**文件：** `mpvpaper/src/mpv_ctx.rs`

做法一（简单）：
```rust
impl MpvContext {
    // 高频命令的缓存版本，避免每次重新分配
    fn command_cached(&self, cache: &'static [&'static str]) -> i32 {
        // 用 OnceLock 做全局缓存
        static mut CACHE: OnceLock<HashMap<u64, (Vec<CString>, Vec<*const i8>)>> = OnceLock::new();
        let key = cache.as_ptr() as u64; // 不完美但可用
        let (c_args, ptrs) = CACHE.get_or_init(|| {
            let c_args: Vec<CString> = cache.iter().map(|a| CString::new(*a).unwrap()).collect();
            let mut ptrs: Vec<*const i8> = c_args.iter().map(|a| a.as_ptr()).collect();
            ptrs.push(ptr::null());
            (c_args, ptrs)
        });
        unsafe { mpv_command(self.handle, ptrs.as_ptr() as *mut *const i8) }
    }
}
```

做法二（更简单、无 unsafe）：为高频命令提供专用方法：
```rust
impl MpvContext {
    // 预缓存，在初始化时一次分配
    pub fn with_cached_commands() -> (Self, CachedCommands) { ... }
}

pub struct CachedCommands {
    set_pause_yes: Vec<*const i8>,
    set_pause_no: Vec<*const i8>,
    playlist_next: Vec<*const i8>,
    loadfile: Vec<*const i8>,
}

impl CachedCommands {
    pub fn new() -> Self { /* 一次分配，之后复用 */ }
}
```

建议用做法二：将 `MpvContext` 与一组预分配的 CString 命令绑定，在 `main.rs` 初始化时创建，传给所有线程。

#### 1.5 修正 auto-pause 内部忙等循环

**文件：** `mpvpaper/src/monitor.rs:114-116`

```rust
// 改前
while !halt_info.frame_ready.load(Ordering::SeqCst) {
    thread::sleep(Duration::from_millis(10));
}

// 改后（简单版）
while !halt_info.frame_ready.load(Acquire) {
    thread::sleep(Duration::from_millis(100));
}

// 改后（高级版 — 用 Condvar）
// 在 HaltInfo 中加入: frame_cond: Condvar, frame_mutex: Mutex<bool>
// frame callback 中: *frame_mutex.lock() = true; frame_cond.notify_one();
// auto_pause 中: frame_cond.wait(frame_mutex.lock()) // 零 CPU 等待
```

---

### P2 — 增量优化

#### 2.1 Log 宏避免堆分配

**文件：** `mpvpaper/src/log.rs`

```rust
// 改前
macro_rules! log_info {
    ($($arg:tt)*) => {
        println!("[*] {}{}", format!($($arg)*), $crate::log::RESET);
    };
}

// 改后
macro_rules! log_info {
    ($($arg:tt)*) => {
        println!("[*] {}{}", format_args!($($arg)*), $crate::log::RESET);
    };
}
```

`format!` 返回 `String`（堆分配），`format_args!` 返回 `fmt::Arguments`（栈上）。`println!` 接收 `Arguments` 直接写入 stdout，避免中间 `String`。

#### 2.2 复用 PollFd 数组

**文件：** `mpvpaper/src/main.rs:288-291`

```rust
// 改前：每次循环新建
loop {
    let mut fds = [
        nix::poll::PollFd::new(unsafe { BorrowedFd::borrow_raw(wl_fd) }, POLLIN),
        nix::poll::PollFd::new(unsafe { BorrowedFd::borrow_raw(wakeup_fd) }, POLLIN),
    ];
    poll(&mut fds, ...);
}

// 改后：移到循环外，每次只重置 events
let mut fds = [
    nix::poll::PollFd::new(unsafe { BorrowedFd::borrow_raw(wl_fd) }, nix::poll::PollFlags::POLLIN),
    nix::poll::PollFd::new(unsafe { BorrowedFd::borrow_raw(wakeup_fd) }, nix::poll::PollFlags::POLLIN),
];
loop {
    // 重置 events（被 poll 修改过 revents 但 events 不变）
    fds[0].set_events(nix::poll::PollFlags::POLLIN);
    fds[1].set_events(nix::poll::PollFlags::POLLIN);
    poll(&mut fds, ...);
}
```

或者完全避开 `nix::poll`，用原始 `libc::poll` + `RawFd` 避免生命周期问题：
```rust
loop {
    let mut pfds = [
        libc::pollfd { fd: wl_fd, events: libc::POLLIN, revents: 0 },
        libc::pollfd { fd: wakeup_fd, events: libc::POLLIN, revents: 0 },
    ];
    let ret = unsafe { libc::poll(pfds.as_mut_ptr(), 2, 16) };
    // ...
}
```

#### 2.3 减少 set_init_options 重复分配

**文件：** `mpvpaper/src/mpv_ctx.rs:109-129`

`set_init_options` 被调两次（`main.rs:122,124`），每次内部都重新创建相同的 `CString`。

**做法：** 将 `set_init_options` 拆为：
1. `set_init_options_pre()` — 设置必须的选项（input-default-bindings, background-color 等）
2. `set_init_options_post()` — 覆盖 config 里的选项
3. 两者共享预分配的 CString

```rust
fn set_option_cached(&self, cache: &mut Option<(CString, CString)>, name: &str, value: &str) {
    let (name_c, value_c) = cache.get_or_insert_with(|| {
        (CString::new(name).unwrap(), CString::new(value).unwrap())
    });
    unsafe { mpv_set_option_string(self.handle, name_c.as_ptr(), value_c.as_ptr()); }
}
```

#### 2.4 `check_pidof` 改用 `/proc` 扫描

**文件：** `mpvpaper/src/monitor.rs:20-28`

```rust
// 改前：每秒 spawn 子进程
fn check_pidof(name: &str) -> bool {
    Command::new("pidof")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// 改后：扫描 /proc，无进程创建开销
fn check_pidof(name: &str) -> bool {
    std::fs::read_dir("/proc").ok().map_or(false, |entries| {
        for entry in entries.flatten() {
            let pid_str = entry.file_name();
            if !pid_str.to_str().map_or(false, |s| s.bytes().all(|b| b.is_ascii_digit())) {
                continue;
            }
            let comm_path = entry.path().join("comm");
            if let Ok(comm) = std::fs::read_to_string(&comm_path) {
                if comm.trim() == name {
                    return true;
                }
            }
        }
        false
    })
}
```

这个改动消除了 pauselist/stoplist 线程每秒的进程 spawn 开销。

#### 2.5 slideshow 计时器优化（可选）

**文件：** `monitor.rs:54`

`Instant::now()` 在 Linux 上走 vDSO，开销极小（~20-50ns），不是性能瓶颈。但如果 slideshow_time 是 0（不使用 slideshow），可以完全跳过 `Instant::now()` 调用：

```rust
if slideshow_time > 0 {
    if last_slideshow.elapsed() >= Duration::from_secs(slideshow_time as u64) {
        mpv.command_async(&["playlist-next"]);
        last_slideshow = Instant::now();
    }
}
```

已经是条件调用，不需要改。

---

## 预期收益

| 序号 | 优化项 | 收益预估 | 难度 | 涉及文件 |
|------|--------|---------|------|---------|
| 0.1 | 移除冗余属性查询 | CPU 降 ~30%（省 100次 FFI + 分配/秒） | 低 | monitor.rs |
| 0.2 | EGL config 过滤 | 启动速度提升 10-100×（某些配置） | 中 | egl.rs |
| 0.3 | swap_remove | 调试性（输出移除 O(n)→O(1)） | 低 | wayland.rs |
| 1.1 | 降低轮询频率 | CPU 降 50-80%（100Hz → 20Hz 或事件驱动） | 低 | monitor.rs |
| 1.2 | 修复泄漏 | 正确性修复，防止长期运行退化 | 中 | main.rs, monitor.rs |
| 1.3 | 修正内存顺序 | x86 降 5-15%，ARM 降更多 | 低 | 全局 |
| 1.4 | CString 缓存 | 热路径分配减少 90%+ | 中 | mpv_ctx.rs |
| 1.5 | 修正 auto-pause 忙等 | 隐藏时 CPU 降 90%+ | 低 | monitor.rs |
| 2.1 | Log 宏改 format_args | 减少非必要堆分配 | 低 | log.rs |
| 2.2 | PollFd 复用 | 减少每帧堆分配 | 低 | main.rs |
| 2.3 | 减少重复分配 | 微量启动时间 | 低 | mpv_ctx.rs |
| 2.4 | /proc 替代 pidof | 消除每秒进程 spawn | 中 | monitor.rs |

## 推荐实施顺序

```
Phase 1 → 0.1, 0.3, 1.1, 1.3 （易实现、收益高，互不依赖）
Phase 2 → 0.2, 1.5, 2.1, 2.2, 2.3, 2.4 （中等难度）
Phase 3 → 1.2, 1.4 （涉及架构调整，放在最后）
```
