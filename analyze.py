import json

def load(path):
    with open(path, "r") as f:
        return json.load(f)

def smoothed_winrate(wins, losses):
    # Laplace smoothing: prevents 1/0 from looking like a god wallet
    # (wins+2)/(samples+4)
    samples = wins + losses
    return (wins + 2) / (samples + 4) if samples >= 0 else 0.0

def top_entities(d, n=15, min_samples=8, label="WALLETS"):
    rows = []
    for addr, info in d.items():
        wins = int(info.get("wins", 0))
        losses = int(info.get("losses", 0))
        samples = wins + losses

        # try multiple possible "seen" keys (your file might differ)
        seen = info.get("seen", info.get("seen_count", info.get("txs", info.get("count", 0))))
        try:
            seen = int(seen)
        except Exception:
            seen = 0

        score = info.get("score", 0)
        try:
            score = float(score)
        except Exception:
            score = 0.0

        if samples < min_samples:
            continue

        smooth = smoothed_winrate(wins, losses)
        rows.append((smooth, samples, score, seen, wins, losses, addr))

    # sort: best smoothed first, then more samples, then score
    rows.sort(key=lambda x: (x[0], x[1], x[2]), reverse=True)
    return rows[:n]

def print_table(title, rows):
    print(f"\n=== {title} (min samples applied) ===")
    if not rows:
        print("No entries meet the minimum sample requirement yet.")
        return
    for i, (smooth, samples, score, seen, wins, losses, addr) in enumerate(rows, 1):
        print(f"{i:>2}. {addr} | smooth {smooth:.1%} | W {wins} / L {losses} | samples {samples} | seen {seen} | score {score:g}")

def main():
    wallets = load("wallets.json") if __import__("os").path.exists("wallets.json") else {}
    whales  = load("whales.json") if __import__("os").path.exists("whales.json") else {}

    print(f"Wallets tracked: {len(wallets):,}")
    print(f"Whales tracked:  {len(whales):,}")

    print_table("TOP WALLETS", top_entities(wallets, n=15, min_samples=8, label="WALLETS"))
    print_table("TOP WHALES",  top_entities(whales,  n=15, min_samples=6, label="WHALES"))  # whales often need a lower min early

    # quick sanity: show 3 raw whale entries so we can verify schema
    if whales:
        print("\n=== RAW WHALE SAMPLE (first 3) ===")
        for i, (k, v) in enumerate(whales.items()):
            print(k, "=>", v)
            if i == 2:
                break

if __name__ == "__main__":
    main()
