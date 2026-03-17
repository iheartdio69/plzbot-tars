use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::Path;

pub fn append_call_line(line: &str) {
    // put logs in ./data (same place as sqlite)
    let dir = Path::new("./data");
    let _ = create_dir_all(dir);

    let path = dir.join("calls.log");

    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", line);
    }
}

pub fn append_gambol_line(line: &str) {
    // put logs in ./data (same place as sqlite)
    let dir = Path::new("./data");
    let _ = create_dir_all(dir);

    let path = dir.join("gambol.log");

    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", line);
    }
}
