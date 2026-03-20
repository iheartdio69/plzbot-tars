// plzbot UI server
// Serves the dashboard and tails the bot's log + state files
// Usage: node ui/server.js
// Open: http://localhost:4269

const http = require('http');
const fs = require('fs');
const path = require('path');

const PORT = 4269;
const CALLS_JSON = path.join(__dirname, '..', 'data', 'calls.json');
const STATE_JSON = path.join(__dirname, '..', 'data', 'state.json');
const LOG_FILE   = path.join(__dirname, '..', 'data', 'bot.log');

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
    tars_enabled: live.tars_enabled || false,
    log: logLines.slice(-100),
    bot_running: fs.existsSync(LOG_FILE),
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
