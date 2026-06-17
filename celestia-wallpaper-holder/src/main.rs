use std::ffi::CString;
use std::fs;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;

use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_output, wl_region, wl_registry, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle};

use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

struct HolderOutput {
    wl_name: u32,
    wl_output: wl_output::WlOutput,
    name: Option<String>,
    identifier: Option<String>,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<ZwlrLayerSurfaceV1>,
    width: u32,
    height: u32,
}

struct HolderState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<ZwlrLayerShellV1>,
    outputs: Vec<HolderOutput>,
    monitor: String,
    argv_copy: Vec<String>,
    stoplist: Vec<String>,
    auto_stop: bool,
    start_time: u32,
}

impl HolderState {
    fn output_matches(&self, name: &Option<String>, identifier: &Option<String>) -> bool {
        let m = &self.monitor;
        if m == "*" || m == "ALL" || m == "All" || m == "all" {
            return true;
        }
        if let Some(n) = name {
            if m.contains(n.as_str()) {
                return true;
            }
        }
        if let Some(id) = identifier {
            if !id.is_empty() && m.contains(id.as_str()) {
                return true;
            }
        }
        false
    }
}

fn revive_celestia_wallpaper(argv: &[String]) {
    let exe_path = std::fs::read_link("/proc/self/exe").unwrap_or_default();
    let exe_dir = exe_path.parent().unwrap_or(exe_path.as_ref());
    let celestia_wallpaper_path = exe_dir.join("celestia-wallpaper");

    let c_path = CString::new(celestia_wallpaper_path.as_os_str().as_bytes()).unwrap();
    let c_argv: Vec<CString> = std::iter::once(c_path.clone())
        .chain(argv.iter().map(|a| CString::new(a.as_bytes()).unwrap()))
        .collect();
    let c_ptrs: Vec<*const i8> = c_argv
        .iter()
        .map(|a| a.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    unsafe {
        libc::execv(c_path.as_ptr(), c_ptrs.as_ptr());
    }
}

fn check_pidof(name: &str) -> bool {
    Command::new("pidof")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn check_stoplist(stoplist: &[String]) {
    for app in stoplist {
        while check_pidof(app) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

fn create_dummy_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<HolderState>,
) -> wl_buffer::WlBuffer {
    let width: i32 = 1;
    let height: i32 = 1;
    let stride = width * 4;
    let size = stride * height;

    let shm_name = std::ffi::CString::new("/wl_shm-dummy").unwrap();
    unsafe {
        libc::shm_unlink(shm_name.as_ptr());
    }
    let fd = unsafe {
        libc::shm_open(
            shm_name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )
    };
    assert!(fd >= 0, "shm_open failed");
    unsafe {
        libc::shm_unlink(shm_name.as_ptr());
    }

    let owned_fd = unsafe { OwnedFd::from_raw_fd(fd) };

    nix::unistd::ftruncate(&owned_fd, size as i64).expect("ftruncate failed");

    let pool = shm.create_pool(owned_fd.as_fd(), size, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Xrgb8888, qh, ());
    pool.destroy();

    buffer
}

impl Dispatch<wl_registry::WlRegistry, ()> for HolderState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, .. } => match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 4, qh, ()),
                    );
                }
                "wl_shm" => {
                    state.shm =
                        Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                }
                "wl_output" => {
                    let wl_output = registry.bind::<wl_output::WlOutput, _, _>(name, 4, qh, ());
                    state.outputs.push(HolderOutput {
                        wl_name: name,
                        wl_output,
                        name: None,
                        identifier: None,
                        surface: None,
                        layer_surface: None,
                        width: 0,
                        height: 0,
                    });
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(
                        registry.bind::<ZwlrLayerShellV1, _, _>(name, 1, qh, ()),
                    );
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                state.outputs.retain(|o| o.wl_name != name);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for HolderState {
    fn event(
        state: &mut Self,
        wl_output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let idx = match state.outputs.iter().position(|o| o.wl_output == *wl_output) {
            Some(i) => i,
            None => return,
        };

        match event {
            wl_output::Event::Name { name } => {
                state.outputs[idx].name = Some(name);
            }
            wl_output::Event::Description { description } => {
                let identifier = if let Some(paren_pos) = description.rfind('(') {
                    let trimmed = &description[..paren_pos];
                    if !trimmed.is_empty() {
                        trimmed[..trimmed.len() - 1].to_string()
                    } else {
                        String::new()
                    }
                } else {
                    description
                };
                state.outputs[idx].identifier = Some(identifier);
            }
            wl_output::Event::Done => {
                let output = &state.outputs[idx];
                let matches = state.output_matches(&output.name, &output.identifier);
                let has_surface = output.layer_surface.is_some();

                if matches && !has_surface {
                    let compositor = state.compositor.as_ref().unwrap();
                    let layer_shell = state.layer_shell.as_ref().unwrap();
                    let output_ref = &mut state.outputs[idx];

                    let surface = compositor.create_surface(qh, ());
                    let input_region = compositor.create_region(qh, ());
                    surface.set_input_region(Some(&input_region));
                    input_region.destroy();

                    let layer_surface = layer_shell.get_layer_surface(
                        &surface,
                        Some(&output_ref.wl_output),
                        Layer::Background,
                        "celestia-wallpaper".to_string(),
                        qh,
                        (),
                    );

                    layer_surface.set_size(0, 0);
                    layer_surface.set_anchor(
                        zwlr_layer_surface_v1::Anchor::Top
                            | zwlr_layer_surface_v1::Anchor::Right
                            | zwlr_layer_surface_v1::Anchor::Bottom
                            | zwlr_layer_surface_v1::Anchor::Left,
                    );
                    layer_surface.set_exclusive_zone(-1);

                    output_ref.surface = Some(surface);
                    output_ref.layer_surface = Some(layer_surface);

                    if let Some(ref s) = output_ref.surface {
                        s.commit();
                    }
                }

                if !matches {
                    state.outputs.remove(idx);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for HolderState {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let idx = match state
            .outputs
            .iter()
            .position(|o| o.layer_surface.as_ref() == Some(layer_surface))
        {
            Some(i) => i,
            None => return,
        };

        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                state.outputs[idx].width = width;
                state.outputs[idx].height = height;
                layer_surface.ack_configure(serial);

                if !state.stoplist.is_empty() {
                    check_stoplist(&state.stoplist);
                }
                if state.auto_stop {
                    // Create dummy buffer + frame callback
                    let shm = state.shm.as_ref().unwrap();
                    let surface = state.outputs[idx].surface.as_ref().unwrap();
                    let buffer = create_dummy_buffer(shm, qh);

                    let _callback = surface.frame(qh, ());
                    surface.attach(Some(&buffer), 0, 0);
                    surface.damage(0, 0, state.outputs[idx].width as i32, state.outputs[idx].height as i32);
                    surface.commit();
                    buffer.destroy();
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.outputs.remove(idx);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for HolderState {
    fn event(
        state: &mut Self,
        _callback: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { callback_data: frame_time } = event {

            if !state.stoplist.is_empty() {
                check_stoplist(&state.stoplist);
                if frame_time.wrapping_sub(state.start_time) < 1000 {
                    revive_celestia_wallpaper(&state.argv_copy);
                }
            } else {
                revive_celestia_wallpaper(&state.argv_copy);
            }

            state.start_time = frame_time;

            // Find output with matching callback and create new frame
            for output in &state.outputs {
                if let Some(ref surface) = output.surface {
                    let shm = state.shm.as_ref().unwrap();
                    let buffer = create_dummy_buffer(shm, qh);
                    let _callback = surface.frame(qh, ());
                    surface.attach(Some(&buffer), 0, 0);
                    surface.damage(0, 0, output.width as i32, output.height as i32);
                    surface.commit();
                    buffer.destroy();
                }
            }
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for HolderState {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for HolderState {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for HolderState {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for HolderState {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for HolderState {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for HolderState {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_region::WlRegion, ()> for HolderState {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: wl_region::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn load_stoplist() -> Vec<String> {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let path = PathBuf::from(home).join(".config/celestia-wallpaper/stoplist");
    match fs::read_to_string(&path) {
        Ok(content) => content.split_whitespace().map(|s| s.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

fn parse_args() -> (String, bool, Vec<String>) {
    let args: Vec<String> = std::env::args().collect();
    let mut monitor = String::new();
    let mut auto_stop = false;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!(
                    "Usage: celestia-wallpaper-holder <celestia-wallpaper options>\n\
                     Description:\n\
                     celestia-wallpaper-holder acts as a lean gate keeper before celestia-wallpaper can run\n\n\
                     It's sole purpose is to check if there is:\n\
                     Any program that is running from the stoplist file\n\
                     - Set in \"~/.config/celestia-wallpaper/stoplist\"\n\
                     And if the wallpaper needs to be seen when drawn\n\
                     - Set with \"-s\" or \"--auto-stop\" celestia-wallpaper option"
                );
                std::process::exit(0);
            }
            "-s" | "--auto-stop" => {
                auto_stop = true;
            }
            "-Z" => {
                i += 1; // skip save info value
            }
            _ => {
                if !args[i].starts_with('-') && monitor.is_empty() {
                    monitor = args[i].clone();
                }
            }
        }
        i += 1;
    }

    if monitor.is_empty() {
        eprintln!("celestia-wallpaper-holder: not enough args");
        std::process::exit(1);
    }

    (monitor, auto_stop, args)
}

fn main() {
    let (monitor, auto_stop, argv_copy) = parse_args();
    let stoplist = load_stoplist();

    let conn = Connection::connect_to_env().expect("Failed to connect to Wayland compositor");

    let mut event_queue = conn.new_event_queue::<HolderState>();
    let qh = event_queue.handle();

    let mut state = HolderState {
        compositor: None,
        shm: None,
        layer_shell: None,
        outputs: Vec::new(),
        monitor,
        argv_copy,
        stoplist,
        auto_stop,
        start_time: 0,
    };

    let display = conn.display();
    let _registry = display.get_registry(&qh, ());

    event_queue.roundtrip(&mut state).expect("Roundtrip failed");

    if state.compositor.is_none() || state.layer_shell.is_none() {
        eprintln!("Missing required Wayland interfaces");
        std::process::exit(1);
    }

    event_queue.roundtrip(&mut state).expect("Roundtrip failed");

    if state.outputs.is_empty() {
        eprintln!("No outputs found");
        std::process::exit(1);
    }

    // Main dispatch loop
    loop {
        if event_queue.blocking_dispatch(&mut state).is_err() {
            break;
        }
    }
}
