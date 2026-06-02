const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const PROFILE_SINGLETON_MARKERS = ['SingletonLock', 'SingletonSocket', 'SingletonCookie'];
const DEFAULT_BROWSER_SESSION_TIMEOUT_MS = 5 * 60 * 1000;

function positiveInt(value, fallback) {
  const parsed = Number.parseInt(String(value || ''), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function browserSessionTimeoutMs(env = process.env) {
  return positiveInt(env.PLAYWRIGHT_SESSION_TIMEOUT_MS, DEFAULT_BROWSER_SESSION_TIMEOUT_MS);
}

function defaultRealBrowserNoSandbox({
  platform = process.platform,
  existsSync = fs.existsSync,
  env = process.env,
} = {}) {
  if (platform !== 'linux') return false;
  return Boolean(
    existsSync('/.dockerenv') ||
    String(env.container || env.CONTAINER || '').trim() ||
    String(env.KUBERNETES_SERVICE_HOST || '').trim()
  );
}

function browserCommandCandidates(browserName, platform = process.platform) {
  const normalized = String(browserName || '').trim().toLowerCase();
  if (normalized === 'edge') {
    return ['microsoft-edge', 'microsoft-edge-stable', 'msedge'];
  }
  if (normalized === 'firefox') {
    return ['firefox'];
  }
  if (normalized === 'chromium') {
    return ['chromium', 'chromium-browser'];
  }
  return ['google-chrome', 'google-chrome-stable', 'chrome'];
}

function firstAvailableCommand(candidates, {
  platform = process.platform,
  existsSync = fs.existsSync,
  spawnSyncFn = spawnSync,
} = {}) {
  const probe = platform === 'win32' ? 'where' : 'which';
  for (const candidate of candidates.filter(Boolean)) {
    if (path.isAbsolute(candidate) && existsSync(candidate)) return candidate;
    const result = spawnSyncFn(probe, [candidate], { encoding: 'utf8' });
    if (result.status === 0) {
      const found = String(result.stdout || '').split(/\r?\n/).map((line) => line.trim()).find(Boolean);
      return found || candidate;
    }
  }
  return '';
}

function browserExecutableNames(browserName) {
  const normalized = String(browserName || '').trim().toLowerCase();
  if (normalized === 'edge') return ['msedge.exe'];
  if (normalized === 'firefox') return ['firefox.exe'];
  if (normalized === 'chromium') return ['chromium.exe'];
  return ['chrome.exe'];
}

function windowsAppPathRegistryKeys(executableName) {
  const exe = String(executableName || '').trim();
  if (!exe) return [];
  return [
    `HKCU\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\${exe}`,
    `HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\${exe}`,
    `HKCU\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths\\${exe}`,
    `HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths\\${exe}`,
  ];
}

function expandWindowsEnvVars(value, env = process.env) {
  return String(value || '').replace(/%([^%]+)%/g, (match, key) => {
    const replacement = env[key] || env[key.toUpperCase()] || env[key.toLowerCase()];
    return replacement || match;
  });
}

function parseWindowsAppPathDefault(stdout, env = process.env) {
  for (const line of String(stdout || '').split(/\r?\n/)) {
    const match = line.match(/\bREG_(?:EXPAND_)?SZ\s+(.+)$/i);
    if (!match) continue;
    const value = match[1].trim().replace(/^"(.*)"$/, '$1');
    if (!value || /^\(value not set\)$/i.test(value)) continue;
    return expandWindowsEnvVars(value, env);
  }
  return '';
}

function windowsAppPathCommand(browserName, {
  existsSync = fs.existsSync,
  spawnSyncFn = spawnSync,
  env = process.env,
} = {}) {
  for (const executableName of browserExecutableNames(browserName)) {
    for (const key of windowsAppPathRegistryKeys(executableName)) {
      const result = spawnSyncFn('reg', ['query', key, '/ve'], { encoding: 'utf8' });
      if (result.status !== 0) continue;
      const command = parseWindowsAppPathDefault(result.stdout, env);
      if (command && existsSync(command)) return command;
    }
  }
  return '';
}

function findBrowserCommand(browserName, {
  platform = process.platform,
  existsSync = fs.existsSync,
  spawnSyncFn = spawnSync,
  env = process.env,
} = {}) {
  const fromPath = firstAvailableCommand(browserCommandCandidates(browserName, platform), {
    platform,
    existsSync,
    spawnSyncFn,
  });
  if (fromPath) return fromPath;
  if (platform !== 'win32') return '';
  return windowsAppPathCommand(browserName, {
    existsSync,
    spawnSyncFn,
    env,
  });
}

function chromiumFallbacks(browserName) {
  const normalized = String(browserName || '').trim().toLowerCase();
  if (normalized === 'chrome' || normalized === 'chromium') return ['edge'];
  return [];
}

function resolveExternalBrowserCommand(browserName, {
  platform = process.platform,
  existsSync = fs.existsSync,
  spawnSyncFn = spawnSync,
  env = process.env,
  allowChromiumFallback = true,
} = {}) {
  const requestedBrowserName = String(browserName || '').trim().toLowerCase() || 'chrome';
  const command = findBrowserCommand(requestedBrowserName, {
    platform,
    existsSync,
    spawnSyncFn,
    env,
  });
  if (command) {
    return {
      browserName: requestedBrowserName,
      requestedBrowserName,
      command,
      fallback: false,
    };
  }

  if (allowChromiumFallback) {
    for (const fallbackBrowserName of chromiumFallbacks(requestedBrowserName)) {
      const fallbackCommand = findBrowserCommand(fallbackBrowserName, {
        platform,
        existsSync,
        spawnSyncFn,
        env,
      });
      if (fallbackCommand) {
        return {
          browserName: fallbackBrowserName,
          requestedBrowserName,
          command: fallbackCommand,
          fallback: true,
        };
      }
    }
  }

  const compatible = chromiumFallbacks(requestedBrowserName);
  const installHint = compatible.length
    ? `Install ${[requestedBrowserName, ...compatible].join(' or ')}`
    : `Install ${requestedBrowserName}`;
  throw new Error(
    `No real ${requestedBrowserName} executable was found. ${installHint} or set PLAYWRIGHT_REAL_BROWSER_EXECUTABLE to the browser path.`,
  );
}

function buildManualLoginBrowserArgs({
  browserName,
  userDataDir,
  initialUrl,
  width,
  height,
  includeSandboxFlags = false,
  remoteDebuggingPort,
}) {
  const normalizedBrowser = String(browserName || '').trim().toLowerCase();
  const url = String(initialUrl || '').trim() || 'about:blank';
  if (normalizedBrowser === 'firefox') {
    return ['-profile', userDataDir, '-new-window', url];
  }

  const displayWidth = positiveInt(width, 1920);
  const displayHeight = positiveInt(height, 1080);
  const args = [
    `--user-data-dir=${userDataDir}`,
    '--no-first-run',
    '--no-default-browser-check',
  ];
  if (includeSandboxFlags) {
    args.push('--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage');
  }
  const debuggingPort = positiveInt(remoteDebuggingPort, 0);
  if (debuggingPort > 0) {
    args.push('--remote-debugging-address=127.0.0.1', `--remote-debugging-port=${debuggingPort}`);
  }
  args.push('--start-maximized', `--window-size=${displayWidth},${displayHeight}`, url);
  return args;
}

function manualLoginExitReason(code, signal) {
  if (signal) return `signal ${signal}`;
  if (code === null || code === undefined) return 'unknown exit';
  return `code ${code}`;
}

function waitForManualLoginBrowserReady(child, {
  timeoutMs = 2000,
  executable = 'browser',
  displayValue = '',
  logPath = '',
} = {}) {
  return new Promise((resolve, reject) => {
    let settled = false;
    let timer = null;

    const cleanup = () => {
      if (timer) clearTimeout(timer);
      child.off('error', onError);
      child.off('exit', onExit);
    };
    const settle = (fn, value) => {
      if (settled) return;
      settled = true;
      cleanup();
      fn(value);
    };
    const context = () => {
      const parts = [];
      if (displayValue) parts.push(`DISPLAY=${displayValue}`);
      if (logPath) parts.push(`log=${logPath}`);
      return parts.length ? ` (${parts.join(', ')})` : '';
    };
    function onError(error) {
      const detail = error && error.message ? error.message : String(error || 'unknown error');
      settle(reject, new Error(`Manual-login browser failed to start: ${detail}${context()}`));
    }
    function onExit(code, signal) {
      settle(
        reject,
        new Error(
          `Manual-login browser exited before it was ready (${executable}, ${manualLoginExitReason(code, signal)})${context()}`,
        ),
      );
    }

    if (child.exitCode !== null || child.signalCode) {
      onExit(child.exitCode, child.signalCode);
      return;
    }

    child.once('error', onError);
    child.once('exit', onExit);
    timer = setTimeout(() => settle(resolve), positiveInt(timeoutMs, 2000));
    if (typeof timer.unref === 'function') timer.unref();
  });
}

function parseProfileLockPid(value) {
  const text = String(value || '').trim();
  if (!text) return null;
  const named = text.match(/(?:^|[^a-z0-9])pid[^0-9]*([1-9][0-9]*)/i);
  const trailing = text.match(/(?:^|[^0-9])([1-9][0-9]*)\s*$/);
  const parsed = Number.parseInt((named || trailing || [])[1] || '', 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
}

function defaultPidAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err && err.code === 'EPERM';
  }
}

function defaultTerminatePid(pid, signal = 'SIGTERM') {
  if (!Number.isInteger(pid) || pid <= 0) {
    throw new Error(`Invalid browser profile owner pid: ${pid}`);
  }
  process.kill(pid, signal);
}

function readProfileLockMarker(userDataDir, marker) {
  const lockPath = path.join(userDataDir, marker);
  try {
    const stat = fs.lstatSync(lockPath);
    let value = '';
    if (stat.isSymbolicLink()) {
      value = fs.readlinkSync(lockPath);
    } else if (stat.isFile() && stat.size <= 4096) {
      value = fs.readFileSync(lockPath, 'utf8');
    }
    return {
      marker,
      path: lockPath,
      value: String(value || '').trim(),
      pid: parseProfileLockPid(value),
    };
  } catch (err) {
    if (err && err.code === 'ENOENT') return null;
    return {
      marker,
      path: lockPath,
      value: '',
      pid: null,
      readError: err && err.message ? err.message : String(err || 'unknown error'),
    };
  }
}

function browserProfileLockInfo(userDataDir, { pidAlive = defaultPidAlive } = {}) {
  const dir = String(userDataDir || '').trim();
  if (!dir) return { locked: false };
  for (const marker of PROFILE_SINGLETON_MARKERS) {
    const lock = readProfileLockMarker(dir, marker);
    if (!lock) continue;
    const active = lock.pid ? Boolean(pidAlive(lock.pid)) : false;
    return {
      locked: true,
      active,
      stale: !active,
      marker: lock.marker,
      path: lock.path,
      pid: lock.pid,
      value: lock.value,
      readError: lock.readError,
    };
  }
  return { locked: false };
}

function clearStaleProfileLockMarkers(userDataDir) {
  const dir = String(userDataDir || '').trim();
  if (!dir) return { cleared: 0, errors: [] };
  let cleared = 0;
  const errors = [];
  for (const marker of PROFILE_SINGLETON_MARKERS) {
    const markerPath = path.join(dir, marker);
    try {
      fs.rmSync(markerPath, { force: true });
      cleared += 1;
    } catch (err) {
      errors.push({
        marker,
        path: markerPath,
        error: err && err.message ? err.message : String(err || 'unknown error'),
      });
    }
  }
  return { cleared, errors };
}

function manualLoginSessionIsActive(session) {
  if (!session || !session.manualLogin || !session.persistentProfile || !session.userDataDir) {
    return false;
  }
  const child = session.externalProcess;
  return Boolean(child && !child.killed && child.exitCode === null && child.signalCode === null);
}

function activeManualLoginSessionForUserDataDir(sessions, userDataDir) {
  const target = path.resolve(userDataDir || '');
  if (!target) return null;
  for (const session of sessions.values()) {
    if (!manualLoginSessionIsActive(session)) continue;
    if (path.resolve(session.userDataDir) !== target) continue;
    return session;
  }
  return null;
}

function activePersistentProfileSessionForUserDataDir(sessions, userDataDir, {
  includeManualLogin = true,
} = {}) {
  const target = path.resolve(userDataDir || '');
  if (!target) return null;
  for (const session of sessions.values()) {
    if (!session || !session.persistentProfile || !session.userDataDir) continue;
    if (!includeManualLogin && session.manualLogin) continue;
    if (path.resolve(session.userDataDir) !== target) continue;
    if (session.manualLogin && !manualLoginSessionIsActive(session)) continue;
    if (!session.manualLogin && !session.context && !session.page) continue;
    return session;
  }
  return null;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, positiveInt(ms, 1)));
}

