const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  browserProfileLockInfo,
  buildManualLoginBrowserArgs,
  resolveExternalBrowserCommand,
  closeManualLoginLog,
  activeManualLoginSessionForUserDataDir,
  activePersistentProfileSessionForUserDataDir,
  findBrowserCommand,
  makeManualLoginLog,
  browserSessionTimeoutMs,
  parseWindowsAppPathDefault,
  defaultRealBrowserNoSandbox,
  savedProfileUsesPersistentContext,
  waitForBrowserProfileUnlock,
  waitForManualLoginBrowserReady,
  writeManualLoginLog,
} = require('./manual-login');

class FakeChild extends EventEmitter {
  constructor() {
    super();
    this.exitCode = null;
    this.signalCode = null;
  }

  kill() {
    this.killed = true;
  }
}

class FakeWritable extends EventEmitter {
  constructor() {
    super();
    this.destroyed = false;
    this.writableEnded = false;
    this.writes = [];
  }

  write(chunk) {
    if (this.writableEnded) throw new Error('write after end');
    this.writes.push(String(chunk));
  }

  end() {
    this.writableEnded = true;
  }
}

test('chromium manual login keeps real profile launch flags minimal by default', () => {
  const args = buildManualLoginBrowserArgs({
    browserName: 'chromium',
    userDataDir: '/profiles/alex',
    initialUrl: 'about:blank',
    width: 1600,
    height: 900,
  });

  assert(!args.includes('--no-sandbox'));
  assert(!args.includes('--disable-setuid-sandbox'));
  assert(!args.includes('--disable-dev-shm-usage'));
  assert(args.includes('--start-maximized'));
  assert(args.includes('--window-size=1600,900'));
  assert.equal(args[0], '--user-data-dir=/profiles/alex');
  assert.equal(args.at(-1), 'about:blank');
});

test('chromium manual login can opt into sandbox-disabling flags', () => {
  const args = buildManualLoginBrowserArgs({
    browserName: 'chromium',
    userDataDir: '/profiles/alex',
    initialUrl: 'about:blank',
    width: 1600,
    height: 900,
    includeSandboxFlags: true,
  });

  assert(args.includes('--no-sandbox'));
  assert(args.includes('--disable-setuid-sandbox'));
  assert(args.includes('--disable-dev-shm-usage'));
});

test('chromium manual login can expose a local CDP port for real-browser attach', () => {
  const args = buildManualLoginBrowserArgs({
    browserName: 'chrome',
    userDataDir: '/profiles/alex',
    initialUrl: 'about:blank',
    width: 1600,
    height: 900,
    remoteDebuggingPort: 9223,
  });

  assert(args.includes('--remote-debugging-address=127.0.0.1'));
  assert(args.includes('--remote-debugging-port=9223'));
});

test('firefox manual login keeps native profile launch shape', () => {
  const args = buildManualLoginBrowserArgs({
    browserName: 'firefox',
    userDataDir: '/profiles/alex',
    initialUrl: 'https://example.com',
    width: 1600,
    height: 900,
  });

  assert.deepEqual(args, ['-profile', '/profiles/alex', '-new-window', 'https://example.com']);
});

test('real chrome profile falls back to installed edge when chrome is missing', () => {
  const edgePath = 'D:\\Browsers\\Edge\\msedge.exe';
  const result = resolveExternalBrowserCommand('chrome', {
    platform: 'win32',
    existsSync: (candidate) => candidate === edgePath,
    spawnSyncFn: (command, args) => {
      if (command === 'where') return { status: 1, stdout: '' };
      if (command !== 'reg') return { status: 1, stdout: '' };
      const key = String(args[1] || '');
      if (key.endsWith('\\msedge.exe')) {
        return {
          status: 0,
          stdout: `HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\msedge.exe\r\n    (Default)    REG_SZ    ${edgePath}\r\n`,
        };
      }
      return { status: 1, stdout: '' };
    },
  });

  assert.equal(result.requestedBrowserName, 'chrome');
  assert.equal(result.browserName, 'edge');
  assert.equal(result.fallback, true);
  assert.match(result.command, /msedge\.exe$/);
});

test('real chrome profile prefers chrome when chrome is installed', () => {
  const chromePath = 'D:\\Browsers\\Chrome\\chrome.exe';
  const result = resolveExternalBrowserCommand('chrome', {
    platform: 'win32',
    existsSync: () => false,
    spawnSyncFn: (command, args) => {
      if (command !== 'where') return { status: 1, stdout: '' };
      if (String(args[0] || '') === 'chrome') {
        return { status: 0, stdout: `${chromePath}\r\n` };
      }
      return { status: 1, stdout: '' };
    },
  });

  assert.equal(result.requestedBrowserName, 'chrome');
  assert.equal(result.browserName, 'chrome');
  assert.equal(result.fallback, false);
  assert.match(result.command, /chrome\.exe$/);
});

