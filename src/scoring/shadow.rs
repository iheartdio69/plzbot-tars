use std::collections::HashMap;

pub type ShadowMap = HashMap<String, u64>; // mint -> until_ts

pub fn is_shadowed(shadow: &ShadowMap, mint: &str, now: u64) -> bool {
    shadow.get(mint).copied().unwrap_or(0) > now
}

pub fn shadow_for(shadow: &mut ShadowMap, mint: &str, now: u64, secs: u64) {
    shadow.insert(mint.to_string(), now + secs);
}