async function waitForBrowserProfileUnlock(userDataDir, {
  timeoutMs = 5000,
  intervalMs = 100,
  closeActiveOwner = false,
  closeTimeoutMs = 3000,
  lockInfo = browserProfileLockInfo,
  sleep: sleepFn = sleep,
  terminatePid = defaultTerminatePid,
} = {}) {
  const deadline = Date.now() + positiveInt(timeoutMs, 5000);
  let info = lockInfo(userDataDir);
  while (info.locked && Date.now() < deadline) {
    await sleepFn(positiveInt(intervalMs, 100));
    info = lockInfo(userDataDir);
  }
  if (info.locked && info.stale && !info.active) {
    const cleanup = clearStaleProfileLockMarkers(userDataDir);
    if (cleanup.errors.length > 0) {
      return { ...info, staleLockCleanup: cleanup };
    }
    info = lockInfo(userDataDir);
    if (!info.locked) {
      return { ...info, staleLockCleanup: cleanup };
    }
  }
  if (info.locked && closeActiveOwner && info.active && info.pid) {
    try {
      terminatePid(info.pid, 'SIGTERM');
      info = { ...info, closeAttempted: true, closeSignal: 'SIGTERM' };
    } catch (err) {
      return {
        ...info,
        closeAttempted: true,
        closeError: err && err.message ? err.message : String(err || 'unknown error'),
      };
    }
    const closeDeadline = Date.now() + positiveInt(closeTimeoutMs, 3000);
    while (info.locked && Date.now() < closeDeadline) {
      await sleepFn(positiveInt(intervalMs, 100));
      info = lockInfo(userDataDir);
    }
  }
  return info;
}