test('real firefox profile does not fall back to edge', () => {
  assert.throws(
    () => resolveExternalBrowserCommand('firefox', {
      platform: 'win32',
      existsSync: () => false,
      spawnSyncFn: (command, args) => {
        if (command === 'where') return { status: 1, stdout: '' };
        if (command === 'reg' && String(args[1] || '').endsWith('\\msedge.exe')) {
          return {
            status: 0,
            stdout: 'HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\msedge.exe\r\n    (Default)    REG_SZ    D:\\Browsers\\Edge\\msedge.exe\r\n',
          };
        }
        return { status: 1, stdout: '' };
      },
    }),
    /No real firefox executable was found/,
  );
});

test('docker and linux browser discovery only use PATH lookup', () => {
  const calls = [];
  const command = findBrowserCommand('chrome', {
    platform: 'linux',
    existsSync: () => false,
    spawnSyncFn: (probe, args) => {
      calls.push([probe, args[0]]);
      return { status: 1, stdout: '' };
    },
  });

  assert.equal(command, '');
  assert(calls.every(([probe]) => probe === 'which'));
});

test('real browser launch disables sandbox by default inside Linux containers', () => {
  assert.equal(defaultRealBrowserNoSandbox({
    platform: 'linux',
    existsSync: (candidate) => candidate === '/.dockerenv',
    env: {},
  }), true);
  assert.equal(defaultRealBrowserNoSandbox({
    platform: 'linux',
    existsSync: () => false,
    env: { container: 'docker' },
  }), true);
  assert.equal(defaultRealBrowserNoSandbox({
    platform: 'linux',
    existsSync: () => false,
    env: {},
  }), false);
  assert.equal(defaultRealBrowserNoSandbox({
    platform: 'win32',
    existsSync: () => true,
    env: { container: 'docker' },
  }), false);
});

test('windows app path parser ignores unset registry values and expands variables', () => {
  const unset = parseWindowsAppPathDefault(
    'HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\chrome.exe\r\n    (Default)    REG_SZ    (value not set)\r\n',
  );
  const expanded = parseWindowsAppPathDefault(
    'HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\msedge.exe\r\n    (Default)    REG_EXPAND_SZ    %LOCALAPPDATA%\\Edge\\msedge.exe\r\n',
    { LOCALAPPDATA: 'D:\\Users\\alex\\AppData\\Local' },
  );

  assert.equal(unset, '');
  assert.equal(expanded, 'D:\\Users\\alex\\AppData\\Local\\Edge\\msedge.exe');
});

test('manual login readiness rejects a browser that exits during startup', async () => {
  const child = new FakeChild();
  const ready = waitForManualLoginBrowserReady(child, {
    timeoutMs: 50,
    executable: 'node',
    displayValue: ':99',
    logPath: '/tmp/agentark-manual-login.log',
  });

  child.exitCode = 43;
  child.emit('exit', 43, null);

  await assert.rejects(ready, /exited before it was ready.*code 43/);
});

test('manual login readiness rejects spawn failures', async () => {
  const child = new FakeChild();
  const ready = waitForManualLoginBrowserReady(child, {
    timeoutMs: 50,
    executable: 'missing-browser',
    displayValue: ':99',
    logPath: '/tmp/agentark-manual-login.log',
  });

  child.emit('error', new Error('spawn ENOENT'));

  await assert.rejects(ready, /failed to start.*spawn ENOENT/);
});

