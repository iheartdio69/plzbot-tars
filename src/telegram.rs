use crate::types::CallRecord;

pub async fn send_message(token: &str, chat_id: &str, text: &str) {
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        token
    );
    let params = [
        ("chat_id", chat_id),
        ("text", text),
        ("parse_mode", "HTML"),
    ];
    let _ = reqwest::Client::new()
        .post(&url)
        .form(&params)
        .send()
        .await;
}

pub async fn alert_call(token: &str, chat_id: &str, call: &CallRecord, fdv: f64, liq: f64, vel: f64, bsr: f64, buys: u64) {
    // Message 1: the CA alone so it's easy to copy
    send_message(token, chat_id, &call.mint).await;

    // Message 2: the details
    let detail = format!(
        "🎯 <b>CALL</b>\nFDV: <b>${:.0}</b>\nLIQ: ${:.0}\nVel: {:.1}%/min\nBSR: {:.1}x\nBuys 5m: {}\nScore: {}",
        fdv, liq, vel, bsr, buys, call.score
    );
    send_message(token, chat_id, &detail).await;
}
