const express = require('express');
const fs = require('fs');
const path = require('path');
const { randomUUID } = require('crypto');
const { chromium } = require('playwright');

const app = express();
app.use(express.json({ limit: '10mb' }));

const PORT = process.env.PORT || 3100;
const HOST = process.env.PLAYWRIGHT_BRIDGE_HOST || process.env.HOST || '127.0.0.1';
const SESSION_TIMEOUT_MS = 15 * 60 * 1000; // 15 min inactivity timeout
const HEADLESS = /^(1|true|yes|on)$/i.test(process.env.PLAYWRIGHT_HEADLESS || '');
const LIVE_VIEW_PORT = Number.parseInt(process.env.PLAYWRIGHT_LIVE_VIEW_PORT || '6080', 10) || 6080;
const LIVE_VIEW_PATH = process.env.PLAYWRIGHT_LIVE_VIEW_PATH || '/vnc.html?autoconnect=1&resize=remote&path=websockify';
const LIVE_VIEW_ENABLED = !HEADLESS && Boolean(process.env.DISPLAY);
const PROFILE_ROOT = process.env.PLAYWRIGHT_PROFILE_ROOT || path.join(process.env.AGENTARK_DATA || '/app/data', 'browser-profiles');

// Active browser sessions: id -> { context, page, mode, claimed, claimedAt, lastActivity, cleanupTimer, diagnostics }
const sessions = new Map();

let browser = null;

async function ensureBrowser() {
  if (!browser || !browser.isConnected()) {
    const launchOptions = {
      headless: HEADLESS,
      args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'],
    };
    const executablePath = process.env.PLAYWRIGHT_EXECUTABLE_PATH;
    if (executablePath) {
      if (!fs.existsSync(executablePath)) {
        throw new Error(`Configured PLAYWRIGHT_EXECUTABLE_PATH does not exist: ${executablePath}`);
      }
      launchOptions.executablePath = executablePath;
    }
    const channel = process.env.PLAYWRIGHT_CHANNEL;
    if (channel) {
      launchOptions.channel = channel;
    }
    browser = await chromium.launch(launchOptions);
  }
  return browser;
}

async function sessionStatePayload(session) {
  let title = '';
  let url = '';
  try {
    title = await session.page.title();
  } catch (_) {}
  try {
    url = session.page.url();
  } catch (_) {}
  return {
    session_id: session.id,
    mode: session.mode || (HEADLESS ? 'headless' : 'interactive'),
    claimed: Boolean(session.claimed),
    claimed_at: session.claimedAt || null,
    title,
    url,
    live_view_enabled: LIVE_VIEW_ENABLED,
    live_view_port: LIVE_VIEW_ENABLED ? LIVE_VIEW_PORT : null,
    live_view_path: LIVE_VIEW_ENABLED ? LIVE_VIEW_PATH : null,
    profile_id: session.profileId || null,
    profile_name: session.profileName || null,
  };
}

function safeProfileId(raw) {
  const value = String(raw || '').trim();
  if (!value) return '';
  const safe = value.replace(/[^A-Za-z0-9_.-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 120);
  return safe || '';
}

function profileStorageStatePath(profileId) {
  const safeId = safeProfileId(profileId);
  if (!safeId) return '';
  return path.join(PROFILE_ROOT, safeId, 'storage-state.json');
}

async function saveSessionProfileState(session) {
  if (!session || !session.storageStatePath) return;
  try {
    await fs.promises.mkdir(path.dirname(session.storageStatePath), { recursive: true });
    await session.context.storageState({ path: session.storageStatePath });
  } catch (e) {
    console.warn(`Failed to save browser profile state for ${session.profileId || 'profile'}: ${e.message}`);
  }
}

async function settlePage(page, timeoutMs = 2500) {
  try {
    await page.waitForLoadState('domcontentloaded', { timeout: timeoutMs });
  } catch (_) {}
}

async function readPageSnapshot(page) {
  await settlePage(page);
  return page.evaluate(() => {
    const body = document.body;
    const bodyText = body ? body.innerText.substring(0, 5000) : '';
    const results = [];
    const interactiveSelectors = 'a, button, input, select, textarea, [role="button"], [role="link"], [onclick]';
    const els = document.querySelectorAll(interactiveSelectors);
    for (let i = 0; i < Math.min(els.length, 50); i++) {
      const el = els[i];
      const rect = el.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) continue;
      const tag = el.tagName.toLowerCase();
      const type = el.getAttribute('type') || '';
      const text = (el.innerText || el.value || el.getAttribute('aria-label') || el.getAttribute('placeholder') || '').trim().substring(0, 80);
      const name = el.getAttribute('name') || '';
      const id = el.id || '';
      const href = el.getAttribute('href') || '';
      results.push({
        index: results.length,
        tag, type, text, name, id, href,
        x: Math.round(rect.x + rect.width / 2),
        y: Math.round(rect.y + rect.height / 2),
      });
    }
    return {
      title: document.title || '',
      url: window.location.href,
      body_text: bodyText,
      elements: results,
    };
  });
}

