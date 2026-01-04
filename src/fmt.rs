pub fn fmt_i64_commas(n: i64) -> String {
    let mut s = n.abs().to_string();
    let mut out = String::new();
    while s.len() > 3 {
        let chunk = s.split_off(s.len() - 3);
        out = if out.is_empty() { chunk } else { format!("{},{}", chunk, out) };
    }
    out = if out.is_empty() { s } else { format!("{},{}", s, out) };
    if n < 0 { format!("-{}", out) } else { out }
}

pub fn fmt_f64_0_commas(x: f64) -> String {
    fmt_i64_commas(x.round() as i64)
}