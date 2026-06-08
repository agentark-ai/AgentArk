const express = require('express');
const fs = require('fs');
const path = require('path');
const net = require('net');
const { spawn } = require('child_process');
const { randomUUID } = require('crypto');
const { chromium, firefox } = require('playwright');
const {
  activeManualLoginSessionForUserDataDir,
  activePersistentProfileSessionForUserDataDir,
  browserProfileLockInfo,
  browserSessionTimeoutMs,
  buildManualLoginBrowserArgs,
  clearStaleProfileLockMarkers,
  closeManualLoginLog,
  defaultRealBrowserNoSandbox,
  openManualLoginLog,
  resolveExternalBrowserCommand,
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

function readBooleanEnv(names, fallback = false) {
  const keys = Array.isArray(names) ? names : [names];
  for (const key of keys) {
    const value = String(process.env[key] || '').trim();
    if (!value) continue;
    if (/^(1|true|yes|on)$/i.test(value)) return true;
    if (/^(0|false|no|off)$/i.test(value)) return false;
  }
  return fallback;
}

const PORT = process.env.PORT || 3100;
const HOST = process.env.PLAYWRIGHT_BRIDGE_HOST || process.env.HOST || '127.0.0.1';
const SESSION_TIMEOUT_MS = browserSessionTimeoutMs(process.env);
const HEADLESS = /^(1|true|yes|on)$/i.test(process.env.PLAYWRIGHT_HEADLESS || '');
const LIVE_VIEW_PORT = Number.parseInt(process.env.PLAYWRIGHT_LIVE_VIEW_PORT || '6080', 10) || 6080;
const LIVE_VIEW_PATH = process.env.PLAYWRIGHT_LIVE_VIEW_PATH || '/vnc.html?autoconnect=1&resize=scale&path=websockify';
const LIVE_VIEW_ENABLED = !HEADLESS && Boolean(process.env.DISPLAY);
const AGENTARK_DATA_ROOT = resolveAgentArkDataRoot();
const PROFILE_ROOT = process.env.PLAYWRIGHT_PROFILE_ROOT || path.join(AGENTARK_DATA_ROOT, 'browser-profiles');
const DOWNLOAD_ROOT = process.env.PLAYWRIGHT_DOWNLOAD_ROOT || path.join(AGENTARK_DATA_ROOT, 'browser-downloads');
const BROWSER_WIDTH = readPositiveIntEnv(['PLAYWRIGHT_BROWSER_WIDTH', 'PLAYWRIGHT_VIEWPORT_WIDTH'], 1920);
const BROWSER_HEIGHT = readPositiveIntEnv(['PLAYWRIGHT_BROWSER_HEIGHT', 'PLAYWRIGHT_VIEWPORT_HEIGHT'], 1080);
const CLOSE_ACTIVE_PROFILE_LOCK = !/^(0|false|no|off)$/i.test(
  String(process.env.PLAYWRIGHT_CLOSE_ACTIVE_PROFILE_LOCK || 'true').trim(),
);
const REAL_BROWSER_SANDBOX_FLAGS = readBooleanEnv(
  ['PLAYWRIGHT_REAL_BROWSER_NO_SANDBOX', 'AGENTARK_REAL_BROWSER_NO_SANDBOX'],
  defaultRealBrowserNoSandbox(),
);
const HOST_PROFILE_SANDBOX_FLAGS = readBooleanEnv(['PLAYWRIGHT_HOST_PROFILE_NO_SANDBOX'], false);

// Active browser sessions: id -> { context, page, mode, claimed, claimedAt, lastActivity, cleanupTimer, diagnostics, downloads }
const sessions = new Map();

let browser = null;

function resolveAgentArkDataRoot() {
  const configured = String(process.env.AGENTARK_DATA || process.env.AGENTARK_DATA_DIR || '').trim();
  if (configured) return path.resolve(configured);

  let cursor = process.cwd();
  for (let i = 0; i < 6; i++) {
    if (
      fs.existsSync(path.join(cursor, 'Cargo.toml')) &&
      fs.existsSync(path.join(cursor, 'src')) &&
      fs.existsSync(path.join(cursor, 'bridges'))
    ) {
      return path.join(cursor, '.agentark', 'data');
    }
    const parent = path.dirname(cursor);
    if (!parent || parent === cursor) break;
    cursor = parent;
  }

  return path.join(process.cwd(), '.agentark', 'data');
}

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
  const hostProfile = targetKind === 'host';
  const launchOptions = {
    headless: HEADLESS,
  };
  if (browserName !== 'firefox') {
    const args = [];
    if (!hostProfile || HOST_PROFILE_SANDBOX_FLAGS) {
      args.push('--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage');
    }
    if (!HEADLESS) {
      args.push('--start-maximized', `--window-size=${BROWSER_WIDTH},${BROWSER_HEIGHT}`);
    }
    launchOptions.args = args;
    if (hostProfile) {
      launchOptions.ignoreDefaultArgs = ['--enable-automation'];
    }
  }
  const executablePath = process.env.PLAYWRIGHT_EXECUTABLE_PATH;
  if (executablePath) {
    if (!fs.existsSync(executablePath)) {
      throw new Error(`Configured PLAYWRIGHT_EXECUTABLE_PATH does not exist: ${executablePath}`);
    }
    launchOptions.executablePath = executablePath;
  } else if (hostProfile) {
    const resolved = resolveRealBrowser(browserName);
    launchOptions.executablePath = resolved.command;
    if (resolved.fallback) {
      console.warn(
        `Real browser profile requested ${resolved.requestedBrowserName}, using installed ${resolved.browserName} at ${resolved.command}`,
      );
    }
  }
  const channel = launchOptions.executablePath || browserName === 'firefox'
    ? ''
    : browserChannelFor(browserName, targetKind);
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
    download_dir: session.downloadDir || null,
    downloads: await sessionDownloadArtifacts(session),
  };
}

