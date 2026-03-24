// plzbot UI server
// Serves the dashboard and tails the bot's log + state files
// Usage: node ui/server.js
// Open: http://localhost:4269

const http = require('http');
const fs = require('fs');
const path = require('path');

const PORT = 4269;
// Bot runs from ~/.openclaw/workspace/plzbot — data lives there
const BOT_DATA = process.env.BOT_DATA_DIR
  || require('os').homedir() + '/.openclaw/workspace/plzbot/data';

const CALLS_JSON   = path.join(BOT_DATA, 'calls.json');
const STATE_JSON   = path.join(BOT_DATA, 'state.json');
const LOG_FILE     = require('os').homedir() + '/Desktop/TARS/logs/bot.log';
const WALLETS_JSON = path.join(BOT_DATA, 'paper_wallet_trades.json');

let cachedState = null;
let lastRead = 0;
const logLines = [];
const MAX_LOG = 200;

// Tail the log file — watch for appends
function watchLog() {
  if (!fs.existsSync(LOG_FILE)) {
    setTimeout(watchLog, 2000);
    return;
  }

  let size = fs.statSync(LOG_FILE).size;
  fs.watchFile(LOG_FILE, { interval: 500 }, (curr) => {
    if (curr.size <= size) return;
    const stream = fs.createReadStream(LOG_FILE, { start: size, end: curr.size });
    size = curr.size;
    let buf = '';
    stream.on('data', d => buf += d.toString());
    stream.on('end', () => {
      buf.split('\n').filter(l => l.trim()).forEach(line => {
        logLines.push({ ts: Date.now(), line });
        if (logLines.length > MAX_LOG) logLines.shift();
      });
    });
  });

  // Initial read
  const content = fs.readFileSync(LOG_FILE, 'utf8');
  content.split('\n').filter(l => l.trim()).slice(-MAX_LOG).forEach(line => {
    logLines.push({ ts: Date.now(), line });
  });
  size = fs.statSync(LOG_FILE).size;
}

watchLog();

function loadState() {
  const now = Date.now();
  if (cachedState && now - lastRead < 1000) return cachedState;
  lastRead = now;

  let calls = [];
  try { calls = JSON.parse(fs.readFileSync(CALLS_JSON, 'utf8')) || []; } catch (_) {}

  let live = {};
  try { live = JSON.parse(fs.readFileSync(STATE_JSON, 'utf8')) || {}; } catch (_) {}

  cachedState = {
    coins_tracked: live.coins || 0,
    active: live.active || [],
    calls: calls.map(c => ({
      mint: c.mint,
      score: c.score,
      call_ts: c.call_ts,
      fdv_at_call: c.fdv_at_call || null,
      outcome: c.outcome || null,
      wallets_t5: c.wallets_t5 || null,
      wallets_t15: c.wallets_t15 || null,
      tx_t5: c.tx_t5 || null,
      tx_t15: c.tx_t15 || null,
    })),
    // active is a list of mint strings — convert to objects for the UI
    coins: (live.active || []).map(mint => ({
      mint,
      active: true,
      score: calls.find(c => c.mint === mint)?.score || 0,
      fdv: calls.find(c => c.mint === mint)?.fdv_at_call || 0,
      first_seen_ts: Math.floor(Date.now() / 1000) - 300,
      wallets: 0,
    })),
    tars_enabled: live.tars_enabled || false,
    log: logLines.slice(-100).map((l, i) => ({ ts: l.ts || i, msg: l.line || l.msg || '' })),
    bot_running: true,
  };

  return cachedState;
}

const server = http.createServer((req, res) => {
  res.setHeader('Access-Control-Allow-Origin', '*');

  if (req.url === '/api/state') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(loadState()));
    return;
  }

  if (req.url === '/api/log') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(logLines.slice(-100)));
    return;
  }

  if (req.url.startsWith('/api/sol-price')) {
    // Fetch SOL price from CoinGecko (free, no key)
    try {
      const fetch = require('https');
      fetch.get('https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd', (r) => {
        let data = '';
        r.on('data', d => data += d);
        r.on('end', () => {
          try {
            const price = JSON.parse(data)?.solana?.usd || 130;
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ price }));
          } catch {
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ price: 130 }));
          }
        });
      }).on('error', () => {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ price: 130 }));
      });
    } catch {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ price: 130 }));
    }
    return;
  }

  if (req.url.startsWith('/api/wallets')) {
    let trades = [];
    try { trades = JSON.parse(fs.readFileSync(WALLETS_JSON, 'utf8')) || []; } catch (_) {}

    const strategies = ['LOGIC_V2', 'GUT', 'GUT_V2', 'DIAMOND', 'BALANCED', 'SCALPER', 'SNIPER', 'LOGIC'];
    const sizes = { LOGIC: 1.0, LOGIC_V2: 1.0, GUT: 0.25, GUT_V2: 0.25, DIAMOND: 0.1, BALANCED: 1.0, SNIPER: 2.0, SCALPER: 0.75 };

    const stats = strategies.map(name => {
      const st = trades.filter(t => t.strategy === name);
      const closed = st.filter(t => t.status === 'Closed');
      const open = st.filter(t => t.status === 'Open');
      const wins = closed.filter(t => t.pnl_sol > 0).length;
      const losses = closed.filter(t => t.pnl_sol <= 0).length;
      const realizedPnl = closed.reduce((s, t) => s + (t.pnl_sol || 0), 0);
      // Unrealized: sum of (peak_mult - 1) * sol_in for open trades
      const unrealizedPnl = open.reduce((s, t) => s + ((t.peak_mult || 1) - 1) * (t.sol_in || sizes[name] || 0), 0);
      const wr = (wins + losses) > 0 ? Math.round(wins / (wins + losses) * 100) : 0;
      const best = closed.length > 0 ? Math.max(...closed.map(t => t.peak_mult || 1)) : 0;
      return { name, total: st.length, open: open.length, wins, losses, wr, realizedPnl, unrealizedPnl, best };
    });

    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(stats));
    return;
  }

  let filePath = path.join(__dirname, req.url === '/' ? 'index.html' : req.url);
  const ext = path.extname(filePath);
  const mimeTypes = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css' };

  fs.readFile(filePath, (err, data) => {
    if (err) { res.writeHead(404); res.end('Not found'); return; }
    res.writeHead(200, { 'Content-Type': mimeTypes[ext] || 'text/plain' });
    res.end(data);
  });
});

server.listen(PORT, () => {
  console.log(`\n🤖 plzbot dashboard → http://localhost:${PORT}\n`);
});
