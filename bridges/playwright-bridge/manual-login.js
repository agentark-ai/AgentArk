const fs = require('node:fs');
const path = require('node:path');

function positiveInt(value, fallback) {
  const parsed = Number.parseInt(String(value || ''), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function buildManualLoginBrowserArgs({ browserName, userDataDir, initialUrl, width, height }) {
  const normalizedBrowser = String(browserName || '').trim().toLowerCase();
  const url = String(initialUrl || '').trim() || 'about:blank';
  if (normalizedBrowser === 'firefox') {
    return ['-profile', userDataDir, '-new-window', url];
  }

  const displayWidth = positiveInt(width, 1920);
  const displayHeight = positiveInt(height, 1080);
  return [
    `--user-data-dir=${userDataDir}`,
    '--no-first-run',
    '--no-default-browser-check',
    '--no-sandbox',
    '--disable-setuid-sandbox',
    '--disable-dev-shm-usage',
    '--start-maximized',
    `--window-size=${displayWidth},${displayHeight}`,
    url,
  ];
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
  for (const marker of ['SingletonLock', 'SingletonSocket', 'SingletonCookie']) {
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

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, positiveInt(ms, 1)));
}

async function waitForBrowserProfileUnlock(userDataDir, {
  timeoutMs = 5000,
  intervalMs = 100,
  lockInfo = browserProfileLockInfo,
  sleep: sleepFn = sleep,
} = {}) {
  const deadline = Date.now() + positiveInt(timeoutMs, 5000);
  let info = lockInfo(userDataDir);
  while (info.locked && Date.now() < deadline) {
    await sleepFn(positiveInt(intervalMs, 100));
    info = lockInfo(userDataDir);
  }
  return info;
}

function savedProfileUsesPersistentContext({ profileId, manualLogin = false, targetKind = '' } = {}) {
  const id = String(profileId || '').trim();
  if (!id) return false;
  if (manualLogin) return true;
  return String(targetKind || '').trim().toLowerCase() === 'host';
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
  browserProfileLockInfo,
  buildManualLoginBrowserArgs,
  closeManualLoginLog,
  makeManualLoginLog,
  openManualLoginLog,
  savedProfileUsesPersistentContext,
  waitForBrowserProfileUnlock,
  waitForManualLoginBrowserReady,
  writeManualLoginLog,
};