function safeProfileId(raw) {
  const value = String(raw || '').trim();
  if (!value) return '';
  const safe = value.replace(/[^A-Za-z0-9_.-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 120);
  return safe || '';
}

function safeDownloadFilename(raw) {
  const value = String(raw || 'download').trim() || 'download';
  const base = path.basename(value).replace(/[<>:"/\\|?*\x00-\x1F]+/g, '_').slice(0, 160);
  return base || 'download';
}

function browserDownloadDir(sessionId, profileId) {
  const safeProfile = safeProfileId(profileId);
  if (safeProfile) return path.join(DOWNLOAD_ROOT, 'profiles', safeProfile);
  return path.join(DOWNLOAD_ROOT, 'sessions', safeProfileId(sessionId) || sessionId);
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

function resolveRealBrowser(browserName) {
  const configured = String(
    process.env.PLAYWRIGHT_REAL_BROWSER_EXECUTABLE ||
    process.env.AGENTARK_REAL_BROWSER_EXECUTABLE ||
    ''
  ).trim();
  if (configured) {
    if (!fs.existsSync(configured)) {
      throw new Error(`Configured real browser executable does not exist: ${configured}`);
    }
    return {
      browserName,
      requestedBrowserName: browserName,
      command: configured,
      fallback: false,
    };
  }
  return resolveExternalBrowserCommand(browserName);
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

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, readPositiveIntEnv([], ms || 1)));
}

async function reserveLocalPort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      const port = address && typeof address === 'object' ? address.port : 0;
      server.close(() => {
        if (port > 0) resolve(port);
        else reject(new Error('Unable to reserve local browser debugging port'));
      });
    });
  });
}

async function connectToExternalBrowserOverCdp(port, { timeoutMs = 6000, intervalMs = 150 } = {}) {
  const endpoint = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + readPositiveIntEnv([], timeoutMs);
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      return await chromium.connectOverCDP(endpoint);
    } catch (err) {
      lastError = err;
      await sleep(intervalMs);
    }
  }
  const detail = lastError && lastError.message ? lastError.message : String(lastError || 'unknown error');
  throw new Error(`Unable to attach to real browser over CDP at ${endpoint}: ${detail}`);
}

