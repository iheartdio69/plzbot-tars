use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HeliusTx {
    pub signature: Option<String>,
    pub timestamp: Option<u64>,
    pub fee_payer: Option<String>,

    #[serde(default)]
    pub token_transfers: Vec<TokenTransfer>,

    #[serde(default)]
    pub native_transfers: Vec<NativeTransfer>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenTransfer {
    pub mint: Option<String>,

    pub user_account: Option<String>,
    pub from_user_account: Option<String>,
    pub to_user_account: Option<String>,

    pub token_amount: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NativeTransfer {
    pub from_user_account: Option<String>,
    pub to_user_account: Option<String>,
    pub amount: u64,
}
