pub fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn now() -> u64 {
    now_ts()
}

pub fn day_number_now() -> u64 {
    now_ts() / 86400
}