async function launchExternalBrowserProfile(browserName, userDataDir, initialUrl, {
  remoteDebuggingPort,
} = {}) {
  const resolved = resolveRealBrowser(browserName);
  const executable = resolved.command;
  const launchBrowserName = resolved.browserName;
  if (executable && /[\\/]/.test(executable) && !fs.existsSync(executable)) {
    throw new Error(`Browser executable not found for ${launchBrowserName}: ${executable}`);
  }
  if (launchBrowserName !== 'firefox') {
    const lockInfo = browserProfileLockInfo(userDataDir);
    if (lockInfo.locked) throw browserProfileLockedError(lockInfo, userDataDir);
  }
  const args = buildManualLoginBrowserArgs({
    browserName: launchBrowserName,
    userDataDir,
    initialUrl,
    width: BROWSER_WIDTH,
    height: BROWSER_HEIGHT,
    remoteDebuggingPort,
    includeSandboxFlags: REAL_BROWSER_SANDBOX_FLAGS,
  });
  const displayValue = process.env.DISPLAY && process.env.DISPLAY.trim()
    ? process.env.DISPLAY
    : ':99';
  const launchEnv = { ...process.env, DISPLAY: displayValue };
  const logPath = '/tmp/agentark-manual-login.log';
  const log = openManualLoginLog(logPath);
  const fallbackNote = resolved.fallback
    ? ` requested=${resolved.requestedBrowserName} resolved=${resolved.browserName}`
    : '';
  writeManualLoginLog(log, `\n[${new Date().toISOString()}] launching ${executable} (${launchBrowserName})${fallbackNote} DISPLAY=${displayValue} url=${initialUrl}\n`);
  if (resolved.fallback) {
    console.warn(
      `Real browser profile requested ${resolved.requestedBrowserName}, using installed ${resolved.browserName} at ${resolved.command}`,
    );
  }
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
  return {
    process: child,
    executable,
    initialUrl,
    browserName: launchBrowserName,
    requestedBrowserName: resolved.requestedBrowserName,
  };
}

async function prepareBrowserDownloadDirectory(downloadDir) {
  if (!downloadDir) return;
  await fs.promises.mkdir(downloadDir, { recursive: true });
}

async function prepareChromiumDownloadPreferences(userDataDir, downloadDir) {
  if (!userDataDir || !downloadDir) return;
  const defaultDir = path.join(userDataDir, 'Default');
  const preferencesPath = path.join(defaultDir, 'Preferences');
  await fs.promises.mkdir(defaultDir, { recursive: true });
  let preferences = {};
  try {
    preferences = JSON.parse(await fs.promises.readFile(preferencesPath, 'utf8'));
  } catch (_) {
    preferences = {};
  }
  preferences.download = {
    ...(preferences.download || {}),
    default_directory: downloadDir,
    directory_upgrade: true,
    prompt_for_download: false,
  };
  preferences.savefile = {
    ...(preferences.savefile || {}),
    default_directory: downloadDir,
  };
  await fs.promises.writeFile(preferencesPath, JSON.stringify(preferences, null, 2));
}

async function prepareFirefoxDownloadPreferences(userDataDir, downloadDir) {
  if (!userDataDir || !downloadDir) return;
  await fs.promises.mkdir(userDataDir, { recursive: true });
  const escapedDir = downloadDir.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  const userJsPath = path.join(userDataDir, 'user.js');
  const block = [
    'user_pref("browser.download.folderList", 2);',
    `user_pref("browser.download.dir", "${escapedDir}");`,
    'user_pref("browser.download.useDownloadDir", true);',
  ].join('\n');
  let existing = '';
  try {
    existing = await fs.promises.readFile(userJsPath, 'utf8');
  } catch (_) {}
  const markerStart = '// AgentArk managed download preferences start';
  const markerEnd = '// AgentArk managed download preferences end';
  const managedBlock = `${markerStart}\n${block}\n${markerEnd}`;
  const next = existing.includes(markerStart) && existing.includes(markerEnd)
    ? existing.replace(new RegExp(`${markerStart}[\\s\\S]*?${markerEnd}`), managedBlock)
    : `${existing.trim() ? `${existing.trim()}\n\n` : ''}${managedBlock}\n`;
  await fs.promises.writeFile(userJsPath, next);
}

