mod app;
mod config;
mod fmt;
mod helius;
mod io;
mod market;
mod onchain;
mod missed_calls;
mod reputation;
mod resolver;
mod rug_tracker;
mod rugcheck;
mod scoring;
mod telegram;
mod time;
mod types;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let cfg = config::load_config();

    println!("🚀 plzbot starting");
    println!(
        "CFG snapshot={}s window={}s min_call_fdv=${}",
        cfg.snapshot_interval_secs, cfg.window_secs, cfg.min_call_fdv_usd
    );

    // Load wallet reputation history before scanning
    reputation::load_reputation();

    app::run(cfg).await;
}
