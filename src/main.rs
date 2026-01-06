mod app;
mod config;
mod fmt;
mod helius;
mod market;
mod scoring;
mod time;
mod types;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let cfg = config::load_config();

    println!("🚀 solana_meme starting");
    println!(
        "CFG snapshot={}s window={}s min_call_fdv=${}",
        cfg.snapshot_interval_secs, cfg.window_secs, cfg.min_call_fdv_usd
    );

    app::run(cfg).await;
}