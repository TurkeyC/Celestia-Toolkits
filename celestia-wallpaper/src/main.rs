#[macro_use]
mod log;
mod cli;
mod egl;
mod monitor;
mod mpv_ctx;
mod wayland;

use std::ffi::c_void;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use clap::Parser;

use cli::Args;
use egl::EglState;
use mpv_ctx::{MpvContext, MpvRenderContext};
use wayland::{HaltInfo, WaylandState};

pub(crate) static WAKEUP_FD: OnceLock<RawFd> = OnceLock::new();
static WAKEUP_EVENTFD: OnceLock<nix::sys::eventfd::EventFd> = OnceLock::new();

extern "C" fn render_update_callback(_ctx: *mut c_void) {
    if let Some(&fd) = WAKEUP_FD.get() {
        let inc: u64 = 1;
        unsafe {
            libc::write(fd, &inc as *const u64 as *const c_void, 8);
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

    let slideshow_time = args.slideshow.unwrap_or(0);
    let surface_layer = cli::parse_layer(&args);
    let verbose = args.verbose;
    let show_outputs = args.help_output;

    let (monitor_name, video_path) = if show_outputs {
        (String::new(), String::new())
    } else {
        cli::validate_args(&args)
    };

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

    let mpv_options_config = args.mpv_options.as_ref().map(|o| cli::mpv_options_to_config(o));
    let save_info = args.save_info.clone();
    let _argv_copy: Vec<String> = std::env::args().collect();

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
    let mpv_ctx: Option<Arc<MpvContext>>;
    let mpv_render_ctx: Option<MpvRenderContext>;

    if !show_outputs {
        let wl_display_ptr = conn.backend().display_ptr() as *mut c_void;

        let egl = EglState::init(wl_display_ptr, verbose);
        if verbose > 0 {
            log_success!("EGL initialized");
        }

        let mpv = Arc::new(MpvContext::new());
        mpv.set_init_options(slideshow_time, mpv_options_config.as_deref());
        mpv.initialize();
        mpv.set_init_options(slideshow_time, mpv_options_config.as_deref());
        mpv.force_libmpv_vo(verbose);

        let render_ctx = MpvRenderContext::new(&mpv, wl_display_ptr, egl.get_proc_address);

        if let Some(ref info) = save_info {
            let parts: Vec<&str> = info.split_whitespace().collect();
            if parts.len() >= 2 {
                if verbose > 0 {
                    log_info!(
                        "Restoring previous time: {} and playlist position: {}",
                        parts[0],
                        parts[1]
                    );
                }
                let default_start = mpv.get_property_string("start");
                mpv.command(&["set", "start", parts[0]]);
                mpv.command(&["set", "playlist-start", parts[1]]);
                mpv.load_media(&video_path);
                mpv.wait_for_file_loaded(verbose, &video_path);
                if let Some(ref ds) = default_start {
                    mpv.command(&["set", "start", ds]);
                }
            } else {
                mpv.load_media(&video_path);
                mpv.wait_for_file_loaded(verbose, &video_path);
            }
        } else {
            mpv.load_media(&video_path);
            mpv.wait_for_file_loaded(verbose, &video_path);
        }

        mpv.command(&["set", "idle", "no"]);
        render_ctx.set_update_callback(render_update_callback, std::ptr::null_mut());

        if verbose > 0 {
            log_success!("MPV initialized");
        }

        egl_state = Some(egl);
        mpv_ctx = Some(mpv);
        mpv_render_ctx = Some(render_ctx);
    } else {
        egl_state = None;
        mpv_ctx = None;
        mpv_render_ctx = None;
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

    if let Some(ref egl) = egl_state {
        for output in &mut state.outputs {
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

                        if let Some(ref rc) = mpv_render_ctx {
                            let w = output.width as i32 * output.scale;
                            let h = output.height as i32 * output.scale;
                            unsafe { gl::Viewport(0, 0, w, h) };
                            rc.render(w, h);

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
    }

    let mut thread_handles = Vec::new();

    if let Some(ref mpv) = mpv_ctx {
        thread_handles.push(monitor::spawn_mpv_event_thread(
            halt_info.clone(),
            mpv.clone(),
            slideshow_time,
        ));
    }

    if auto_pause {
        if let Some(ref mpv) = mpv_ctx {
            thread_handles.push(monitor::spawn_auto_pause_thread(
                halt_info.clone(),
                mpv.clone(),
            ));
        }
    } else if auto_stop {
        let hi = halt_info.clone();
        thread_handles.push(monitor::spawn_auto_stop_thread(hi));
    }

    if let Some(list) = monitor::load_watch_list("pauselist") {
        if verbose > 0 {
            log_info!("pauselist found and will be monitored");
        }
        if let Some(ref mpv) = mpv_ctx {
            thread_handles.push(monitor::spawn_pauselist_thread(
                halt_info.clone(),
                list,
                mpv.clone(),
            ));
        }
    }

    if let Some(list) = monitor::load_watch_list("stoplist") {
        if verbose > 0 {
            log_info!("stoplist found and will be monitored");
        }
        thread_handles.push(monitor::spawn_stoplist_thread(list));
    }

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
            if wl_readable
                && rg.read().is_err() {
                    break;
                }
            // If not readable, ReadEventsGuard is dropped here which cancels the read
        }

        if event_queue.dispatch_pending(&mut state).is_err() {
            break;
        }

        if halt_info.stop_render_loop.load(Ordering::Relaxed) {
            halt_info.stop_render_loop.store(false, Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_secs(2));
            break;
        }

        let mut inc: u64 = 0;
        let ret = unsafe {
            libc::read(wakeup_fd, &mut inc as *mut u64 as *mut c_void, 8)
        };

        if ret > 0 && inc > 0 {
            if let Some(ref rc) = mpv_render_ctx {
                rc.update();

                if let Some(ref egl) = egl_state {
                    for output in &mut state.outputs {
                        if output.frame_callback.is_none() {
                            if let (Some(_), Some(egl_surf_raw)) = (output.egl_window, output.egl_surface) {
                                let egl_surf = unsafe { std::mem::transmute::<*mut c_void, khronos_egl::Surface>(egl_surf_raw) };
                                egl.make_current(Some(egl_surf));

                                let w = output.width as i32 * output.scale;
                                let h = output.height as i32 * output.scale;
                                unsafe { gl::Viewport(0, 0, w, h) };
                                rc.render(w, h);

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
    }

    for handle in thread_handles {
        let _ = handle.join();
    }
}