async function readPageSnapshotWithRetry(page) {
  let lastError = null;
  for (let attempt = 0; attempt < 5; attempt++) {
    try {
      return await readPageSnapshot(page);
    } catch (e) {
      lastError = e;
      await page.waitForTimeout(200 + attempt * 250).catch(() => {});
    }
  }
  throw lastError || new Error('Unable to read page snapshot');
}

function touchSession(session) {
  session.lastActivity = Date.now();
  if (session.cleanupTimer) clearTimeout(session.cleanupTimer);
  session.cleanupTimer = setTimeout(() => destroySession(session.id), SESSION_TIMEOUT_MS);
}

function recordDiagnostic(session, entry) {
  if (!session || !Array.isArray(session.diagnostics)) return;
  const normalized = {
    time: new Date().toISOString(),
    kind: String(entry.kind || 'browser'),
    severity: String(entry.severity || 'info'),
    message: String(entry.message || '').slice(0, 600),
    url: String(entry.url || '').slice(0, 600),
    resource_type: String(entry.resource_type || ''),
  };
  if (!normalized.message && !normalized.url) return;
  session.diagnostics.push(normalized);
  if (session.diagnostics.length > 80) {
    session.diagnostics.splice(0, session.diagnostics.length - 80);
  }
}

async function destroySession(id) {
  const session = sessions.get(id);
  if (!session) return;
  if (session.cleanupTimer) clearTimeout(session.cleanupTimer);
  await saveSessionProfileState(session);
  try { await session.context.close(); } catch (_) {}
  sessions.delete(id);
  console.log(`Session ${id} destroyed (${sessions.size} remaining)`);
}

// Health check
app.get('/health', (req, res) => {
  res.json({ status: 'ok', sessions: sessions.size });
});

