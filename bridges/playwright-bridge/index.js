const express = require('express');
const fs = require('fs');
const path = require('path');
const { spawn, spawnSync } = require('child_process');
const { randomUUID } = require('crypto');
const { chromium, firefox } = require('playwright');
const {
  activeManualLoginSessionForUserDataDir,
  browserProfileLockInfo,
  buildManualLoginBrowserArgs,
  closeManualLoginLog,
  openManualLoginLog,
  savedProfileUsesPersistentContext,
  waitForBrowserProfileUnlock,
  waitForManualLoginBrowserReady,
  writeManualLoginLog,
} = require('./manual-login');

const app = express();
app.use(express.json({ limit: '10mb' }));

function readPositiveIntEnv(names, fallback) {
  const keys = Array.isArray(names) ? names : [names];
  for (const key of keys) {
    const value = Number.parseInt(process.env[key] || '', 10);
    if (Number.isFinite(value) && value > 0) return value;
  }
  return fallback;
}

const PORT = process.env.PORT || 3100;
const HOST = process.env.PLAYWRIGHT_BRIDGE_HOST || process.env.HOST || '127.0.0.1';
const SESSION_TIMEOUT_MS = 15 * 60 * 1000; // 15 min inactivity timeout
const HEADLESS = /^(1|true|yes|on)$/i.test(process.env.PLAYWRIGHT_HEADLESS || '');
const LIVE_VIEW_PORT = Number.parseInt(process.env.PLAYWRIGHT_LIVE_VIEW_PORT || '6080', 10) || 6080;
const LIVE_VIEW_PATH = process.env.PLAYWRIGHT_LIVE_VIEW_PATH || '/vnc.html?autoconnect=1&resize=scale&path=websockify';
const LIVE_VIEW_ENABLED = !HEADLESS && Boolean(process.env.DISPLAY);
const PROFILE_ROOT = process.env.PLAYWRIGHT_PROFILE_ROOT || path.join(process.env.AGENTARK_DATA || '/app/data', 'browser-profiles');
const BROWSER_WIDTH = readPositiveIntEnv(['PLAYWRIGHT_BROWSER_WIDTH', 'PLAYWRIGHT_VIEWPORT_WIDTH'], 1920);
const BROWSER_HEIGHT = readPositiveIntEnv(['PLAYWRIGHT_BROWSER_HEIGHT', 'PLAYWRIGHT_VIEWPORT_HEIGHT'], 1080);

// Active browser sessions: id -> { context, page, mode, claimed, claimedAt, lastActivity, cleanupTimer, diagnostics }
const sessions = new Map();

let browser = null;

function normalizeBrowserName(raw) {
  const value = String(raw || '').trim().toLowerCase();
  if (value === 'edge' || value === 'msedge' || value === 'microsoft-edge') return 'edge';
  if (value === 'firefox') return 'firefox';
  if (value === 'chromium') return 'chromium';
  return 'chrome';
}

function browserTypeFor(browserName) {
  if (browserName === 'firefox') return firefox;
  return chromium;
}

function browserChannelFor(browserName, targetKind) {
  const configured = String(process.env.PLAYWRIGHT_CHANNEL || '').trim();
  if (configured) return configured;
  if (browserName === 'edge') return 'msedge';
  if (browserName === 'chrome' && targetKind === 'host') return 'chrome';
  return '';
}

function buildLaunchOptions(browserName, targetKind) {
  const launchOptions = {
    headless: HEADLESS,
  };
  if (browserName !== 'firefox') {
    const args = ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'];
    if (!HEADLESS) {
      args.push('--start-maximized', `--window-size=${BROWSER_WIDTH},${BROWSER_HEIGHT}`);
    }
    launchOptions.args = args;
  }
  const executablePath = process.env.PLAYWRIGHT_EXECUTABLE_PATH;
  if (executablePath) {
    if (!fs.existsSync(executablePath)) {
      throw new Error(`Configured PLAYWRIGHT_EXECUTABLE_PATH does not exist: ${executablePath}`);
    }
    launchOptions.executablePath = executablePath;
  }
  const channel = browserName === 'firefox' ? '' : browserChannelFor(browserName, targetKind);
  if (channel) {
    launchOptions.channel = channel;
  }
  return launchOptions;
}

