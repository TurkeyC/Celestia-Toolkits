use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static RUNNING: AtomicBool = AtomicBool::new(true);
static CTRL_C_HANDLER: OnceLock<Result<(), String>> = OnceLock::new();

pub fn install_ctrlc_handler() -> Result<&'static AtomicBool, Box<dyn std::error::Error>> {
    RUNNING.store(true, Ordering::SeqCst);

    let result = CTRL_C_HANDLER.get_or_init(|| {
        ctrlc::set_handler(|| {
            RUNNING.store(false, Ordering::SeqCst);
        })
        .map_err(|err| err.to_string())
    });

    match result {
        Ok(()) => Ok(&RUNNING),
        Err(err) => Err(err.clone().into()),
    }
}
