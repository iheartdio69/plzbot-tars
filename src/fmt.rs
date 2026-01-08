pub const c_reset: &str = "\x1b[0m";
pub const c_dim: &str = "\x1b[2m";

pub const c_pink: &str = "\x1b[38;5;205m";
pub const c_green: &str = "\x1b[38;5;82m";
pub const c_yellow: &str = "\x1b[38;5;226m";
pub const c_cyan: &str = "\x1b[38;5;51m";
pub const c_red: &str = "\x1b[38;5;196m";

pub fn pink(s: &str) -> String {
    format!("{c_pink}{s}{c_reset}")
}
pub fn green(s: &str) -> String {
    format!("{c_green}{s}{c_reset}")
}
pub fn yellow(s: &str) -> String {
    format!("{c_yellow}{s}{c_reset}")
}
pub fn cyan(s: &str) -> String {
    format!("{c_cyan}{s}{c_reset}")
}
pub fn red(s: &str) -> String {
    format!("{c_red}{s}{c_reset}")
}
pub fn dim(s: &str) -> String {
    format!("{c_dim}{s}{c_reset}")
}
