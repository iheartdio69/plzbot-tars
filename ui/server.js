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

const CALLS_JSON     = path.join(BOT_DATA, 'calls.json');
const STATE_JSON     = path.join(BOT_DATA, 'state.json');
const LOG_FILE       = require('os').homedir() + '/Desktop/TARS/logs/bot.log';
const POSITIONS_JSON = path.join(BOT_DATA, 'positions.json');
const ENV_FILE       = require('os').homedir() + '/Desktop/TARS/bots/plzbot/.env';

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

  if (req.url.startsWith('/api/wallet-balance')) {
    let PUBKEY = '';
    try {
      const envContent = fs.readFileSync(ENV_FILE, 'utf8');
      const match = envContent.match(/^TARS_WALLET_PUBKEY=(.+)$/m);
      if (match) PUBKEY = match[1].trim();
    } catch (_) {}
    const RPC = 'https://mainnet.helius-rpc.com/?api-key=bba3e681-5664-434e-a66c-75ff2f8dba24';
    const https = require('https');
    const url = new URL(RPC);
    const body = JSON.stringify({ jsonrpc:'2.0', id:1, method:'getBalance', params:[PUBKEY] });
    const opts = { hostname: url.hostname, path: url.pathname + url.search, method: 'POST', headers: { 'Content-Type': 'application/json' } };
    const req2 = https.request(opts, r2 => {
      let d = '';
      r2.on('data', c => d += c);
      r2.on('end', () => {
        try {
          const sol = (JSON.parse(d)?.result?.value || 0) / 1e9;
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ sol, pubkey: PUBKEY }));
        } catch {
          res.writeHead(200); res.end(JSON.stringify({ sol: 0 }));
        }
      });
    });
    req2.on('error', () => { res.writeHead(200); res.end(JSON.stringify({ sol: 0 })); });
    req2.write(body);
    req2.end();
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

  if (req.url.startsWith('/api/positions')) {
    // Real live positions with current price lookup
    let positions = [];
    let calls = [];
    try { positions = JSON.parse(fs.readFileSync(POSITIONS_JSON, 'utf8')) || []; } catch (_) {}
    try { calls = JSON.parse(fs.readFileSync(CALLS_JSON, 'utf8')) || []; } catch (_) {}

    const open = positions.filter(p => p.status === 'Open');
    const closed = positions.filter(p => p.status === 'Closed');

    const wins = closed.filter(p => (p.outcome||'').includes('WIN') || p.peak_mult >= 1.5).length;
    const losses = closed.filter(p => (p.outcome||'').includes('LOSS') || p.peak_mult < 1.0).length;
    const totalCalls = calls.length;
    const callWins = calls.filter(c => c.outcome === 'WIN').length;
    const callLosses = calls.filter(c => c.outcome === 'LOSS').length;
    const callWR = (callWins + callLosses) > 0 ? Math.round(callWins/(callWins+callLosses)*100) : 0;

    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      open,
      closed,
      wins,
      losses,
      totalCalls,
      callWins,
      callLosses,
      callWR,
    }));
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
