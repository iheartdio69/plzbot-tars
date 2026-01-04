import json
import math
from pathlib import Path
from datetime import datetime

WALLETS_PATH = Path("wallets.json")
WHALES_PATH = Path("whales.json")  # optional
OUT_DIR = Path("reports")

MIN_SAMPLES = 8          # ignore tiny sample sizes
MIN_WINRATE_A = 0.62     # 62%+
MIN_SCORE_A = 18         # wallet score threshold
MIN_SCORE_WATCH = 8      # watch threshold

def safe_load(path: Path, default):
    if not path.exists():
        return default
    try:
        return json.loads(path.read_text())
    except Exception:
        return default

def WilsonLowerBound(wins, n, z=1.96):
    # conservative winrate lower bound (better than raw winrate for small n)
    if n == 0:
        return 0.0
    phat = wins / n
    denom = 1 + z*z/n
    center = phat + z*z/(2*n)
    margin = z*math.sqrt((phat*(1-phat) + z*z/(4*n))/n)
    return (center - margin) / denom

def main():
    OUT_DIR.mkdir(exist_ok=True)

    wallets = safe_load(WALLETS_PATH, {})
    whales = safe_load(WHALES_PATH, {})

    rows = []
    for addr, s in wallets.items():
        wins = int(s.get("wins", 0))
        losses = int(s.get("losses", 0))
        score = int(s.get("score", 0))
        seen = int(s.get("seen", 0))
        last_seen = int(s.get("last_seen_ts", 0))
        n = wins + losses
        winrate = (wins / n) if n else 0.0
        wlb = WilsonLowerBound(wins, n)

        # whale metadata if present
        wmeta = whales.get(addr, {})
        blue_txs = int(wmeta.get("blue_txs", 0) or 0)
        beluga_txs = int(wmeta.get("beluga_txs", 0) or 0)
        whale_score = float(wmeta.get("score", 0.0) or 0.0)

        rows.append({
            "wallet": addr,
            "score": score,
            "wins": wins,
            "losses": losses,
            "samples": n,
            "winrate": winrate,
            "wlb": wlb,
            "seen": seen,
            "last_seen_ts": last_seen,
            "last_seen_utc": datetime.utcfromtimestamp(last_seen).isoformat() if last_seen else "",
            "blue_txs": blue_txs,
            "beluga_txs": beluga_txs,
            "whale_score": whale_score,
        })

    # filter + rank
    stable = [r for r in rows if r["samples"] >= MIN_SAMPLES]
    stable.sort(key=lambda r: (r["score"], r["wlb"], r["samples"]), reverse=True)

    # Buckets
    A = [r for r in stable if r["score"] >= MIN_SCORE_A and r["winrate"] >= MIN_WINRATE_A]
    WATCH = [r for r in stable if (r["score"] >= MIN_SCORE_WATCH and r not in A)]
    AVOID = [r for r in stable if r not in A and r not in WATCH]

    def dump_csv(name, items):
        import csv
        p = OUT_DIR / f"{name}.csv"
        if not items:
            p.write_text("")
            return
        with p.open("w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=list(items[0].keys()))
            w.writeheader()
            w.writerows(items)

    dump_csv("wallets_ranked", stable)
    dump_csv("wallets_A_tier", A)
    dump_csv("wallets_watch", WATCH)
    dump_csv("wallets_avoid", AVOID)

    # Print top results
    print("\n=== A-TIER (copy these into a 'greenlist') ===")
    for r in A[:25]:
        print(f"{r['wallet']} | score {r['score']} | W/L {r['wins']}/{r['losses']} | wr {r['winrate']:.1%} | wlb {r['wlb']:.2f}")

    print("\n=== WATCHLIST ===")
    for r in WATCH[:25]:
        print(f"{r['wallet']} | score {r['score']} | W/L {r['wins']}/{r['losses']} | wr {r['winrate']:.1%} | wlb {r['wlb']:.2f}")

    print("\nWrote CSVs to:", OUT_DIR.resolve())

if __name__ == "__main__":
    main()