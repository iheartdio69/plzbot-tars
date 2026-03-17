use std::fs::OpenOptions;
use std::io::Write;

pub fn append(line: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("gambol.log")
    {
        let _ = writeln!(file, "{}", line);
    }
}
