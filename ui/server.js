// plzbot UI server
// Serves the dashboard and proxies state from the bot's SQLite DB
// Usage: node ui/server.js
// Open: http://localhost:4269

const http = require('http');
const fs = require('fs');
const path = require('path');

const PORT = 4269;
const DB_PATH = path.join(__dirname, '..', 'data', 'STUPID_MAIN_DB_DO_NOT_TOUCH.sqlite');
const CALLS_JSON = path.join(__dirname, '..', 'calls.json');

let cachedState = null;
let lastRead = 0;

function loadState() {
  const now = Date.now();
  if (cachedState && now - lastRead < 2000) return cachedState;
  lastRead = now;

  let calls = [];
  try {
    const raw = fs.readFileSync(CALLS_JSON, 'utf8');
    calls = JSON.parse(raw) || [];
  } catch (_) {}

  cachedState = {
    coins_tracked: calls.length,
    active: [],
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
    tars_enabled: false,
    log: [],
  };

  return cachedState;
}

const server = http.createServer((req, res) => {
  if (req.url === '/api/state') {
    res.writeHead(200, { 'Content-Type': 'application/json', 'Access-Control-Allow-Origin': '*' });
    res.end(JSON.stringify(loadState()));
    return;
  }

  // Serve static files
  let filePath = path.join(__dirname, req.url === '/' ? 'index.html' : req.url);
  const ext = path.extname(filePath);
  const mimeTypes = {
    '.html': 'text/html',
    '.js': 'application/javascript',
    '.css': 'text/css',
  };

  fs.readFile(filePath, (err, data) => {
    if (err) {
      res.writeHead(404);
      res.end('Not found');
      return;
    }
    res.writeHead(200, { 'Content-Type': mimeTypes[ext] || 'text/plain' });
    res.end(data);
  });
});

server.listen(PORT, () => {
  console.log(`\n🤖 plzbot dashboard → http://localhost:${PORT}\n`);
});
