use anyhow::{anyhow, Result};
use crate::trading::wallet::TradingWallet;

const JUPITER_QUOTE_URL: &str = "https://quote-api.jup.ag/v6/quote";
const JUPITER_SWAP_URL: &str = "https://quote-api.jup.ag/v6/swap";
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

pub struct TradeExecutor {
    pub wallet: TradingWallet,
    pub rpc_url: String,
    pub enabled: bool,
}

impl TradeExecutor {
    pub fn new(wallet: TradingWallet, rpc_url: String) -> Self {
        let enabled = std::env::var("TARS_ENABLED")
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);
        Self {
            wallet,
            rpc_url,
            enabled,
        }
    }

    /// Buy `sol_amount` SOL worth of `token_mint`
    pub async fn buy(&self, token_mint: &str, sol_amount: f64) -> Result<String> {
        if !self.enabled {
            println!("  🔒 TARS_ENABLED=false — paper trade only");
            return Ok("paper".to_string());
        }

        let lamports = (sol_amount * LAMPORTS_PER_SOL as f64) as u64;

        // Get Jupiter quote
        let quote_url = format!(
            "{}?inputMint={}&outputMint={}&amount={}&slippageBps=300",
            JUPITER_QUOTE_URL, SOL_MINT, token_mint, lamports
        );

        let client = reqwest::Client::new();
        let quote: serde_json::Value = client
            .get(&quote_url)
            .send()
            .await?
            .json()
            .await
            .map_err(|e| anyhow!("Jupiter quote failed: {}", e))?;

        // Get swap transaction
        let swap_body = serde_json::json!({
            "quoteResponse": quote,
            "userPublicKey": self.wallet.pubkey,
            "wrapAndUnwrapSol": true,
            "dynamicComputeUnitLimit": true,
            "prioritizationFeeLamports": "auto"
        });

        let swap_resp: serde_json::Value = client
            .post(JUPITER_SWAP_URL)
            .json(&swap_body)
            .send()
            .await?
            .json()
            .await
            .map_err(|e| anyhow!("Jupiter swap failed: {}", e))?;

        let swap_tx = swap_resp["swapTransaction"]
            .as_str()
            .ok_or_else(|| anyhow!("No swapTransaction in response"))?;

        // Sign and send
        let signature = self.sign_and_send(swap_tx).await?;
        println!("  ✅ BUY tx: {}", &signature[..12]);
        Ok(signature)
    }

    /// Sell `pct` percent of `token_amount` tokens of `token_mint`
    pub async fn sell(&self, token_mint: &str, token_amount: f64, pct: f64) -> Result<String> {
        if !self.enabled {
            println!("  🔒 TARS_ENABLED=false — paper sell only");
            return Ok("paper".to_string());
        }

        let amount_to_sell = (token_amount * pct / 100.0) as u64;

        let quote_url = format!(
            "{}?inputMint={}&outputMint={}&amount={}&slippageBps=500",
            JUPITER_QUOTE_URL, token_mint, SOL_MINT, amount_to_sell
        );

        let client = reqwest::Client::new();
        let quote: serde_json::Value = client
            .get(&quote_url)
            .send()
            .await?
            .json()
            .await
            .map_err(|e| anyhow!("Jupiter sell quote failed: {}", e))?;

        let swap_body = serde_json::json!({
            "quoteResponse": quote,
            "userPublicKey": self.wallet.pubkey,
            "wrapAndUnwrapSol": true,
            "dynamicComputeUnitLimit": true,
            "prioritizationFeeLamports": "auto"
        });

        let swap_resp: serde_json::Value = client
            .post(JUPITER_SWAP_URL)
            .json(&swap_body)
            .send()
            .await?
            .json()
            .await?;

        let swap_tx = swap_resp["swapTransaction"]
            .as_str()
            .ok_or_else(|| anyhow!("No swapTransaction in response"))?;

        let signature = self.sign_and_send(swap_tx).await?;
        println!("  ✅ SELL {:.0}% tx: {}", pct, &signature[..12]);
        Ok(signature)
    }

    async fn sign_and_send(&self, encoded_tx: &str) -> Result<String> {
        use solana_sdk::transaction::VersionedTransaction;
        use solana_sdk::signer::Signer;
        use solana_client::rpc_client::RpcClient;
        use solana_client::rpc_config::RpcSendTransactionConfig;
        use solana_sdk::commitment_config::CommitmentConfig;

        let tx_bytes = bs58::decode(encoded_tx)
            .into_vec()
            .map_err(|e| anyhow!("Decode tx failed: {}", e))?;

        let mut tx: VersionedTransaction = bincode::deserialize(&tx_bytes)
            .map_err(|e| anyhow!("Deserialize tx failed: {}", e))?;

        // Sign with our keypair
        let message_bytes = tx.message.serialize();
        let sig = self.wallet.keypair.sign_message(&message_bytes);
        tx.signatures[0] = sig;

        let client = RpcClient::new_with_commitment(
            self.rpc_url.clone(),
            CommitmentConfig::confirmed(),
        );

        let signature = client
            .send_transaction_with_config(
                &tx,
                RpcSendTransactionConfig {
                    skip_preflight: true,
                    ..Default::default()
                },
            )
            .map_err(|e| anyhow!("Send tx failed: {}", e))?;

        Ok(signature.to_string())
    }
}
