use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::wayland::HaltInfo;
use crate::mpv_ctx::MpvContext;

pub fn load_watch_list(name: &str) -> Option<Vec<String>> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".config/celestia-wallpaper").join(name);
    let content = fs::read_to_string(&path).ok()?;
    let list: Vec<String> = content.split_whitespace().map(|s| s.to_string()).collect();
    if list.is_empty() { None } else { Some(list) }
}

fn check_pidof(name: &str) -> bool {
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let pid_str = entry.file_name();
            if !pid_str.to_str().map_or(false, |s| s.bytes().all(|b| b.is_ascii_digit())) {
                continue;
            }
            match std::fs::read_to_string(entry.path().join("comm")) {
                Ok(comm) if comm.trim() == name => return true,
                _ => {}
            }
        }
    }
    false
}

fn check_watch_list(list: &[String]) -> Option<String> {
    for app in list {
        if check_pidof(app) {
            return Some(app.clone());
        }
    }
    None
}

pub fn spawn_mpv_event_thread(
    halt_info: Arc<HaltInfo>,
    mpv: Arc<MpvContext>,
    slideshow_time: u32,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let observe_pause: u64 = 1;
        mpv.observe_property(observe_pause, "pause", libmpv_sys::mpv_format_MPV_FORMAT_FLAG);

        let mut last_slideshow = Instant::now();
        let mut mpv_paused = 0;

        while !halt_info.stop_render_loop.load(Ordering::Relaxed) {
            if slideshow_time > 0 && last_slideshow.elapsed() >= Duration::from_secs(slideshow_time as u64) {
                mpv.command_async(&["playlist-next"]);
                last_slideshow = Instant::now();
            }

            let event = mpv.wait_event(0.05);
            if !event.is_null() {
                let event = unsafe { &*event };

                if event.event_id == libmpv_sys::mpv_event_id_MPV_EVENT_SHUTDOWN {
                    halt_info.stop_render_loop.store(true, Ordering::Relaxed);
                    return;
                } else if event.event_id == libmpv_sys::mpv_event_id_MPV_EVENT_PROPERTY_CHANGE
                    && event.reply_userdata == observe_pause
                {
                    if let Some(paused) = mpv.get_property_flag("pause") {
                        mpv_paused = paused;
                        if mpv_paused != 0 {
                            if halt_info.is_paused.load(Ordering::Relaxed) == 0 {
                                halt_info.is_paused.fetch_add(1, Ordering::Relaxed);
                            }
                        } else {
                            halt_info.is_paused.store(0, Ordering::Relaxed);
                        }
                    }
                }
            }

            let is_paused = halt_info.is_paused.load(Ordering::Relaxed);
            if is_paused == 0 && mpv_paused != 0 {
                mpv.command_async(&["set", "pause", "no"]);
            }
        }

        mpv.unobserve_property(observe_pause);
    })
}

pub fn spawn_auto_pause_thread(
    halt_info: Arc<HaltInfo>,
    mpv: Arc<MpvContext>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while halt_info.auto_pause.load(Ordering::Relaxed) {
            halt_info.frame_ready.store(false, Ordering::Relaxed);
            thread::sleep(Duration::from_secs(2));

            if !halt_info.frame_ready.load(Ordering::Acquire)
                && halt_info.is_paused.load(Ordering::Relaxed) == 0
            {
                log_info!("Pausing because celestia-wallpaper is hidden");
                mpv.command_async(&["set", "pause", "yes"]);
                halt_info.is_paused.fetch_add(1, Ordering::Relaxed);

                while !halt_info.frame_ready.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(100));
                }
                halt_info.is_paused.fetch_sub(1, Ordering::Relaxed);
            }
        }
    })
}

pub fn spawn_auto_stop_thread(
    halt_info: Arc<HaltInfo>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while halt_info.auto_stop.load(Ordering::Relaxed) {
            halt_info.frame_ready.store(false, Ordering::Relaxed);
            thread::sleep(Duration::from_secs(2));

            if !halt_info.frame_ready.load(Ordering::Acquire) {
                log_info!("Stopping because celestia-wallpaper is hidden");
                halt_info.stop_render_loop.store(true, Ordering::Relaxed);
                return;
            }
        }
    })
}

pub fn spawn_pauselist_thread(
    halt_info: Arc<HaltInfo>,
    pauselist: Vec<String>,
    mpv: Arc<MpvContext>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut list_paused = false;

        loop {
            if let Some(app) = check_watch_list(&pauselist) {
                if !list_paused && halt_info.is_paused.load(Ordering::Relaxed) == 0 {
                    log_info!("Pausing for {}", app);
                    mpv.command_async(&["set", "pause", "yes"]);
                    list_paused = true;
                    halt_info.is_paused.fetch_add(1, Ordering::Relaxed);
                }
            } else if list_paused {
                list_paused = false;
                let current = halt_info.is_paused.load(Ordering::Relaxed);
                if current > 0 {
                    halt_info.is_paused.fetch_sub(1, Ordering::Relaxed);
                }
            }

            thread::sleep(Duration::from_secs(1));
        }
    })
}

pub fn spawn_stoplist_thread(
    stoplist: Vec<String>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            if let Some(app) = check_watch_list(&stoplist) {
                log_info!("Stopping for {}", app);
                return;
            }
            thread::sleep(Duration::from_secs(1));
        }
    })
}

pub fn check_paper_processes() {
    let others = ["swaybg", "glpaper", "hyprpaper", "wpaperd", "swww-daemon"];
    for name in &others {
        if check_pidof(name) {
            log_warning!("{} is running. This may block celestia-wallpaper from being seen.", name);
        }
    }
}
