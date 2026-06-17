pub const GREEN: &str = "\x1b[0;32m";
pub const RED: &str = "\x1b[1;31m";
pub const YELLOW: &str = "\x1b[0;33m";
pub const RESET: &str = "\x1b[0m";

#[macro_export]
macro_rules! log_success {
    ($($arg:tt)*) => {
        println!("{}[+] {}{}", $crate::log::GREEN, format_args!($($arg)*), $crate::log::RESET);
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        println!("{}[-] {}{}", $crate::log::RED, format_args!($($arg)*), $crate::log::RESET);
    };
}

#[macro_export]
macro_rules! log_warning {
    ($($arg:tt)*) => {
        println!("{}[!] {}{}", $crate::log::YELLOW, format_args!($($arg)*), $crate::log::RESET);
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        println!("[*] {}{}", format_args!($($arg)*), $crate::log::RESET);
    };
}
