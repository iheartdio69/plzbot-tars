// common helpers

pub fn sanitize_mint(mut s: String) -> String {
    s = s.trim().trim_matches('"').trim_matches('\'').to_string();

    // strip trailing punctuation
    while matches!(s.chars().last(), Some(c) if c == ',' || c == ')' || c == ']' || c == '}') {
        s.pop();
        s = s.trim().to_string();
    }

    // If the source appends "pump"/"bonk", strip it (these are NOT part of a real mint)
    if let Some(stripped) = s.strip_suffix("pump") {
        s = stripped.to_string();
    }
    if let Some(stripped) = s.strip_suffix("bonk") {
        s = stripped.to_string();
    }

    s
}

pub fn is_probably_pubkey(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 32 || s.len() > 50 {
        return false;
    }
    bs58::decode(s)
        .into_vec()
        .map(|b| b.len() == 32)
        .unwrap_or(false)
}
