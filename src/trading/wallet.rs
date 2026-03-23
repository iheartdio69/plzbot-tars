use anyhow::{anyhow, Result};
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_client::rpc_client::RpcClient;

pub struct TradingWallet {
    pub keypair: Keypair,
    pub pubkey: String,
}

impl TradingWallet {
    pub fn load_from_env() -> Result<Self> {
        let key_str = std::env::var("TARS_PRIVATE_KEY")
            .map_err(|_| anyhow!("TARS_PRIVATE_KEY not set"))?;

        // Try base58 decode
        let key_bytes = bs58::decode(&key_str)
            .into_vec()
            .map_err(|e| anyhow!("Invalid private key: {}", e))?;

        let keypair = Keypair::try_from(key_bytes.as_slice())
            .map_err(|e| anyhow!("Invalid keypair bytes: {}", e))?;

        let pubkey = keypair.pubkey().to_string();
        println!("💳 Wallet loaded: {}", &pubkey[..8]);

        Ok(Self { keypair, pubkey })
    }

    pub fn check_balance(&self, rpc_url: &str) -> Result<f64> {
        let client = RpcClient::new(rpc_url.to_string());
        let balance = client
            .get_balance(&self.keypair.pubkey())
            .map_err(|e| anyhow!("Balance check failed: {}", e))?;
        Ok(balance as f64 / 1_000_000_000.0)
    }

    pub fn is_funded(&self, rpc_url: &str, min_sol: f64) -> bool {
        match self.check_balance(rpc_url) {
            Ok(bal) => bal >= min_sol,
            Err(_) => false,
        }
    }
}