// Create a new browser session
app.post('/session', async (req, res) => {
  try {
    const b = await ensureBrowser();
    const requestedMode = String(req.body?.mode || '').trim().toLowerCase() || (HEADLESS ? 'headless' : 'interactive');
    const profile = req.body?.profile && typeof req.body.profile === 'object' ? req.body.profile : null;
    const profileId = safeProfileId(profile?.id || req.body?.profile_id);
    const profileName = String(profile?.name || req.body?.profile_name || '').trim();
    const storageStatePath = profileId ? profileStorageStatePath(profileId) : '';
    const contextOptions = {
      viewport: { width: 1280, height: 720 },
      userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
    };
    if (storageStatePath && fs.existsSync(storageStatePath)) {
      try {
        JSON.parse(fs.readFileSync(storageStatePath, 'utf8'));
        contextOptions.storageState = storageStatePath;
      } catch (e) {
        console.warn(`Ignoring unreadable browser profile state ${storageStatePath}: ${e.message}`);
      }
    }
    const context = await b.newContext(contextOptions);
    const page = await context.newPage();
    const id = randomUUID();
    const session = {
      id,
      context,
      page,
      mode: requestedMode,
      claimed: false,
      claimedAt: null,
      lastActivity: Date.now(),
      cleanupTimer: null,
      diagnostics: [],
      profileId,
      profileName,
      storageStatePath,
    };
    page.on('console', (msg) => {
      const type = msg.type();
      recordDiagnostic(session, {
        kind: 'console',
        severity: type === 'error' ? 'error' : type === 'warning' ? 'warning' : 'info',
        message: msg.text(),
        url: page.url(),
      });
    });
    page.on('pageerror', (err) => {
      recordDiagnostic(session, {
        kind: 'pageerror',
        severity: 'error',
        message: err && err.message ? err.message : String(err || ''),
        url: page.url(),
      });
    });
    page.on('requestfailed', (request) => {
      const failure = request.failure();
      recordDiagnostic(session, {
        kind: 'requestfailed',
        severity: 'warning',
        message: failure && failure.errorText ? failure.errorText : 'request failed',
        url: request.url(),
        resource_type: request.resourceType(),
      });
    });
    page.on('response', (response) => {
      const status = response.status();
      if (status >= 400) {
        recordDiagnostic(session, {
          kind: 'response',
          severity: status >= 500 ? 'error' : 'warning',
          message: `HTTP ${status}`,
          url: response.url(),
        });
      }
    });
    sessions.set(id, session);
    touchSession(session);
    console.log(`Session ${id} created (${sessions.size} total)`);
    res.json(await sessionStatePayload(session));
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Close a session
app.delete('/session/:id', async (req, res) => {
  const { id } = req.params;
  if (!sessions.has(id)) return res.status(404).json({ error: 'Session not found' });
  await destroySession(id);
  res.json({ status: 'closed' });
});

app.get('/session/:id/state', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    res.json(await sessionStatePayload(session));
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

app.post('/session/:id/claim', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    session.claimed = true;
    session.claimedAt = new Date().toISOString();
    if (typeof session.page.bringToFront === 'function') {
      await session.page.bringToFront();
    }
    res.json(await sessionStatePayload(session));
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

app.post('/session/:id/release', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    session.claimed = false;
    session.claimedAt = null;
    res.json(await sessionStatePayload(session));
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Navigate to URL
app.post('/session/:id/navigate', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    const { url } = req.body;
    await session.page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
    res.json({ status: 'ok', url: session.page.url(), title: await session.page.title() });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Navigate back
app.post('/session/:id/back', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    await session.page.goBack({ waitUntil: 'domcontentloaded', timeout: 30000 });
    res.json({ status: 'ok', url: session.page.url(), title: await session.page.title() });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Take screenshot
app.get('/session/:id/screenshot', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    const buffer = await session.page.screenshot({ type: 'png', fullPage: false });
    res.set('Content-Type', 'image/png');
    res.send(buffer);
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Click element
app.post('/session/:id/click', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    const { selector, text, x, y } = req.body;
    if (x !== undefined && y !== undefined) {
      await session.page.mouse.click(x, y);
    } else if (text) {
      await session.page.getByText(text, { exact: false }).first().click({ timeout: 5000 });
    } else if (selector) {
      await session.page.click(selector, { timeout: 5000 });
    } else {
      return res.status(400).json({ error: 'Provide selector, text, or x/y coordinates' });
    }
    await session.page.waitForTimeout(250);
    await settlePage(session.page);
    res.json({ status: 'ok' });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Type text
app.post('/session/:id/type', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    const { selector, text, clear } = req.body;
    if (selector) {
      if (clear) await session.page.fill(selector, '');
      await session.page.fill(selector, text || '');
    } else {
      // Type into currently focused element
      if (clear) {
        await session.page.keyboard.down('Control');
        await session.page.keyboard.press('a');
        await session.page.keyboard.up('Control');
        await session.page.keyboard.press('Backspace');
      }
      await session.page.keyboard.type(text || '', { delay: 30 });
    }
    await settlePage(session.page, 1000);
    res.json({ status: 'ok' });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Scroll page
app.post('/session/:id/scroll', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    const { direction, amount } = req.body;
    const pixels = amount || 500;
    const dy = direction === 'up' ? -pixels : pixels;
    await session.page.mouse.wheel(0, dy);
    await session.page.waitForTimeout(300);
    res.json({ status: 'ok' });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Press keyboard key
app.post('/session/:id/press', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    const { key } = req.body;
    await session.page.keyboard.press(key);
    await session.page.waitForTimeout(250);
    await settlePage(session.page);
    res.json({ status: 'ok' });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Get page content and interactive elements
app.get('/session/:id/content', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    const snapshot = await readPageSnapshotWithRetry(session.page);
    res.json({ ...snapshot, diagnostics: session.diagnostics || [] });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Evaluate JavaScript on the page
app.post('/session/:id/evaluate', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    const { expression } = req.body;
    const result = await session.page.evaluate(expression);
    res.json({ result });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Wait for navigation/selector
app.post('/session/:id/wait', async (req, res) => {
  const session = sessions.get(req.params.id);
  if (!session) return res.status(404).json({ error: 'Session not found' });
  touchSession(session);
  try {
    const { selector, timeout } = req.body;
    const ms = timeout || 10000;
    if (selector) {
      await session.page.waitForSelector(selector, { timeout: ms });
    } else {
      await session.page.waitForLoadState('domcontentloaded', { timeout: ms });
    }
    res.json({ status: 'ok' });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// List active sessions
app.get('/sessions', (req, res) => {
  const list = [];
  for (const [id, s] of sessions) {
    list.push({
      id,
      mode: s.mode,
      claimed: Boolean(s.claimed),
      lastActivity: s.lastActivity,
      age_ms: Date.now() - s.lastActivity,
    });
  }
  res.json({ sessions: list });
});

// Graceful shutdown
process.on('SIGTERM', async () => {
  console.log('Shutting down...');
  for (const [id] of sessions) await destroySession(id);
  if (browser) await browser.close();
  process.exit(0);
});

app.listen(PORT, HOST, () => {
  console.log(`Playwright bridge listening on ${HOST}:${PORT}`);
});