function savedProfileUsesPersistentContext({ profileId, manualLogin = false, targetKind = '' } = {}) {
  const id = String(profileId || '').trim();
  if (!id) return false;
  if (manualLogin) return true;
  return String(targetKind || '').trim().toLowerCase() !== 'remote_cdp';
}

function makeManualLoginLog(stream, logPath = '') {
  if (!stream) return null;
  const log = { stream, logPath, closed: false };
  if (typeof stream.on === 'function') {
    stream.on('error', (err) => {
      if (!log.closed) {
        const detail = err && err.message ? err.message : String(err || 'unknown error');
        console.warn(`Manual-login log stream error${logPath ? ` (${logPath})` : ''}: ${detail}`);
      }
      log.closed = true;
    });
  }
  return log;
}

function openManualLoginLog(logPath) {
  try {
    return makeManualLoginLog(fs.createWriteStream(logPath, { flags: 'a' }), logPath);
  } catch (_) {
    return null;
  }
}

function writeManualLoginLog(log, line) {
  if (!log || !log.stream || log.closed || log.stream.destroyed || log.stream.writableEnded) return false;
  try {
    log.stream.write(line);
    return true;
  } catch (_) {
    log.closed = true;
    return false;
  }
}

function closeManualLoginLog(log) {
  if (!log || !log.stream || log.closed || log.stream.destroyed || log.stream.writableEnded) return false;
  log.closed = true;
  try {
    log.stream.end();
    return true;
  } catch (_) {
    return false;
  }
}

module.exports = {
  activeManualLoginSessionForUserDataDir,
  activePersistentProfileSessionForUserDataDir,
  browserProfileLockInfo,
  browserSessionTimeoutMs,
  buildManualLoginBrowserArgs,
  browserCommandCandidates,
  clearStaleProfileLockMarkers,
  closeManualLoginLog,
  defaultRealBrowserNoSandbox,
  findBrowserCommand,
  makeManualLoginLog,
  openManualLoginLog,
  parseWindowsAppPathDefault,
  resolveExternalBrowserCommand,
  savedProfileUsesPersistentContext,
  waitForBrowserProfileUnlock,
  waitForManualLoginBrowserReady,
  writeManualLoginLog,
};
