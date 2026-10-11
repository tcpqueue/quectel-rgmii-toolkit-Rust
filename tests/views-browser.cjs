// Split menu, 5G frequency lock, background scans and outage notices against the --mock server.
// Set VIEWS_BASE_URL to use a mock server that is already running.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const {spawn} = require('node:child_process');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
async function freePort() {
  const server = net.createServer(); await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port; await new Promise(resolve => server.close(resolve)); return port;
}
(async () => {
  let base = process.env.VIEWS_BASE_URL, child;
  if (!base) {
    const root = path.resolve(__dirname, '..');
    const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-views-'));
    const port = await freePort(); base = 'http://127.0.0.1:' + port;
    child = spawn(path.join(root, 'target/debug/simpleadmin-httpd'), ['--mock', '--http', '127.0.0.1:' + port, '--static', path.join(root, 'development/simpleadmin/www'), '--auth-file', path.join(temporary, 'auth'), '--ttl-file', path.join(temporary, 'ttl')], {stdio: 'ignore'});
    for (let i = 0; i < 100; i++) { try { await fetch(base + '/login.html'); break; } catch { await new Promise(r => setTimeout(r, 100)); } }
  }
  const browser = await chromium.launch({headless: true, executablePath: process.env.CHROMIUM_EXECUTABLE_PATH || undefined});
  try {
    const page = await browser.newPage({viewport: {width: 1280, height: 900}, locale: 'zh-CN'});
    const errors = []; page.on('pageerror', e => errors.push(e.message));
    page.on('dialog', d => d.accept());
    const api = params => page.evaluate(p => SimpleAdmin.Api.networkData(p).catch(e => ({ok: false, error: e.message})), params);
    const visible = id => page.locator(`section[aria-labelledby="${id}"]`).isVisible();
    // Polls an API until the check passes (waitForFunction treats a returned promise as true).
    const until = async (path, check, timeout, label) => {
      const end = Date.now() + timeout;
      for (;;) {
        const data = await page.evaluate(p => fetch(p).then(r => r.json()), path);
        if (check(data)) return data;
        if (Date.now() > end) throw new Error('timed out waiting for ' + label);
        await page.waitForTimeout(500);
      }
    };
    // Shorthands for the page's Vue apps inside page functions.
    await page.addInitScript(() => { window.net = () => SimpleAdmin.Vue.apps['#networkApp']; window.fw = () => SimpleAdmin.Vue.apps['#forwardingApp']; });
    await page.goto(base + '/login.html');
    await page.locator('#loginLanguage').selectOption('zh-CN');
    await page.locator('#username').fill('admin'); await page.locator('#password').fill('admin');
    await page.locator('#loginButton').click(); await page.waitForURL(base + '/');

    // --- Menu: one entry per view; shared pages show only their view's groups ---
    assert.equal(await page.locator('.sa-sidebar [data-page-link]').count(), 13);
    assert.equal(await page.locator('#page-dashboard #eventsApp').count(), 0, 'events moved off the overview');
    await page.goto(base + '/#network');
    await page.waitForFunction(() => location.hash === '#bands');
    assert.equal(await page.locator('#page-title-network').textContent(), '频段与频点');
    for (const [id, shown] of [['bandLockTitle', true], ['freqLockTitle', true], ['mbnTitle', true], ['cellScanTitle', false], ['pdpTitle', false]]) assert.equal(await visible(id), shown, id);
    await page.locator('.sa-sidebar [data-page-link="cells"]').click();
    for (const [id, shown] of [['cellLockTitle', true], ['cellScanTitle', true], ['manualLockTitle', true], ['bandLockTitle', false]]) assert.equal(await visible(id), shown, id);
    await page.locator('.sa-sidebar [data-page-link="connection"]').click();
    for (const [id, shown] of [['networkParamsTitle', true], ['pdpTitle', true], ['networkSwitchTitle', true], ['freqLockTitle', false]]) assert.equal(await visible(id), shown, id);
    const settingsViews = {watchdog: ['watchdogHeading'], lan: ['networkFeaturesHeading', 'ttlHeading'], at: ['atTerminalHeading'], system: ['accountSecurityHeading', 'preferencesHeading', 'otaHeading', 'maintenanceHeading']};
    const allSettings = Object.values(settingsViews).flat();
    for (const [view, ids] of Object.entries(settingsViews)) {
      await page.locator(`.sa-sidebar [data-page-link="${view}"]`).click();
      for (const id of allSettings) assert.equal(await visible(id), ids.includes(id), `${view}: ${id}`);
    }
    await page.goto(base + '/#forwarding');
    await page.waitForFunction(() => location.hash === '#notify' && SimpleAdmin.Vue.apps['#forwardingApp']?.loaded);
    await page.goto(base + '/#events');
    await page.locator('#eventsApp').waitFor();

    // --- Phone: tab bar groups and the sub-menu ---
    await page.setViewportSize({width: 390, height: 844});
    await page.locator('.sa-tabbar [data-tab="network"]').click();
    await page.waitForFunction(() => location.hash === '#connection', null, {timeout: 5000});
    assert(await page.locator('#page-network .sa-subnav').isVisible(), 'the group reopens its last view');
    await page.locator('#page-network .sa-subnav [data-view-link="cells"]').click();
    assert.equal(await page.locator('.sa-tabbar [data-tab="network"]').getAttribute('aria-current'), 'page');
    await page.locator('.sa-tabbar [data-tab="message"]').click();
    await page.waitForFunction(() => location.hash === '#notify', null, {timeout: 5000});
    await page.locator('#page-notify .sa-subnav [data-view-link="sms"]').click();
    await page.waitForFunction(() => location.hash === '#sms', null, {timeout: 5000});
    await page.locator('.sa-tabbar [data-tab="network"]').click();
    await page.waitForFunction(() => location.hash === '#cells', null, {timeout: 5000});
    assert(!(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth)), 'no horizontal scroll on a phone');
    await page.setViewportSize({width: 1280, height: 900});

    // --- 5G frequency lock: strict checks, guard, persistence ---
    await page.goto(base + '/#bands');
    await page.waitForFunction(() => SimpleAdmin.Vue.apps['#networkApp']?.freqLoaded && SimpleAdmin.Vue.apps['#networkApp'].freq.allowed_bands);
    assert.deepEqual(await page.evaluate(() => [...SimpleAdmin.Vue.apps['#networkApp'].freq.allowed_bands]), [1, 5, 28, 41, 78]);
    for (const [entries, message] of [['627264:15', 'not valid for n78'], ['152650:30', 'not valid for n28'], ['2079165:120', 'not in a supported FR1 band'], ['396000:15', 'outside the allowed bands'], ['633984', 'SCS'], ['', 'enter 1 to 32']]) {
      const result = await api({action: 'freq_lock_set', entries});
      assert.equal(result.ok, false, entries);
      assert(result.error.includes(message), `${entries}: ${result.error}`);
    }
    const arfcn = page.locator('.sa-freq-arfcn').first();
    await arfcn.fill('633984'); await arfcn.dispatchEvent('change');
    assert.equal(await page.locator('.sa-freq-scs').first().inputValue(), '30', 'n78 SCS picked');
    assert((await page.locator('.sa-freq-hint').first().textContent()).includes('n78'));
    await page.locator('#freqLockButton').click();
    await page.waitForFunction(() => { const a = SimpleAdmin.Vue.apps['#networkApp']; return a.freq.guard === 'ok' && !a.freqBusy; }, null, {timeout: 15000});
    assert.equal(await page.locator('#freqState').textContent(), '临时锁定 · 重启后失效');
    assert.deepEqual(await page.evaluate(() => JSON.parse(JSON.stringify(SimpleAdmin.Vue.apps['#networkApp'].freq.module))), [[633984, 30]]);
    const cellLock = await api({action: 'lock_nr_manual', pci: '1', earfcn: '633984', scs: '30', band: '78'});
    assert(cellLock.ok === false && cellLock.error.includes('frequency lock'), 'NR cell lock refused while frequencies are locked');
    await page.locator('#freqUnlock').click();
    await page.waitForFunction(() => { const a = SimpleAdmin.Vue.apps['#networkApp']; return !a.freqBusy && a.freq.module && a.freq.module.length === 0; });
    // A frequency the serving cell is not on: the guard puts the old state back.
    await arfcn.fill('627264'); await arfcn.dispatchEvent('change');
    await page.locator('#freqLockButton').click();
    await page.waitForFunction(() => { const a = SimpleAdmin.Vue.apps['#networkApp']; return a.freq.guard === 'reverted'; }, null, {timeout: 20000});
    assert(await page.locator('#freqMessage').evaluate(e => e.classList.contains('text-danger')));
    assert.equal((await api({action: 'freq_lock'})).module.length, 0);
    assert((await api({action: 'events'})).events.some(e => e.kind === 'freq_reverted' && e.detail.locked === '627264:30'));
    // A working previous lock comes back; a previous lock without service is removed too.
    const guardEnd = async () => { for (let i = 0; i < 60; i++) { const f = await api({action: 'freq_lock'}); if (f.guard !== 'checking') return f; await page.waitForTimeout(500); } throw new Error('guard still checking'); };
    assert.equal((await api({action: 'freq_lock_set', entries: '633984:30', guard: '0'})).ok, true);
    assert.equal((await api({action: 'freq_lock_set', entries: '627264:30', guard: '1'})).ok, true);
    let f = await guardEnd();
    assert.equal(f.guard, 'reverted');
    assert.deepEqual(f.module, [[633984, 30]], 'the working lock is back');
    assert.equal((await api({action: 'freq_lock_set', entries: '627264:30', guard: '0'})).ok, true);
    assert.equal((await api({action: 'freq_lock_set', entries: '640000:30', guard: '1'})).ok, true);
    f = await guardEnd();
    assert.equal(f.guard, 'reverted');
    assert.deepEqual(f.module, [], 'neither lock finds the cell: unlocked');
    assert((await api({action: 'events'})).events.some(e => e.kind === 'freq_reverted' && e.detail.locked === '640000:30' && e.detail.previous === ''));
    // Persistent: saved for restarts until unlocked.
    await arfcn.fill('633984'); await arfcn.dispatchEvent('change');
    await page.locator('input[name="freq-lock-mode"][value="persistent"]').check();
    await page.locator('#freqLockButton').click();
    await page.waitForFunction(() => SimpleAdmin.Vue.apps['#networkApp'].freq.guard === 'ok', null, {timeout: 15000});
    assert.equal(await page.locator('#freqState').textContent(), '持久锁定 · 重启后自动恢复');
    await page.locator('#freqUnlock').click();
    await page.waitForFunction(() => { const a = SimpleAdmin.Vue.apps['#networkApp']; return !a.freqBusy && !a.freq.saved && a.freq.module.length === 0; });

    // --- Background scans: they keep running across pages and stay cached ---
    await page.goto(base + '/#cells');
    await page.locator('.sa-scan-kind button').nth(1).click();
    await page.locator('#cellSelect').selectOption('NR5G Only');
    await page.locator('#cellScanStart').click();
    await page.locator('.sa-sidebar [data-page-link="bands"]').click();
    await until('/api/network_data?action=scan_status', s => !s.running && s.results.full, 30000, 'the full scan');
    await page.reload();
    await page.locator('.sa-sidebar [data-page-link="cells"]').click();
    await page.waitForFunction(() => SimpleAdmin.Vue.apps['#networkApp']?.resultDoneCell, null, {timeout: 15000});
    assert.equal(await page.evaluate(() => net().scanKind), 'full');
    assert.equal(await page.evaluate(() => net().cellScanMode), 'NR5G Only');
    assert((await page.locator('#scanAge').textContent()).includes('结果保留 10 分钟'));
    // The mock has no AT+QSCAN data: an empty but finished scan.
    assert.equal(await page.locator('#cellScanTableBody tr.table-row').count(), 0);
    assert.equal(await page.locator('#cellScanTableBody .ui-empty').textContent(), '没有可锁定的小区');
    await page.locator('.sa-scan-kind button').nth(0).click();
    await page.locator('#cellScanStart').click();
    await page.waitForFunction(() => net().resultDoneCell && net().scanKind === 'neighbour', null, {timeout: 15000});
    assert(await page.locator('#cellScanTableBody tr.table-row').count() > 0);
    await page.locator('.sa-scan-kind button').nth(1).click();
    await page.waitForFunction(() => net().resultDoneCell && net().cellScanMode === 'NR5G Only', null, {timeout: 5000});
    await page.locator('#cellSelect').selectOption('LTE Only');
    assert.equal(await page.evaluate(() => net().resultDoneCell), false, 'another mode has no cached result');
    await page.locator('.sa-scan-kind button').nth(0).click();
    await page.waitForFunction(() => net().resultDoneCell && net().nr5g_cells_parsed.length > 0, null, {timeout: 5000});
    // 5G cells from the scan can be added to the frequency lock, but only with a known SCS.
    await page.locator('.sa-sidebar [data-page-link="bands"]').click();
    assert.equal(await page.locator('.sa-freq-suggest').count(), 0, 'the mock neighbour has no SCS');
    await page.evaluate(() => { net().nr5g_cells_parsed = [...net().nr5g_cells_parsed, {type: 'NR5G', band: '78', freq: '627264', pci: '1', scs: 30}]; });
    await page.locator('.sa-freq-suggest .sa-chip').click();
    await page.waitForFunction(() => net().freqEntries.some(e => e.arfcn === '627264' && e.scs === '30'));

    // --- Notifications: shared channels, per-sender choice, outage notice ---
    await page.goto(base + '/#notify');
    await page.waitForFunction(() => SimpleAdmin.Vue.apps['#forwardingApp']?.loaded);
    await page.locator('#outageEnabled').check();
    await page.locator('.sa-forward-toolbar button[type="submit"]').click();
    await page.waitForFunction(() => !SimpleAdmin.Vue.apps['#forwardingApp'].busy);
    assert.equal(await page.evaluate(() => fw().message), 'choose an enabled channel for outage notices');
    await page.locator('#forward-tab-webhook').click();
    await page.locator('#forward-channel-webhook').check();
    await page.locator('#forward-url-webhook').fill('http://127.0.0.1:9/hook');
    await page.locator('#outagePlatforms input[data-outage-platform="webhook"]').check();
    await page.locator('#outageMinSeconds').fill('10');
    await page.locator('.sa-forward-toolbar button[type="submit"]').click();
    await page.waitForFunction(() => !SimpleAdmin.Vue.apps['#forwardingApp'].busy && !SimpleAdmin.Vue.apps['#forwardingApp'].dirty);
    const saved = await page.evaluate(() => fetch('/api/forwarding').then(r => r.json()));
    assert.equal(saved.outage_enabled, true);
    assert.deepEqual(saved.outage_platforms, ['webhook']);
    assert.equal(saved.outage_min_seconds, 10);
    assert.equal(saved.enabled, false, 'SMS forwarding stays off');
    assert.equal(saved.utc_offset_minutes, await page.evaluate(() => -new Date().getTimezoneOffset()));
    // The mock target fails for 14 s, then recovers: one notice, with the outage length.
    const setTarget = target => page.evaluate(t => fetch('/api/telemetry/target', {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify({target: t})}).then(r => r.status), target);
    assert.equal(await setTarget('down14.invalid'), 200);
    await until('/api/network_data?action=events', e => e.events.some(v => v.kind === 'internet_lost'), 30000, 'the outage');
    assert(!(await page.evaluate(() => fetch('/api/forwarding').then(r => r.json()))).records.some(r => r.event === 'outage'), 'nothing before the connection is back');
    const status = await until('/api/forwarding', s => s.records.some(r => r.event === 'outage'), 40000, 'the outage notice');
    const records = status.records.filter(r => r.event === 'outage');
    assert(records.every(r => r.platform === 'webhook'), 'only the chosen channel');
    assert((await api({action: 'events'})).events.some(e => e.kind === 'internet_restored'));
    await page.waitForFunction(() => document.querySelector('.sa-forward-table tbody').textContent.includes('断网恢复'), null, {timeout: 10000});
    assert.equal(await setTarget('www.baidu.com'), 200);
    await page.locator('#outageEnabled').uncheck();
    await page.locator('#forward-channel-webhook').uncheck();
    await page.locator('.sa-forward-toolbar button[type="submit"]').click();
    await page.waitForFunction(() => !SimpleAdmin.Vue.apps['#forwardingApp'].busy && !SimpleAdmin.Vue.apps['#forwardingApp'].dirty);

    // --- English ---
    await page.evaluate(() => SimpleAdmin.Lang.setLanguage('en'));
    await page.goto(base + '/#bands');
    await page.waitForFunction(() => document.querySelector('#freqLockTitle').textContent === '5G frequency lock');
    assert.equal(await page.locator('#page-title-network').textContent(), 'Bands & frequencies');
    assert.equal(await page.locator('.sa-sidebar [data-page-link="notify"]').textContent(), 'Notifications');
    await page.evaluate(() => SimpleAdmin.Lang.setLanguage('zh-CN'));

    assert.deepEqual(errors, []);
    console.log('views browser test passed');
  } finally {
    await browser.close();
    if (child) child.kill();
  }
})().catch(e => { console.error(e); process.exit(1); });
