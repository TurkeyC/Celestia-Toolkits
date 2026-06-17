#[macro_use]
mod log;
mod cli;
mod egl;
mod monitor;
mod mpv_ctx;
mod renderer;
mod renderers;
mod wayland;

use std::error::Error;
use std::ffi::c_void;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use clap::Parser;

use cli::Args;
use egl::EglState;
use renderer::{WallpaperRenderer, WallpaperType};
use renderers::image::ImageRenderer;
use renderers::mpv::MpvVideoRenderer;
use renderers::spine::SpineRenderer;
use wayland::{HaltInfo, WaylandState};

pub(crate) static WAKEUP_FD: OnceLock<RawFd> = OnceLock::new();
static WAKEUP_EVENTFD: OnceLock<nix::sys::eventfd::EventFd> = OnceLock::new();

fn create_renderer(
    wtype: WallpaperType,
    args: &Args,
    halt_info: Arc<HaltInfo>,
) -> Result<Box<dyn WallpaperRenderer>, Box<dyn Error>> {
    let (_monitor_name, video_path) = cli::validate_args(args);
    let mpv_options_config = args.mpv_options.as_ref().map(|o| cli::mpv_options_to_config(o));
    let slideshow_time = args.slideshow.unwrap_or(0);

    match wtype {
        WallpaperType::Video => Ok(Box::new(MpvVideoRenderer::new(
            video_path,
            slideshow_time,
            mpv_options_config,
            args.save_info.clone(),
            args.verbose,
            halt_info,
        ))),
        WallpaperType::Picture => Ok(Box::new(ImageRenderer::new(
            video_path,
            args.verbose,
        ))),
        WallpaperType::Spine => Ok(Box::new(SpineRenderer::new(
            video_path,
            args.verbose,
        ))),
        WallpaperType::Web => {
            // TODO: Phase 4 - WebRenderer
            log_error!("Web wallpaper not yet implemented");
            std::process::exit(1);
        }
    }
}

