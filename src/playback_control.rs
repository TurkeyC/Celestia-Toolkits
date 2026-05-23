use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static RUNNING: AtomicBool = AtomicBool::new(true);
static HARD_EXIT_ON_CTRL_C: AtomicBool = AtomicBool::new(false);
static CTRL_C_HANDLER: OnceLock<Result<(), String>> = OnceLock::new();

pub struct HardExitOnCtrlC {
    previous: bool,
}

impl Drop for HardExitOnCtrlC {
    fn drop(&mut self) {
        HARD_EXIT_ON_CTRL_C.store(self.previous, Ordering::SeqCst);
    }
}

pub fn install_ctrlc_handler() -> Result<&'static AtomicBool, Box<dyn std::error::Error>> {
    RUNNING.store(true, Ordering::SeqCst);

    let result = CTRL_C_HANDLER.get_or_init(|| {
        ctrlc::set_handler(|| {
            if HARD_EXIT_ON_CTRL_C.load(Ordering::SeqCst) {
                std::process::exit(130);
            }
            if !RUNNING.swap(false, Ordering::SeqCst) {
                std::process::exit(130);
            }
        })
        .map_err(|err| err.to_string())
    });

    match result {
        Ok(()) => Ok(&RUNNING),
        Err(err) => Err(err.clone().into()),
    }
}

pub fn hard_exit_on_ctrlc() -> HardExitOnCtrlC {
    let previous = HARD_EXIT_ON_CTRL_C.swap(true, Ordering::SeqCst);
    HardExitOnCtrlC { previous }
}