async function prepareProfileDownloadDefaults(browserName, userDataDir, downloadDir) {
  await prepareBrowserDownloadDirectory(downloadDir);
  const normalized = normalizeBrowserName(browserName);
  if (normalized === 'firefox') {
    await prepareFirefoxDownloadPreferences(userDataDir, downloadDir);
  } else {
    await prepareChromiumDownloadPreferences(userDataDir, downloadDir);
  }
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
    const labelFor = (el) => {
      const tag = el.tagName.toLowerCase();
      const role = String(el.getAttribute('role') || '').toLowerCase();
      const editable = el.isContentEditable || role === 'textbox' || tag === 'input' || tag === 'textarea';
      const parts = editable
        ? [
            el.getAttribute('aria-label'),
            el.getAttribute('placeholder'),
            el.getAttribute('name'),
            el.id,
            el.innerText,
            el.value,
            el.getAttribute('href'),
            tag,
          ]
        : [
            el.innerText,
            el.value,
            el.getAttribute('aria-label'),
            el.getAttribute('placeholder'),
            el.getAttribute('name'),
            el.id,
            el.getAttribute('href'),
            tag,
          ];
      return (parts.find((part) => String(part || '').trim()) || '').trim().substring(0, 80);
    };
    const interactiveSelectors = 'a, button, input, select, textarea, [role="button"], [role="link"], [role="textbox"], [contenteditable], [onclick]';
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
    const interactiveSelectors = 'a, button, input, select, textarea, [role="button"], [role="link"], [role="textbox"], [contenteditable], [onclick]';
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
      const role = String(el.getAttribute('role') || '').toLowerCase();
      const editable = el.isContentEditable || role === 'textbox' || tag === 'input' || tag === 'textarea';
      const type = el.getAttribute('type') || (el.isContentEditable ? 'contenteditable' : role);
      const labelParts = editable
        ? [el.getAttribute('aria-label'), el.getAttribute('placeholder'), el.getAttribute('name'), el.id, el.innerText, el.value]
        : [el.innerText, el.value, el.getAttribute('aria-label'), el.getAttribute('placeholder')];
      const text = (labelParts.find((part) => String(part || '').trim()) || '').trim().substring(0, 80);
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

async function focusEditableTarget(page, selector, elementIndex) {
  return page.evaluate(({ selector, elementIndex }) => {
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
      return el.isContentEditable || String(el.getAttribute('role') || '').toLowerCase() === 'textbox';
    };
    const interactiveSelectors = 'a, button, input, select, textarea, [role="button"], [role="link"], [role="textbox"], [contenteditable], [onclick]';
    const elementByVisibleIndex = (index) => {
      if (!Number.isInteger(index)) return null;
      const els = document.querySelectorAll(interactiveSelectors);
      let visibleIndex = 0;
      for (let i = 0; i < Math.min(els.length, 50); i++) {
        const el = els[i];
        if (!visible(el)) continue;
        if (visibleIndex === index) return el;
        visibleIndex += 1;
      }
      return null;
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
    let candidates = [];
    if (Number.isInteger(elementIndex)) {
      const indexed = elementByVisibleIndex(elementIndex);
      if (!indexed) {
        return { ok: false, error: `No interactive element ${elementIndex} is visible on the current page.` };
      }
      if (!editable(indexed)) {
        return { ok: false, error: `Interactive element ${elementIndex} is not editable.` };
      }
      candidates = [{ el: indexed, selected: true }];
    } else {
      candidates = collect(selector, true);
    }
    if (candidates.length === 0) {
      candidates = Array.from(document.querySelectorAll('input, textarea, [role="textbox"], [contenteditable]'))
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
  }, {
    selector: selector || null,
    elementIndex: Number.isInteger(elementIndex) ? elementIndex : null,
  });
}

function touchSession(session) {
  session.lastActivity = Date.now();
  if (session.cleanupTimer) clearTimeout(session.cleanupTimer);
  session.cleanupTimer = setTimeout(() => destroySession(session.id), SESSION_TIMEOUT_MS);
}

function terminateExternalBrowserProcess(child) {
  if (!child || child.killed) return;
  const pid = Number.parseInt(String(child.pid || ''), 10);
  if (process.platform !== 'win32' && Number.isInteger(pid) && pid > 0) {
    try {
      process.kill(-pid, 'SIGTERM');
      return;
    } catch (err) {
      if (err && err.code === 'ESRCH') return;
    }
  }
  try { child.kill(); } catch (_) {}
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

async function sessionDownloadArtifacts(session) {
  const seen = new Map();
  for (const download of session.downloads || []) {
    if (download && download.path) seen.set(download.path, { ...download });
  }

  const downloadDir = session.downloadDir;
  if (downloadDir) {
    try {
      const entries = await fs.promises.readdir(downloadDir, { withFileTypes: true });
      for (const entry of entries) {
        if (!entry.isFile()) continue;
        const filePath = path.join(downloadDir, entry.name);
        if (seen.has(filePath)) continue;
        const stat = await fs.promises.stat(filePath).catch(() => null);
        seen.set(filePath, {
          id: `file:${entry.name}`,
          filename: entry.name,
          path: filePath,
          bytes: stat ? stat.size : 0,
          url: '',
          downloaded_at: stat ? stat.mtime.toISOString() : '',
          status: 'completed',
        });
      }
    } catch (_) {}
  }

  return Array.from(seen.values())
    .sort((left, right) => String(right.downloaded_at || '').localeCompare(String(left.downloaded_at || '')))
    .slice(0, 80);
}

async function recordDownload(session, download) {
  if (!session || !download) return;
  if (!Array.isArray(session.downloads)) session.downloads = [];
  const id = randomUUID();
  const filename = safeDownloadFilename(download.suggestedFilename && download.suggestedFilename());
  const sessionDir = session.downloadDir || browserDownloadDir(session.id, session.profileId);
  const filePath = path.join(sessionDir, `${Date.now()}-${id}-${filename}`);
  const entry = {
    id,
    filename,
    path: filePath,
    bytes: 0,
    url: '',
    downloaded_at: new Date().toISOString(),
    status: 'pending',
  };
  session.downloads.push(entry);
  if (session.downloads.length > 40) {
    session.downloads.splice(0, session.downloads.length - 40);
  }
  try {
    entry.url = typeof download.url === 'function' ? String(download.url() || '') : '';
    await fs.promises.mkdir(sessionDir, { recursive: true });
    await download.saveAs(filePath);
    const stat = await fs.promises.stat(filePath).catch(() => null);
    entry.bytes = stat ? stat.size : 0;
    entry.status = 'completed';
  } catch (err) {
    entry.status = 'failed';
    recordDiagnostic(session, {
      kind: 'download',
      severity: 'error',
      message: err && err.message ? err.message : String(err || 'download failed'),
      url: entry.url,
    });
  }
}

async function destroySession(id) {
  const session = sessions.get(id);
  if (!session) return;
  if (session.cleanupTimer) clearTimeout(session.cleanupTimer);
  const userDataDir = session.persistentProfile ? session.userDataDir : '';
  await saveSessionProfileState(session);
  if (session.cdpBrowser) {
    try { await session.cdpBrowser.close(); } catch (_) {}
  }
  if (session.externalProcess && !session.externalProcess.killed) {
    terminateExternalBrowserProcess(session.externalProcess);
  }
  if (session.context && !session.cdpBrowser) {
    try { await session.context.close(); } catch (_) {}
  }
  if (userDataDir) {
    await cleanupPersistentProfileLocks(userDataDir, id);
  }
  sessions.delete(id);
  console.log(`Session ${id} destroyed (${sessions.size} remaining)`);
}

async function cleanupPersistentProfileLocks(userDataDir, sessionId) {
  const timeoutMs = readPositiveIntEnv(
    ['PLAYWRIGHT_PROFILE_DESTROY_UNLOCK_TIMEOUT_MS', 'PLAYWRIGHT_PROFILE_CLOSE_TIMEOUT_MS'],
    3000,
  );
  const intervalMs = readPositiveIntEnv(['PLAYWRIGHT_PROFILE_UNLOCK_POLL_MS'], 100);
  let info;
  try {
    info = await waitForBrowserProfileUnlock(userDataDir, {
      timeoutMs,
      intervalMs,
      closeActiveOwner: false,
    });
  } catch (err) {
    console.warn(`Failed to inspect browser profile locks after closing session ${sessionId}: ${err.message}`);
    return;
  }
  if (info && info.staleLockCleanup && info.staleLockCleanup.errors?.length) {
    console.warn(
      `Failed to clear stale browser singleton locks after closing session ${sessionId}: ${JSON.stringify(info.staleLockCleanup.errors)}`,
    );
    return;
  }
  if (info && info.locked && info.stale && !info.active) {
    const cleanup = clearStaleProfileLockMarkers(userDataDir);
    if (cleanup.errors.length) {
      console.warn(
        `Failed to clear stale browser singleton locks after closing session ${sessionId}: ${JSON.stringify(cleanup.errors)}`,
      );
    } else if (cleanup.cleared > 0) {
      console.log(`Cleared ${cleanup.cleared} stale browser singleton lock markers for session ${sessionId}`);
    }
  } else if (info && info.staleLockCleanup && info.staleLockCleanup.cleared > 0) {
    console.log(
      `Cleared ${info.staleLockCleanup.cleared} stale browser singleton lock markers for session ${sessionId}`,
    );
  }
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
    const id = randomUUID();
    const downloadDir = browserDownloadDir(id, profileId);
    const contextOptions = {
      viewport: null,
      screen: { width: BROWSER_WIDTH, height: BROWSER_HEIGHT },
      acceptDownloads: true,
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
    let cdpBrowser = null;
    let externalProcess = null;
    let externalTitle = '';
    let externalUrl = '';
    if (persistentProfile && manualLogin) {
      await fs.promises.mkdir(userDataDir, { recursive: true });
      await prepareProfileDownloadDefaults(browserName, userDataDir, downloadDir);
      const activeSession = activeManualLoginSessionForUserDataDir(sessions, userDataDir);
      if (activeSession) {
        touchSession(activeSession);
        return res.json(await sessionStatePayload(activeSession));
      }
      const launched = await launchExternalBrowserProfile(browserName, userDataDir, externalBrowserInitialUrl(profile, req));
      externalProcess = launched.process;
      externalTitle = `${launched.browserName || browserName} manual login`;
      externalUrl = launched.initialUrl;
    } else if (persistentProfile) {
      await fs.promises.mkdir(userDataDir, { recursive: true });
      await prepareProfileDownloadDefaults(browserName, userDataDir, downloadDir);
      let activeSession = activePersistentProfileSessionForUserDataDir(sessions, userDataDir);
      while (activeSession) {
        await destroySession(activeSession.id);
        activeSession = activePersistentProfileSessionForUserDataDir(sessions, userDataDir);
      }
      const lockInfo = await waitForBrowserProfileUnlock(userDataDir, {
        timeoutMs: readPositiveIntEnv(['PLAYWRIGHT_PROFILE_UNLOCK_TIMEOUT_MS'], 5000),
        closeActiveOwner: CLOSE_ACTIVE_PROFILE_LOCK,
        closeTimeoutMs: readPositiveIntEnv(['PLAYWRIGHT_PROFILE_CLOSE_TIMEOUT_MS'], 3000),
        intervalMs: readPositiveIntEnv(['PLAYWRIGHT_PROFILE_UNLOCK_POLL_MS'], 100),
      });
      if (lockInfo.closeAttempted) {
        console.warn(
          `Closed active browser profile owner for ${profileId || userDataDir} before automation launch`,
        );
      }
      if (lockInfo.locked) throw browserProfileLockedError(lockInfo, userDataDir);
      if (targetKind === 'host' && browserName !== 'firefox') {
        const debuggingPort = await reserveLocalPort();
        const launched = await launchExternalBrowserProfile(browserName, userDataDir, 'about:blank', {
          remoteDebuggingPort: debuggingPort,
        });
        externalProcess = launched.process;
        externalTitle = `${launched.browserName || browserName} real browser`;
        externalUrl = launched.initialUrl;
        try {
          cdpBrowser = await connectToExternalBrowserOverCdp(debuggingPort, {
            timeoutMs: readPositiveIntEnv(['PLAYWRIGHT_CDP_ATTACH_TIMEOUT_MS'], 6000),
            intervalMs: readPositiveIntEnv(['PLAYWRIGHT_CDP_ATTACH_POLL_MS'], 150),
          });
          context = cdpBrowser.contexts()[0];
          if (!context) throw new Error('Real browser did not expose a default context');
          page = context.pages()[0] || await context.newPage();
        } catch (err) {
          terminateExternalBrowserProcess(externalProcess);
          externalProcess = null;
          throw err;
        }
      } else {
        const browserType = browserTypeFor(browserName);
        context = await browserType.launchPersistentContext(userDataDir, {
          ...buildLaunchOptions(browserName, targetKind),
          ...contextOptions,
        });
        page = context.pages()[0] || await context.newPage();
      }
    } else {
      await prepareBrowserDownloadDirectory(downloadDir);
      const b = await ensureBrowser();
      context = await b.newContext(contextOptions);
      page = await context.newPage();
    }
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
      cdpBrowser,
      externalTitle,
      externalUrl,
      claimed: false,
      claimedAt: null,
      lastActivity: Date.now(),
      cleanupTimer: null,
      diagnostics: [],
      downloads: [],
      downloadDir,
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
      page.on('download', (download) => {
        recordDownload(session, download).catch((err) => {
          recordDiagnostic(session, {
            kind: 'download',
            severity: 'error',
            message: err && err.message ? err.message : String(err || 'download failed'),
            url: page.url(),
          });
        });
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
    const { selector, text, clear, element_index, index } = req.body;
    const requestedIndex = Number.isInteger(element_index) ? element_index : Number.isInteger(index) ? index : null;
    const target = await focusEditableTarget(session.page, selector, requestedIndex);
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
    res.json({
      ...snapshot,
      diagnostics: session.diagnostics || [],
      download_dir: session.downloadDir || null,
      downloads: await sessionDownloadArtifacts(session),
    });
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
