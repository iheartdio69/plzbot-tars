use crate::time::{day_number_now};
use crate::types::Usage;
use serde::{de::DeserializeOwned, Serialize};
use std::fs;

pub fn load_json<T: DeserializeOwned + Default>(path: &str) -> T {
    let Ok(s) = fs::read_to_string(path) else { return T::default(); };
    serde_json::from_str(&s).unwrap_or_default()
}

pub fn save_json<T: Serialize>(path: &str, value: &T) {
    if let Ok(s) = serde_json::to_string_pretty(value) {
        let _ = fs::write(path, s);
    }
}

pub fn load_usage(path: &str) -> Usage {
    let Ok(s) = fs::read_to_string(path) else {
        return Usage { day: day_number_now(), requests: 0 };
    };
    serde_json::from_str(&s).unwrap_or(Usage { day: day_number_now(), requests: 0 })
}

pub fn save_usage(path: &str, u: &Usage) {
    if let Ok(s) = serde_json::to_string_pretty(u) {
        let _ = fs::write(path, s);
    }
}