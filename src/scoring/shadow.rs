// scoring/shadow.rs (stub)
type ShadowMap = std::collections::HashMap<String, i32>;

pub fn shadow_should_add(score: i32, cfg: &Config, price_accel: f64, fdv_accel: f64) -> bool {
    score >= cfg.score_target * 8 / 10 || price_accel > 0.0 || fdv_accel > 0.0
}

pub fn shadow_touch(shadow: &mut ShadowMap, mint: &str, cfg: &Config, score: i32) {
    shadow.insert(mint.to_string(), score); // Stub, add logic if needed
}