fn main() {
    let args = Args::parse();

    monitor::check_paper_processes();

    let auto_pause = args.auto_pause;
    let mut auto_stop = args.auto_stop;
    if auto_pause && auto_stop {
        log_warning!("You cannot use auto-stop and auto-pause together");
        auto_stop = false;
    }

    if args.slideshow == Some(0) {
        log_warning!("0 or invalid time set for slideshow\nPlease use a positive integer");
    }

    let surface_layer = cli::parse_layer(&args);
    let verbose = args.verbose;
    let show_outputs = args.help_output;

    let (monitor_name, _file_path) = if show_outputs {
        (String::new(), String::new())
    } else {
        cli::validate_args(&args)
    };

    let wallpaper_type = args.wallpaper_type().unwrap_or(WallpaperType::Video);
    if verbose > 0 {
        log_info!("Wallpaper type: {}", wallpaper_type.as_str());
    }

    if args.fork {
        match unsafe { nix::unistd::fork() } {
            Ok(nix::unistd::ForkResult::Parent { .. }) => std::process::exit(0),
            Ok(nix::unistd::ForkResult::Child) => {}
            Err(_) => {
                log_error!("Failed to fork");
                std::process::exit(1);
            }
        }
    }

    let halt_info = Arc::new(HaltInfo::default());
    halt_info.auto_pause.store(auto_pause, Ordering::Relaxed);
    halt_info.auto_stop.store(auto_stop, Ordering::Relaxed);

    let eventfd = nix::sys::eventfd::EventFd::from_value_and_flags(
        0,
        nix::sys::eventfd::EfdFlags::EFD_CLOEXEC
            | nix::sys::eventfd::EfdFlags::EFD_NONBLOCK
            | nix::sys::eventfd::EfdFlags::EFD_SEMAPHORE,
    )
    .expect("Creating eventfd failed");
    let wakeup_fd = eventfd.as_raw_fd();
    WAKEUP_FD.set(wakeup_fd).ok();
    WAKEUP_EVENTFD.set(eventfd).ok();

    let conn = wayland_client::Connection::connect_to_env().expect(
        "Unable to connect to the compositor.\n\
         If your compositor is running, check or set the WAYLAND_DISPLAY environment variable.",
    );

    if verbose > 0 {
        log_success!("Connected to Wayland compositor");
    }

    let mut event_queue = conn.new_event_queue::<WaylandState>();
    let qh = event_queue.handle();

    let mut state = WaylandState::new(
        monitor_name.clone(),
        surface_layer,
        show_outputs,
        verbose,
        halt_info.clone(),
    );

    let egl_state: Option<EglState>;
    let mut renderer: Option<Box<dyn WallpaperRenderer>> = None;

    if !show_outputs {
        let wl_display_ptr = conn.backend().display_ptr() as *mut c_void;

        let egl = EglState::init(wl_display_ptr, verbose);
        if verbose > 0 {
            log_success!("EGL initialized");
        }

        // Create the wallpaper renderer and initialize it
        let mut r = create_renderer(wallpaper_type, &args, halt_info.clone())
            .unwrap_or_else(|e| {
                log_error!("Failed to create renderer: {}", e);
                std::process::exit(1);
            });

        if let Err(e) = r.init(&egl, wl_display_ptr) {
            log_error!("Renderer initialization failed: {}", e);
            std::process::exit(1);
        }

        egl_state = Some(egl);
        renderer = Some(r);
    } else {
        egl_state = None;
    }

    let display = conn.display();
    let _registry = display.get_registry(&qh, ());

    event_queue.roundtrip(&mut state).expect("Wayland roundtrip 1 failed");

    if state.compositor.is_none() || state.layer_shell.is_none() {
        log_error!("Missing a required Wayland interface");
        std::process::exit(1);
    }

    event_queue.roundtrip(&mut state).expect("Wayland roundtrip 2 failed");

    if show_outputs {
        std::process::exit(0);
    }

    if state.outputs.is_empty() {
        log_error!("sorry but we can't seem to find any output.");
        std::process::exit(1);
    }

    state.egl_initialized = true;

    event_queue.roundtrip(&mut state).expect("Wayland roundtrip 3 failed");

    // Render initial frames on all outputs
    if let (Some(ref egl), Some(ref renderer)) = (egl_state.as_ref(), renderer.as_ref()) {
        for (output_idx, output) in state.outputs.iter_mut().enumerate() {
            if let Some(egl_win) = output.egl_window {
                if output.egl_surface.is_none() {
                    if let Some(surf) = egl.create_surface_for_egl_window(egl_win) {
                        let raw_surf = unsafe { std::mem::transmute::<khronos_egl::Surface, *mut c_void>(surf) };
                        output.egl_surface = Some(raw_surf);

                        egl.make_current(Some(surf));
                        egl.swap_interval(0);

                        unsafe {
                            gl::DrawBuffer(gl::BACK);
                            gl::ClearColor(0.0, 0.0, 0.0, 0.0);
                        }

                        let w = output.width as i32 * output.scale;
                        let h = output.height as i32 * output.scale;
                        unsafe { gl::Viewport(0, 0, w, h) };

                        // Let the renderer draw the initial frame
                        if let Err(e) = renderer.render(output_idx, egl, w, h) {
                            log_error!("Initial render failed: {}", e);
                        }

                        if let Some(ref surface) = output.surface {
                            let cb = surface.frame(&qh, ());
                            output.frame_callback = Some(cb);
                        }

                        egl.swap_buffers(surf);
                    }
                }
            }
        }
    }

    // Spawn renderer threads (requires &mut, do this before taking &ref for the loop)
    let thread_handles = renderer.as_mut()
        .map(|r| r.spawn_threads(halt_info.clone()))
        .unwrap_or_default();

    let wl_fd: RawFd = conn.as_fd().as_raw_fd();

    loop {
        let read_guard = event_queue.prepare_read();

        if let Err(e) = event_queue.flush() {
            match e {
                wayland_client::backend::WaylandError::Io(ref io_err)
                    if io_err.kind() == std::io::ErrorKind::WouldBlock => {}
                _ => break,
            }
        }

        let mut fds = [
            nix::poll::PollFd::new(unsafe { std::os::fd::BorrowedFd::borrow_raw(wl_fd) }, nix::poll::PollFlags::POLLIN),
            nix::poll::PollFd::new(unsafe { std::os::fd::BorrowedFd::borrow_raw(wakeup_fd) }, nix::poll::PollFlags::POLLIN),
        ];

        let poll_result = nix::poll::poll(&mut fds, nix::poll::PollTimeout::from(16u16));
        match poll_result {
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => {
                drop(read_guard);
                continue;
            }
            Err(_) => break,
        }

        if let Some(rg) = read_guard {
            let wl_readable = fds[0].revents().is_some_and(|r| r.contains(nix::poll::PollFlags::POLLIN));
            if wl_readable && rg.read().is_err() {
                break;
            }
        }

        if event_queue.dispatch_pending(&mut state).is_err() {
            break;
        }

        if halt_info.stop_render_loop.load(Ordering::Relaxed) {
            halt_info.stop_render_loop.store(false, Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_secs(2));
            break;
        }

        // Read the wakeup eventfd
        let mut inc: u64 = 0;
        let ret = unsafe {
            libc::read(wakeup_fd, &mut inc as *mut u64 as *mut c_void, 8)
        };

        if ret > 0 && inc > 0 {
            if let (Some(ref egl), Some(ref renderer)) = (egl_state.as_ref(), renderer.as_ref()) {
                renderer.on_eventfd_wakeup();
                for (output_idx, output) in state.outputs.iter_mut().enumerate() {
                    if output.frame_callback.is_none() {
                        if let (Some(_), Some(egl_surf_raw)) = (output.egl_window, output.egl_surface) {
                            let egl_surf = unsafe { std::mem::transmute::<*mut c_void, khronos_egl::Surface>(egl_surf_raw) };
                            egl.make_current(Some(egl_surf));

                            let w = output.width as i32 * output.scale;
                            let h = output.height as i32 * output.scale;
                            unsafe { gl::Viewport(0, 0, w, h) };

                            if let Err(e) = renderer.render(output_idx, egl, w, h) {
                                log_error!("Render failed: {}", e);
                            }

                            if let Some(ref surface) = output.surface {
                                let cb = surface.frame(&qh, ());
                                output.frame_callback = Some(cb);
                            }

                            egl.swap_buffers(egl_surf);
                        }
                    } else {
                        output.redraw_needed = true;
                    }
                }
            }
        }
    }

    for handle in thread_handles {
        let _ = handle.join();
    }
}
