# plzbot-tars

Private Solana meme coin scanner and call engine.

## What it does

- Monitors pump.fun via Helius WebSocket for new token launches
- Discovers trending coins via DexScreener
- Scores coins using FDV velocity, buy pressure, wallet reputation, rugcheck safety
- Makes calls when a coin passes all gates
- Sends Telegram alerts with copy button, chart link, snipe link
- Resolves calls as WIN/MID/LOSS based on price movement
- Dynamically learns from outcomes — flags rug wallets, boosts winning wallets

## Scoring signals

| Signal | Source | Weight |
|--------|--------|--------|
| FDV velocity (% growth in 5m) | DexScreener | Primary |
| Buy/sell ratio 5m + 1h | DexScreener | High |
| Liquidity size + growth | DexScreener | Medium |
| Holder count + distribution | Rugcheck | Medium |
| Dev bonded history | Rugcheck | Medium |
| Real SOL buys (0.5+ SOL) | Helius | Boost |
| Whale buys (3+ SOL) | Helius | Boost |
| Wallet reputation | Local DB | Modifier |

## Hard gates (instant skip)

- Zero liquidity
- More sells than buys (BSR < 1.0)
- Mint authority not revoked
- Top holder > 30% of supply
- Insider network detected
- Already rugged

## Setup

Copy `.env.example` to `.env` and fill in your keys.

```bash
cargo build --release
./target/release/solana_meme
```

## Architecture

```
Helius WebSocket → new mints at birth
DexScreener API  → market data every 3s
Rugcheck API     → safety + holder data
Wallet DB        → 260k+ scored wallets

→ Scoring engine → Telegram alerts → POOR TODAY
```

## Environment variables

See `.env.example` for all required variables.
Never commit `.env` — it contains private keys and API credentials.
