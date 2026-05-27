const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  browserProfileLockInfo,
  buildManualLoginBrowserArgs,
  closeManualLoginLog,
  activeManualLoginSessionForUserDataDir,
  makeManualLoginLog,
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

test('chromium manual login uses Docker-safe launch flags', () => {
  const args = buildManualLoginBrowserArgs({
    browserName: 'chromium',
    userDataDir: '/profiles/alex',
    initialUrl: 'about:blank',
    width: 1600,
    height: 900,
  });

  assert(args.includes('--no-sandbox'));
  assert(args.includes('--disable-setuid-sandbox'));
  assert(args.includes('--disable-dev-shm-usage'));
  assert(args.includes('--start-maximized'));
  assert(args.includes('--window-size=1600,900'));
  assert.equal(args[0], '--user-data-dir=/profiles/alex');
  assert.equal(args.at(-1), 'about:blank');
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

test('sandbox saved profile automation does not use the manual login user-data dir', () => {
  assert.equal(savedProfileUsesPersistentContext({
    profileId: 'alex',
    manualLogin: false,
    targetKind: 'sandbox',
  }), false);
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

test('manual login log helper ignores writes after close', () => {
  const stream = new FakeWritable();
  const log = makeManualLoginLog(stream, '/tmp/agentark-manual-login.log');

  assert.equal(writeManualLoginLog(log, '[before]\n'), true);
  closeManualLoginLog(log);

  assert.doesNotThrow(() => writeManualLoginLog(log, '[after]\n'));
  assert.deepEqual(stream.writes, ['[before]\n']);
});
