pub async fn send_alert(token: &str, chat_id: &str, message: &str) {
    if token.is_empty() || chat_id.is_empty() {
        return;
    }
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
    let client = reqwest::Client::new();
    let _ = client
        .post(&url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": message,
            "parse_mode": "HTML"
        }))
        .send()
        .await;
}