async function ensureBrowser() {
  if (!browser || !browser.isConnected()) {
    browser = await chromium.launch(buildLaunchOptions('chromium', 'sandbox'));
  }
  return browser;
}

async function sessionStatePayload(session) {
  let title = '';
  let url = '';
  if (session.page) {
    try {
      title = await session.page.title();
    } catch (_) {}
    try {
      url = session.page.url();
    } catch (_) {}
  } else if (session.externalProcess) {
    title = session.externalTitle || 'External browser';
    url = session.externalUrl || '';
  }
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

function profileStorageDir(profileId) {
  const safeId = safeProfileId(profileId);
  if (!safeId) return '';
  return path.join(PROFILE_ROOT, safeId);
}

function profileBrowser(profile, req) {
  return normalizeBrowserName(profile?.browser || req.body?.browser || process.env.PLAYWRIGHT_BROWSER || '');
}

function profileTargetKind(profile, req) {
  const raw = String(profile?.target_kind || req.body?.target_kind || '').trim().toLowerCase();
  if (raw === 'host' || raw === 'remote_cdp') return raw;
  return 'sandbox';
}

function profileUserDataDir(profileId, profile, req) {
  const explicit = String(
    profile?.target_profile_path ||
    profile?.targetProfilePath ||
    req.body?.target_profile_path ||
    ''
  ).trim();
  if (explicit) return path.resolve(explicit);
  const base = profileStorageDir(profileId);
  if (!base) return '';
  return path.join(base, profileTargetKind(profile, req) === 'host' ? 'real-browser-profile' : 'browser-profile');
}

function manualLoginRequested(profile, req) {
  return Boolean(profile?.manual_login || profile?.manualLogin || req.body?.manual_login);
}

function externalBrowserInitialUrl(profile, req) {
  return String(
    profile?.target_endpoint ||
    profile?.targetEndpoint ||
    req.body?.url ||
    'about:blank'
  ).trim() || 'about:blank';
}

function firstAvailableCommand(candidates) {
  const probe = process.platform === 'win32' ? 'where' : 'which';
  for (const candidate of candidates.filter(Boolean)) {
    if (path.isAbsolute(candidate) && fs.existsSync(candidate)) return candidate;
    const result = spawnSync(probe, [candidate], { encoding: 'utf8' });
    if (result.status === 0) {
      const found = String(result.stdout || '').split(/\r?\n/).map((line) => line.trim()).find(Boolean);
      return found || candidate;
    }
  }
  return '';
}

function externalBrowserCommand(browserName) {
  const configured = String(
    process.env.PLAYWRIGHT_REAL_BROWSER_EXECUTABLE ||
    process.env.AGENTARK_REAL_BROWSER_EXECUTABLE ||
    ''
  ).trim();
  if (configured) {
    if (!fs.existsSync(configured)) {
      throw new Error(`Configured real browser executable does not exist: ${configured}`);
    }
    return configured;
  }
  const windowsChrome = [
    'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
  ];
  const windowsEdge = [
    'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
  ];
  const candidates = browserName === 'edge'
    ? ['microsoft-edge', 'msedge', ...windowsEdge]
    : browserName === 'firefox'
      ? ['firefox']
      : browserName === 'chromium'
        ? ['chromium', 'chromium-browser']
        : ['google-chrome', 'google-chrome-stable', 'chrome', ...windowsChrome];
  const command = firstAvailableCommand(candidates);
  if (!command) {
    throw new Error(`No real ${browserName} executable was found. Install ${browserName} or set PLAYWRIGHT_REAL_BROWSER_EXECUTABLE to the browser path.`);
  }
  return command;
}

function browserProfileLockedError(lockInfo, userDataDir) {
  const owner = lockInfo.pid ? `process ${lockInfo.pid}` : 'another Chromium process';
  const state = lockInfo.active ? `locked by ${owner}` : `blocked by ${lockInfo.marker || 'a Chromium lock marker'}`;
  const err = new Error(`Saved browser profile is ${state}; close the existing browser session or clear the profile lock before launching it again.`);
  err.statusCode = 409;
  err.code = 'PROFILE_LOCKED';
  err.details = {
    marker: lockInfo.marker || null,
    pid: lockInfo.pid || null,
    active: Boolean(lockInfo.active),
    stale: Boolean(lockInfo.stale),
    profile_path: userDataDir,
  };
  return err;
}

async function launchExternalBrowserProfile(browserName, userDataDir, initialUrl) {
  const executable = externalBrowserCommand(browserName);
  if (executable && /[\\/]/.test(executable) && !fs.existsSync(executable)) {
    throw new Error(`Browser executable not found for ${browserName}: ${executable}`);
  }
  if (browserName !== 'firefox') {
    const lockInfo = browserProfileLockInfo(userDataDir);
    if (lockInfo.locked) throw browserProfileLockedError(lockInfo, userDataDir);
  }
  const args = buildManualLoginBrowserArgs({
    browserName,
    userDataDir,
    initialUrl,
    width: BROWSER_WIDTH,
    height: BROWSER_HEIGHT,
  });
  const displayValue = process.env.DISPLAY && process.env.DISPLAY.trim()
    ? process.env.DISPLAY
    : ':99';
  const launchEnv = { ...process.env, DISPLAY: displayValue };
  const logPath = '/tmp/agentark-manual-login.log';
  const log = openManualLoginLog(logPath);
  writeManualLoginLog(log, `\n[${new Date().toISOString()}] launching ${executable} (${browserName}) DISPLAY=${displayValue} url=${initialUrl}\n`);
  const child = spawn(executable, args, {
    detached: true,
    stdio: ['ignore', log ? 'pipe' : 'ignore', log ? 'pipe' : 'ignore'],
    env: launchEnv,
  });
  if (log) {
    if (child.stdout) child.stdout.pipe(log.stream, { end: false });
    if (child.stderr) child.stderr.pipe(log.stream, { end: false });
  }
  child.on('error', (err) => {
    console.warn(`Manual-login browser spawn error (${executable}): ${err.message}`);
    writeManualLoginLog(log, `[error] ${err.message}\n`);
  });
  child.on('exit', (code, signal) => {
    const reason = signal ? `signal ${signal}` : `code ${code}`;
    if (code !== 0 && code !== null) {
      console.warn(`Manual-login browser exited early (${executable}, ${reason}). Check ${logPath} or DISPLAY=${displayValue} availability (x11vnc/openbox).`);
    }
    writeManualLoginLog(log, `[exit] ${reason}\n`);
    closeManualLoginLog(log);
  });
  try {
    await waitForManualLoginBrowserReady(child, {
      timeoutMs: readPositiveIntEnv(['PLAYWRIGHT_MANUAL_LOGIN_READY_TIMEOUT_MS'], 2000),
      executable,
      displayValue,
      logPath,
    });
  } catch (err) {
    writeManualLoginLog(log, `[startup-failed] ${err.message}\n`);
    closeManualLoginLog(log);
    if (!child.killed && child.exitCode === null) {
      try { child.kill(); } catch (_) {}
    }
    throw err;
  }
  child.unref();
  return { process: child, executable, initialUrl };
}

async function saveSessionProfileState(session) {
  if (!session || !session.storageStatePath) return;
  if (session.persistentProfile) return;
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

function waitForPotentialNavigation(page, timeoutMs = 5000) {
  return page.waitForNavigation({ waitUntil: 'domcontentloaded', timeout: timeoutMs }).catch(() => null);
}

async function settleAfterAction(page, navigationPromise, timeoutMs = 5000) {
  await Promise.race([
    navigationPromise,
    page.waitForTimeout(500),
  ]).catch(() => null);
  await settlePage(page, timeoutMs);
}

async function interactiveElementCenter(page, targetIndex) {
  return page.evaluate((targetIndex) => {
    const visible = (el) => {
      if (!el || !(el instanceof HTMLElement)) return false;
      const style = window.getComputedStyle(el);
      if (style.visibility === 'hidden' || style.display === 'none') return false;
      const rect = el.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    const labelFor = (el) => (
      el.innerText ||
      el.value ||
      el.getAttribute('aria-label') ||
      el.getAttribute('placeholder') ||
      el.getAttribute('name') ||
      el.id ||
      el.getAttribute('href') ||
      el.tagName.toLowerCase() ||
      ''
    ).trim().substring(0, 80);
    const interactiveSelectors = 'a, button, input, select, textarea, [role="button"], [role="link"], [onclick]';
    const els = document.querySelectorAll(interactiveSelectors);
    let visibleIndex = 0;
    for (let i = 0; i < Math.min(els.length, 50); i++) {
      const el = els[i];
      if (!visible(el)) continue;
      if (visibleIndex === targetIndex) {
        el.scrollIntoView({ block: 'center', inline: 'center' });
        const rect = el.getBoundingClientRect();
        return {
          ok: true,
          x: Math.round(rect.x + rect.width / 2),
          y: Math.round(rect.y + rect.height / 2),
          label: labelFor(el),
          tag: el.tagName.toLowerCase(),
        };
      }
      visibleIndex += 1;
    }
    return { ok: false, error: `No interactive element ${targetIndex} is visible on the current page.` };
  }, targetIndex);
}

async function readPageSnapshot(page) {
  await settlePage(page);
  return page.evaluate(() => {
    const body = document.body;
    const bodyText = body ? body.innerText.substring(0, 5000) : '';
    const results = [];
    const interactiveSelectors = 'a, button, input, select, textarea, [role="button"], [role="link"], [onclick]';
    const els = document.querySelectorAll(interactiveSelectors);
    const visible = (el) => {
      if (!el || !(el instanceof HTMLElement)) return false;
      const style = window.getComputedStyle(el);
      if (style.visibility === 'hidden' || style.display === 'none') return false;
      const rect = el.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    for (let i = 0; i < Math.min(els.length, 50); i++) {
      const el = els[i];
      const rect = el.getBoundingClientRect();
      if (!visible(el)) continue;
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

async function focusEditableTarget(page, selector) {
  return page.evaluate((selector) => {
    const visible = (el) => {
      if (!el || !(el instanceof HTMLElement)) return false;
      const style = window.getComputedStyle(el);
      if (style.visibility === 'hidden' || style.display === 'none') return false;
      const rect = el.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    const editable = (el) => {
      if (!visible(el)) return false;
      const tag = el.tagName.toLowerCase();
      if (tag === 'textarea') return !el.disabled && !el.readOnly;
      if (tag === 'input') {
        const type = String(el.getAttribute('type') || 'text').toLowerCase();
        return !el.disabled && !el.readOnly && !['hidden', 'button', 'submit', 'reset', 'checkbox', 'radio', 'file', 'image'].includes(type);
      }
      return el.isContentEditable;
    };
    const score = (el, selected) => {
      const tag = el.tagName.toLowerCase();
      const type = String(el.getAttribute('type') || '').toLowerCase();
      const rect = el.getBoundingClientRect();
      let value = selected ? 100 : 0;
      if (tag === 'input') value += 20;
      if (tag === 'textarea') value += 18;
      if (el.isContentEditable) value += 12;
      if (type === 'search') value += 12;
      value += Math.min(rect.width * rect.height / 10000, 10);
      return value;
    };
    const collect = (css, selected) => {
      if (!css) return [];
      try {
        return Array.from(document.querySelectorAll(css))
          .filter(editable)
          .map((el) => ({ el, selected }));
      } catch (e) {
        return [];
      }
    };
    let candidates = collect(selector, true);
    if (candidates.length === 0) {
      candidates = Array.from(document.querySelectorAll('input, textarea, [contenteditable="true"]'))
        .filter(editable)
        .map((el) => ({ el, selected: false }));
    }
    candidates.sort((left, right) => score(right.el, right.selected) - score(left.el, left.selected));
    const target = candidates[0]?.el;
    if (!target) {
      return { ok: false, error: 'No visible editable browser field is available on the current page.' };
    }
    target.focus();
    if (typeof target.select === 'function') {
      target.select();
    }
    return {
      ok: true,
      tag: target.tagName.toLowerCase(),
      type: target.getAttribute('type') || '',
      id: target.id || '',
      name: target.getAttribute('name') || '',
      selected: Boolean(candidates[0]?.selected),
    };
  }, selector || null);
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
  if (session.externalProcess && !session.externalProcess.killed) {
    try { session.externalProcess.kill(); } catch (_) {}
  }
  if (session.context) {
    try { await session.context.close(); } catch (_) {}
  }
  sessions.delete(id);
  console.log(`Session ${id} destroyed (${sessions.size} remaining)`);
}

function sendBridgeError(res, err) {
  const status = Number.parseInt(String(err && (err.statusCode || err.status) || ''), 10);
  const body = { error: err && err.message ? err.message : String(err || 'Unknown error') };
  if (err && err.code) body.code = err.code;
  if (err && err.details) body.details = err.details;
  res.status(status >= 400 && status < 600 ? status : 500).json(body);
}

// Health check
app.get('/health', (req, res) => {
  res.json({ status: 'ok', sessions: sessions.size });
});

// Create a new browser session
app.post('/session', async (req, res) => {
  try {
    const requestedMode = String(req.body?.mode || '').trim().toLowerCase() || (HEADLESS ? 'headless' : 'interactive');
    const profile = req.body?.profile && typeof req.body.profile === 'object' ? req.body.profile : null;
    const profileId = safeProfileId(profile?.id || req.body?.profile_id);
    const profileName = String(profile?.name || req.body?.profile_name || '').trim();
    const browserName = profileBrowser(profile, req);
    const targetKind = profileTargetKind(profile, req);
    const manualLogin = manualLoginRequested(profile, req);
    const storageStatePath = profileId ? profileStorageStatePath(profileId) : '';
    const contextOptions = {
      viewport: null,
      screen: { width: BROWSER_WIDTH, height: BROWSER_HEIGHT },
    };
    const persistentProfile = savedProfileUsesPersistentContext({
      profileId,
      manualLogin,
      targetKind,
    });
    const userDataDir = persistentProfile ? profileUserDataDir(profileId, profile, req) : '';
    if (!persistentProfile && storageStatePath && fs.existsSync(storageStatePath)) {
      try {
        JSON.parse(fs.readFileSync(storageStatePath, 'utf8'));
        contextOptions.storageState = storageStatePath;
      } catch (e) {
        console.warn(`Ignoring unreadable browser profile state ${storageStatePath}: ${e.message}`);
      }
    }
    let context;
    let page;
    let externalProcess = null;
    let externalTitle = '';
    let externalUrl = '';
    if (persistentProfile && manualLogin) {
      await fs.promises.mkdir(userDataDir, { recursive: true });
      const activeSession = activeManualLoginSessionForUserDataDir(sessions, userDataDir);
      if (activeSession) {
        touchSession(activeSession);
        return res.json(await sessionStatePayload(activeSession));
      }
      const launched = await launchExternalBrowserProfile(browserName, userDataDir, externalBrowserInitialUrl(profile, req));
      externalProcess = launched.process;
      externalTitle = `${browserName} manual login`;
      externalUrl = launched.initialUrl;
    } else if (persistentProfile) {
      await fs.promises.mkdir(userDataDir, { recursive: true });
      const activeSession = activeManualLoginSessionForUserDataDir(sessions, userDataDir);
      if (activeSession) {
        await destroySession(activeSession.id);
      }
      const lockInfo = await waitForBrowserProfileUnlock(userDataDir, {
        timeoutMs: readPositiveIntEnv(['PLAYWRIGHT_PROFILE_UNLOCK_TIMEOUT_MS'], 5000),
        intervalMs: readPositiveIntEnv(['PLAYWRIGHT_PROFILE_UNLOCK_POLL_MS'], 100),
      });
      if (lockInfo.locked) throw browserProfileLockedError(lockInfo, userDataDir);
      const browserType = browserTypeFor(browserName);
      context = await browserType.launchPersistentContext(userDataDir, {
        ...buildLaunchOptions(browserName, targetKind),
        ...contextOptions,
      });
      page = context.pages()[0] || await context.newPage();
    } else {
      const b = await ensureBrowser();
      context = await b.newContext(contextOptions);
      page = await context.newPage();
    }
    const id = randomUUID();
    const session = {
      id,
      context,
      page,
      mode: requestedMode,
      browserName,
      targetKind,
      persistentProfile,
      manualLogin,
      userDataDir,
      externalProcess,
      externalTitle,
      externalUrl,
      claimed: false,
      claimedAt: null,
      lastActivity: Date.now(),
      cleanupTimer: null,
      diagnostics: [],
      profileId,
      profileName,
      storageStatePath,
    };
    if (page) {
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
    }
    sessions.set(id, session);
    touchSession(session);
    console.log(`Session ${id} created (${sessions.size} total)`);
    res.json(await sessionStatePayload(session));
  } catch (e) {
    sendBridgeError(res, e);
  }
});

// Close a session
app.delete('/session/:id', async (req, res) => {
  const { id } = req.params;
  if (!sessions.has(id)) return res.status(404).json({ error: 'Session not found' });
  await destroySession(id);
  res.json({ status: 'closed' });
});

app.delete('/profile/:id', async (req, res) => {
  try {
    const profileId = safeProfileId(req.params.id);
    if (!profileId) return res.status(400).json({ error: 'Profile id required' });

    let closedSessions = 0;
    for (const [sessionId, session] of Array.from(sessions.entries())) {
      if (session.profileId === profileId) {
        await destroySession(sessionId);
        closedSessions += 1;
      }
    }

    const root = path.resolve(PROFILE_ROOT);
    const target = path.resolve(profileStorageDir(profileId));
    if (!target.startsWith(`${root}${path.sep}`) && target !== root) {
      return res.status(400).json({ error: 'Invalid profile storage path' });
    }
    await fs.promises.rm(target, { recursive: true, force: true });
    res.json({ status: 'deleted', closed_sessions: closedSessions });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
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
    if (session.page && typeof session.page.bringToFront === 'function') {
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
    const { selector, text, x, y, element_index, index } = req.body;
    const requestedIndex = Number.isInteger(element_index) ? element_index : Number.isInteger(index) ? index : null;
    const navigation = waitForPotentialNavigation(session.page);
    if (requestedIndex !== null) {
      const target = await interactiveElementCenter(session.page, requestedIndex);
      if (!target.ok) {
        return res.status(422).json({ error: target.error || `No interactive element ${requestedIndex} is visible on the current page.` });
      }
      await session.page.mouse.click(target.x, target.y);
    } else if (x !== undefined && y !== undefined) {
      await session.page.mouse.click(x, y);
    } else if (text) {
      await session.page.getByText(text, { exact: false }).first().click({ timeout: 5000 });
    } else if (selector) {
      await session.page.click(selector, { timeout: 5000 });
    } else {
      return res.status(400).json({ error: 'Provide element_index, selector, text, or x/y coordinates' });
    }
    await settleAfterAction(session.page, navigation);
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
    const target = await focusEditableTarget(session.page, selector);
    if (!target.ok) {
      return res.status(422).json({ error: target.error || 'No editable target found' });
    }
    if (clear || selector) {
      await session.page.keyboard.down(process.platform === 'darwin' ? 'Meta' : 'Control');
      await session.page.keyboard.press('a');
      await session.page.keyboard.up(process.platform === 'darwin' ? 'Meta' : 'Control');
      await session.page.keyboard.press('Backspace');
    }
    await session.page.keyboard.type(text || '', { delay: 30 });
    await settlePage(session.page, 1000);
    res.json({ status: 'ok', target });
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
    const navigation = waitForPotentialNavigation(session.page);
    await session.page.keyboard.press(key);
    await settleAfterAction(session.page, navigation);
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
