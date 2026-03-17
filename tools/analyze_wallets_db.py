import csv, sqlite3, time, json
from pathlib import Path
from datetime import datetime

DB = Path("./data/solana_meme.sqlite")
OUT_DIR = Path("./reports")
WHALES_JSON = Path("./whales.json")  # optional

TOP_N = 5000          # export top N by score
WATCH_MIN = 8         # score >= watch
AVOID_MAX = -50       # score <= avoid

def utc(ts: int) -> str:
    return datetime.utcfromtimestamp(ts).isoformat() if ts else ""

def load_whales():
    if not WHALES_JSON.exists():
        return {}
    try:
        return json.loads(WHALES_JSON.read_text())
    except Exception:
        return {}

def main():
    OUT_DIR.mkdir(exist_ok=True)
    whales = load_whales()

    con = sqlite3.connect(str(DB))
    cur = con.cursor()

    rows = cur.execute("""
      SELECT wallet, score, COALESCE(last_seen_ts,0) AS last_seen_ts, COALESCE(notes,'') AS notes
      FROM wallets
    """).fetchall()

    out = []
    for wallet, score, last_seen_ts, notes in rows:
        wmeta = whales.get(wallet, {}) if isinstance(whales, dict) else {}
        out.append({
            "wallet": wallet,
            "score": int(score),
            "last_seen_ts": int(last_seen_ts),
            "last_seen_utc": utc(int(last_seen_ts)),
            "is_whale": 1 if wallet in whales else 0,
            "blue_txs": int(wmeta.get("blue_txs", 0) or 0) if isinstance(wmeta, dict) else 0,
            "beluga_txs": int(wmeta.get("beluga_txs", 0) or 0) if isinstance(wmeta, dict) else 0,
            "whale_score": float(wmeta.get("score", 0.0) or 0.0) if isinstance(wmeta, dict) else 0.0,
            "notes": notes,
        })

    out.sort(key=lambda r: r["score"], reverse=True)

    def dump(name, items):
        p = OUT_DIR / name
        with p.open("w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=list(items[0].keys()) if items else ["wallet"])
            w.writeheader()
            if items:
                w.writerows(items)
        print("wrote", p)

    dump("wallets_ranked_db.csv", out[:TOP_N])
    dump("wallets_watch_db.csv", [r for r in out if r["score"] >= WATCH_MIN][:TOP_N])
    dump("wallets_avoid_db.csv", [r for r in out if r["score"] <= AVOID_MAX][:TOP_N])
    dump("wallets_top_neg_db.csv", sorted(out, key=lambda r: r["score"])[:2000])

    # quick console summary
    total = len(out)
    n0 = sum(1 for r in out if r["score"] == 0)
    pos = sum(1 for r in out if r["score"] > 0)
    neg = sum(1 for r in out if r["score"] < 0)
    print(f"wallets total={total:,} score0={n0:,} pos={pos:,} neg={neg:,}")

if __name__ == "__main__":
    main()