test('browser profile lock info reports active Chromium singleton lock', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'agentark-profile-lock-'));
  try {
    fs.writeFileSync(path.join(dir, 'SingletonLock'), '6aeb5c564358-1036');

    const info = browserProfileLockInfo(dir, {
      pidAlive: (pid) => pid === 1036,
    });

    assert.equal(info.locked, true);
    assert.equal(info.pid, 1036);
    assert.equal(info.marker, 'SingletonLock');
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('sandbox saved profile automation reuses the manual login user-data dir', () => {
  assert.equal(savedProfileUsesPersistentContext({
    profileId: 'alex',
    manualLogin: false,
    targetKind: 'sandbox',
  }), true);
  assert.equal(savedProfileUsesPersistentContext({
    profileId: 'alex',
    manualLogin: true,
    targetKind: 'sandbox',
  }), true);
  assert.equal(savedProfileUsesPersistentContext({
    profileId: 'alex',
    manualLogin: false,
    targetKind: 'host',
  }), true);
});

test('browser session inactivity timeout defaults to five minutes and remains configurable', () => {
  assert.equal(browserSessionTimeoutMs({}), 5 * 60 * 1000);
  assert.equal(browserSessionTimeoutMs({ PLAYWRIGHT_SESSION_TIMEOUT_MS: '120000' }), 120000);
  assert.equal(browserSessionTimeoutMs({ PLAYWRIGHT_SESSION_TIMEOUT_MS: '0' }), 5 * 60 * 1000);
});

test('active manual login session is found by profile storage path', () => {
  const active = new FakeChild();
  const inactive = new FakeChild();
  inactive.exitCode = 0;
  const sessions = new Map([
    ['other', {
      id: 'other',
      manualLogin: true,
      persistentProfile: true,
      userDataDir: '/profiles/other',
      externalProcess: active,
    }],
    ['inactive', {
      id: 'inactive',
      manualLogin: true,
      persistentProfile: true,
      userDataDir: '/profiles/alex',
      externalProcess: inactive,
    }],
    ['target', {
      id: 'target',
      manualLogin: true,
      persistentProfile: true,
      userDataDir: '/profiles/alex',
      externalProcess: active,
    }],
  ]);

  const session = activeManualLoginSessionForUserDataDir(sessions, '/profiles/alex');

  assert.equal(session.id, 'target');
});

test('active persistent automation session is found by profile storage path', () => {
  const active = new FakeChild();
  const sessions = new Map([
    ['manual', {
      id: 'manual',
      manualLogin: true,
      persistentProfile: true,
      userDataDir: '/profiles/alex',
      externalProcess: active,
    }],
    ['automation', {
      id: 'automation',
      manualLogin: false,
      persistentProfile: true,
      userDataDir: '/profiles/alex',
      context: {},
    }],
  ]);

  const session = activePersistentProfileSessionForUserDataDir(sessions, '/profiles/alex', {
    includeManualLogin: false,
  });

  assert.equal(session.id, 'automation');
});

test('browser profile unlock wait polls until singleton lock clears', async () => {
  const states = [
    { locked: true, marker: 'SingletonLock' },
    { locked: true, marker: 'SingletonLock' },
    { locked: false },
  ];
  let sleeps = 0;

  const info = await waitForBrowserProfileUnlock('/profiles/alex', {
    timeoutMs: 100,
    intervalMs: 1,
    lockInfo: () => states.shift() || { locked: false },
    sleep: async () => { sleeps += 1; },
  });

  assert.equal(info.locked, false);
  assert.equal(sleeps, 2);
});

test('browser profile unlock can close an active profile owner before retrying', async () => {
  const states = [
    { locked: true, active: true, pid: 4242, marker: 'SingletonLock' },
    { locked: true, active: true, pid: 4242, marker: 'SingletonLock' },
    { locked: true, active: true, pid: 4242, marker: 'SingletonLock' },
    { locked: false },
  ];
  const signals = [];
  let sleeps = 0;

  const info = await waitForBrowserProfileUnlock('/profiles/alex', {
    timeoutMs: 1,
    closeActiveOwner: true,
    closeTimeoutMs: 100,
    intervalMs: 1,
    lockInfo: () => states.shift() || { locked: false },
    sleep: async () => {
      sleeps += 1;
      await new Promise((resolve) => setTimeout(resolve, 2));
    },
    terminatePid: (pid, signal) => { signals.push([pid, signal]); },
  });

  assert.equal(info.locked, false);
  assert.deepEqual(signals, [[4242, 'SIGTERM']]);
  assert(sleeps >= 2);
});

test('browser profile unlock clears stale singleton markers before retrying', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'agentark-stale-profile-lock-'));
  try {
    for (const marker of ['SingletonLock', 'SingletonSocket', 'SingletonCookie']) {
      fs.writeFileSync(path.join(dir, marker), marker === 'SingletonLock' ? 'host-424242' : 'stale');
    }
    let sleeps = 0;

    const info = await waitForBrowserProfileUnlock(dir, {
      timeoutMs: 50,
      intervalMs: 1,
      lockInfo: () => browserProfileLockInfo(dir, { pidAlive: () => false }),
      sleep: async () => { sleeps += 1; },
    });

    assert.equal(info.locked, false);
    assert.equal(fs.existsSync(path.join(dir, 'SingletonLock')), false);
    assert.equal(fs.existsSync(path.join(dir, 'SingletonSocket')), false);
    assert.equal(fs.existsSync(path.join(dir, 'SingletonCookie')), false);
    assert(sleeps >= 1);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('manual login log helper ignores writes after close', () => {
  const stream = new FakeWritable();
  const log = makeManualLoginLog(stream, '/tmp/agentark-manual-login.log');

  assert.equal(writeManualLoginLog(log, '[before]\n'), true);
  closeManualLoginLog(log);

  assert.doesNotThrow(() => writeManualLoginLog(log, '[after]\n'));
  assert.deepEqual(stream.writes, ['[before]\n']);
});
