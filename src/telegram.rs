use crate::types::CallRecord;
use serde_json::json;

pub async fn send_message(token: &str, chat_id: &str, text: &str) {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
    let _ = reqwest::Client::new()
        .post(&url)
        .form(&[
            ("chat_id", chat_id),
            ("text", text),
            ("parse_mode", "HTML"),
        ])
        .send()
        .await;
}

pub async fn send_call_alert(token: &str, chat_id: &str, call: &CallRecord, coin_type: &str, fdv: f64, liq: f64, vel: f64, bsr: f64, price_change_1h: f64, holders: u64, score: i32) {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

    let text = format!(
        "🎯 <b>{}</b> | score {}\n\nFDV: <b>${}</b> | LIQ: ${}\nVel: {:.1}%/min | 1h: {:+.0}%\nBSR: {:.1}x | Holders: {}\n\n<code>{}</code>",
        coin_type,
        score,
        fmt_usd(fdv),
        fmt_usd(liq),
        vel,
        price_change_1h,
        bsr,
        holders,
        call.mint,
    );

    // Inline keyboard with copy button + dexscreener link
    let keyboard = json!({
        "inline_keyboard": [[
            {
                "text": "📋 Copy CA",
                "copy_text": { "text": call.mint }
            },
            {
                "text": "📊 Chart",
                "url": format!("https://dexscreener.com/solana/{}", call.mint)
            },
            {
                "text": "🔫 Snipe",
                "url": format!("https://pump.fun/{}", call.mint)
            }
        ]]
    });

    let _ = reqwest::Client::new()
        .post(&url)
        .json(&json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML",
            "reply_markup": keyboard
        }))
        .send()
        .await;
}

pub async fn send_resolution(token: &str, chat_id: &str, mint: &str, outcome: &str, mult: f64, reason: &str) {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

    let icon = match outcome {
        "WIN" => "✅",
        "MID" => "➖",
        _ => "❌",
    };

    let text = format!(
        "{} <b>{}</b> {:.2}x ({})\n<code>{}</code>",
        icon, outcome, mult, reason, mint
    );

    let keyboard = json!({
        "inline_keyboard": [[
            {
                "text": "📋 Copy CA",
                "copy_text": { "text": mint }
            },
            {
                "text": "📊 Chart",
                "url": format!("https://dexscreener.com/solana/{}", mint)
            }
        ]]
    });

    let _ = reqwest::Client::new()
        .post(&url)
        .json(&json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML",
            "reply_markup": keyboard
        }))
        .send()
        .await;
}

fn fmt_usd(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.0}k", n / 1_000.0)
    } else {
        format!("{:.0}", n)
    }
}
