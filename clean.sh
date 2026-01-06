#!/bin/bash
set -euo pipefail

echo "🧹 Starting aggressive project cleanup..."

# Remove old json backups
echo "Removing old JSON backups..."
rm -f calls.old.json usage.old.json wallets.old.json whales.old.json

# Remove old Rust files
echo "Removing old Rust source backups..."
rm -f src/helius_OLD.rs src/market_OLD.rs src/scoring_OLD.rs

# Remove full backup folder
echo "Removing full src backup folder..."
rm -rf src_BACKUP_*

# Remove VSCode snapshot zip
echo "Removing large zip snapshot..."
rm -f solana_meme_vscode_snapshot.zip

# Archive reports
echo "Archiving old reports..."
mkdir -p reports/archive/$(date +%Y-%m-%d)
mv reports/*.csv reports/archive/$(date +%Y-%m-%d)/ 2>/dev/null || echo "No CSV reports to move"

# Cargo clean
echo "Cleaning Rust build artifacts..."
cargo clean

# Format code
echo "Formatting all Rust code..."
cargo fmt --all

# Optional: run clippy (comment out if it complains)
# cargo clippy --fix --allow-dirty --allow-staged -- -D warnings || cargo clippy -- -D warnings

# Update tree.txt
echo "Updating tree.txt..."
tree -a --charset ascii -I 'target|.git|node_modules' > tree.txt

echo "✅ Cleanup complete!"
echo "   • Removed all backups and old files"
echo "   • Archived reports"
echo "   • Cleaned build"
echo "   • Formatted code"
echo "   • Updated tree.txt"
echo ""
echo "Your project is now clean and ready for action. 🚀"
