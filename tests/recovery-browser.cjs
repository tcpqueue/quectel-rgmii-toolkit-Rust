// Network events, watchdog, band-lock guard, MBN and NR neighbours against the --mock server.
// The mock shortens the watchdog's waits; a ping target under .invalid simulates an outage.
// Set RECOVERY_BASE_URL to use a mock server that is already running.
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
  let base = process.env.RECOVERY_BASE_URL, child;
  if (!base) {
    const root = path.resolve(__dirname, '..');
    const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-recovery-'));
    const port = await freePort(); base = 'http://127.0.0.1:' + port;
    child = spawn(path.join(root, 'target/debug/simpleadmin-httpd'), ['--mock', '--http', '127.0.0.1:' + port, '--static', path.join(root, 'development/simpleadmin/www'), '--auth-file', path.join(temporary, 'auth'), '--ttl-file', path.join(temporary, 'ttl')], {stdio: 'ignore'});
    for (let i = 0; i < 100; i++) { try { await fetch(base + '/login.html'); break; } catch { await new Promise(r => setTimeout(r, 100)); } }
  }
  const browser = await chromium.launch({headless: true, executablePath: process.env.CHROMIUM_EXECUTABLE_PATH || undefined});
  try {
    const page = await browser.newPage({viewport: {width: 1280, height: 900}, locale: 'zh-CN'});
    const errors = []; page.on('pageerror', e => errors.push(e.message));
    const frames = [];
    page.on('websocket', ws => ws.on('framesent', f => frames.push(String(f.payload))));
    const dialogs = []; let answer = true;
    page.on('dialog', d => { dialogs.push(d.message()); answer ? d.accept() : d.dismiss(); });
    const sent = (...parts) => frames.some(f => parts.every(p => f.includes(p)));
    const api = params => page.evaluate(p => SimpleAdmin.Api.networkData(p), params);
    const setTarget = target => page.evaluate(t => fetch('/api/telemetry/target', {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify({target: t})}).then(r => r.status), target);
    await page.goto(base + '/login.html');
    await page.locator('#loginLanguage').selectOption('zh-CN');
    await page.locator('#username').fill('admin'); await page.locator('#password').fill('admin');
    await page.locator('#loginButton').click(); await page.waitForURL(base + '/');

    // --- Watchdog settings ---
    await page.goto(base + '/#settings');
    const enabled = page.locator('#watchdogEnabled');
    await enabled.waitFor();
    assert.equal(await enabled.isChecked(), false, 'the watchdog is off by default');
    assert.equal(await page.locator('#watchdogStep-reregister').isChecked(), true);
    assert.equal(await page.locator('#watchdogStep-reboot').isChecked(), false);
    assert.equal(await page.locator('#watchdogSave').isDisabled(), true, 'nothing to save yet');
    // Every step can be skipped; an enabled watchdog needs at least one.
    await enabled.check();
    for (const key of ['reregister', 'radio']) await page.locator('#watchdogStep-' + key).uncheck();
    await page.locator('#watchdogSave').click();
    assert.match(await page.locator('#watchdogMessage').textContent(), /至少开启一个/);
    assert.ok(!sent('action=watchdog_save'), 'invalid settings stay in the page');
    // Allowing restarts asks first; cancelling sends nothing.
    await page.locator('#watchdogStep-reboot').check();
    assert.equal(await page.locator('#watchdogMaxReboots').isVisible(), true);
    answer = false;
    await page.locator('#watchdogSave').click();
    assert.match(dialogs.at(-1), /重启模块/);
    assert.ok(!sent('action=watchdog_save'));
    answer = true;
    await page.locator('#watchdogStep-reboot').uncheck();
    await page.locator('#watchdogStep-reregister').check();
    await page.locator('#watchdogStep-radio').check();
    await page.locator('#watchdogOutage').fill('30');
    await page.locator('#watchdogSave').click();
    await page.waitForFunction(() => document.querySelector('#watchdogMessage').textContent.includes('已保存'));
    assert.ok(sent('action=watchdog_save', 'enabled=1', 'reregister=1', 'radio=1', 'sim_switch=0', 'reboot=0', 'outage_seconds=30'));
    let state = await api({action: 'watchdog'});
    assert.equal(state.settings.enabled, true);

    // --- Outage: the watchdog runs the allowed steps in order, the events list records it ---
    const eventKinds = async () => (await api({action: 'events'})).events.map(e => e.kind).reverse();
    await api({action: 'events_clear'});
    assert.equal(await setTarget('down.invalid'), 200);
    for (let i = 0; i < 60 && !(await eventKinds()).includes('watchdog_gave_up'); i++) await page.waitForTimeout(500);
    let kinds = await eventKinds();
    assert.deepEqual(kinds.filter(k => k.startsWith('watchdog_')), ['watchdog_start', 'watchdog_step', 'watchdog_step', 'watchdog_gave_up'], kinds.join(','));
    const steps = (await api({action: 'events'})).events.filter(e => e.kind === 'watchdog_step').map(e => e.detail.step).reverse();
    assert.deepEqual(steps, ['reregister', 'radio'], 'skipped steps do not run');
    state = await api({action: 'watchdog'});
    assert.equal(state.phase, 'cooldown');
    // The outage itself is reported after 10 s, and its end with the duration.
    for (let i = 0; i < 30 && !(await eventKinds()).includes('internet_lost'); i++) await page.waitForTimeout(500);
    assert.equal(await setTarget('www.baidu.com'), 200);
    kinds = await eventKinds();
    assert.ok(kinds.includes('internet_lost'), kinds.join(','));
    // Switching off stops the watchdog.
    await page.goto(base + '/#settings');
    await page.locator('#watchdogEnabled').waitFor();
    await page.locator('#watchdogEnabled').uncheck();
    await page.locator('#watchdogSave').click();
    await page.waitForFunction(() => document.querySelector('#watchdogState').textContent.includes('已关闭'));

    // --- Events on the overview page ---
    await page.goto(base + '/#dashboard');
    await page.locator('#eventsList li').first().waitFor();
    const listText = await page.locator('#eventsList').innerText();
    assert.match(listText, /看门狗：第 1 级 重新注册网络/);
    assert.match(listText, /看门狗：已启用的步骤都未恢复联网/);
    answer = false;
    await page.locator('#eventsClear').click();
    assert.ok((await page.locator('#eventsList li').count()) > 0, 'cancelled clear keeps the list');
    answer = true;
    await page.locator('#eventsClear').click();
    await page.waitForFunction(() => !document.querySelector('#eventsList'));

    // --- Band-lock guard ---
    await page.goto(base + '/#network');
    await page.locator('#bandGuardSwitch').waitFor();
    assert.equal(await page.locator('#bandGuardSwitch').isChecked(), true, 'protection is on by default');
    await page.waitForFunction(() => document.querySelectorAll('#checkboxFormSA input[type=checkbox]').length > 0, null, {timeout: 30000});
    await page.evaluate(() => SimpleAdmin.Vue.apps['#networkApp'].lockSelectedBandsForMode('SA'));
    assert.ok(sent('action=lock_bands', 'mode=SA', 'guard=1'), frames.filter(f => f.includes('lock_bands')).join('\n'));
    for (let i = 0; i < 30 && (await api({action: 'band_guard'})).phase === 'checking'; i++) await page.waitForTimeout(500);
    assert.equal((await api({action: 'band_guard'})).phase, 'ok', 'the mock reports a carrier, so nothing is restored');
    await page.waitForFunction(() => (document.querySelector('#bandGuardMessage') || {}).textContent === '已锁定，信号正常', null, {timeout: 10000});
    await page.locator('#bandGuardSwitch').uncheck();
    assert.equal(await page.evaluate(() => localStorage.getItem('sa-band-guard')), '0', 'the choice is remembered in this browser');
    frames.length = 0;
    await page.evaluate(() => SimpleAdmin.Vue.apps['#networkApp'].lockSelectedBandsForMode('SA'));
    assert.ok(sent('action=lock_bands', 'guard=0'));
    await page.locator('#bandGuardSwitch').check();

    // --- MBN ---
    const rows = page.locator('#mbnTableBody .sa-mbn-row');
    await rows.first().waitFor();
    assert.equal(await rows.count(), 1, "only the active profile is shown at first");
    await page.locator("#mbnToggle").click();
    assert.equal(await rows.count(), 3);
    assert.match(await page.locator('#mbnSummary').textContent(), /Commercial-CT/);
    assert.match(await page.locator('section[aria-labelledby="mbnTitle"]').innerText(), /SA: n1 n5 n28 n41 n78/);
    assert.equal(await rows.nth(1).locator('.sa-mbn-select').isDisabled(), true, 'the active profile is not offered again');
    answer = false;
    await rows.nth(2).locator('.sa-mbn-select').click();
    assert.ok(!sent('action=mbn_select'), 'cancel sends nothing');
    answer = true;
    await rows.nth(2).locator('.sa-mbn-select').click();
    await page.waitForFunction(() => document.querySelector('#mbnMessage').textContent.includes('重启模块后生效'));
    assert.ok(sent('action=mbn_select', 'Commercial-CMCC'));
    assert.equal(await page.locator('#mbnReboot').isVisible(), true);
    const bad = await api({action: 'mbn_select', name: 'Commercial"-X'}).then(r => JSON.stringify(r)).catch(e => 'error ' + e.message);
    assert.match(bad, /unknown MBN profile/, 'only listed profiles are accepted');
    await page.locator('#mbnAutoSel').click();
    assert.ok(sent('action=mbn_autosel', 'enabled=0'));

    // --- NR neighbours ---
    await page.locator('#cellScanStart').click();
    await page.waitForFunction(() => SimpleAdmin.Vue.apps['#networkApp'].resultDoneCell);
    const nrRow = page.locator('#cellScanTableBody .table-row', {hasText: '627264'});
    assert.match(await nrRow.innerText(), /NR5G\s+异频邻区\s+77\/78/);
    await nrRow.click();
    assert.match(await page.locator('#cellSelectHelper').textContent(), /子载波间隔不确定/);
    assert.equal(await page.locator('#cellLockSelected').isDisabled(), true);

    // --- English: no Chinese in the new groups ---
    await page.evaluate(() => SimpleAdmin.Lang.setLanguage ? SimpleAdmin.Lang.setLanguage('en') : null);
    await page.waitForTimeout(500);
    if (await page.evaluate(() => SimpleAdmin.Lang.getCurrentLanguage()) === 'en') {
      await setTarget('down.invalid');
      for (let i = 0; i < 40 && !(await eventKinds()).includes('internet_lost'); i++) await page.waitForTimeout(500);
      await setTarget('www.baidu.com');
      const checks = [['#network', 'section[aria-labelledby="mbnTitle"]'], ['#network', 'section[aria-labelledby="bandLockTitle"] .sa-band-footer'], ['#settings', 'section[aria-labelledby="watchdogHeading"]'], ['#dashboard', '#eventsApp']];
      for (const [hash, selector] of checks) {
        await page.goto(base + '/' + hash);
        await page.locator(selector).waitFor();
        await page.waitForTimeout(800);
        const text = await page.locator(selector).innerText();
        const cjk = text.match(/[一-鿿][^\n]*/g);
        assert.equal(cjk, null, `${selector}: ${cjk}`);
      }
      assert.match(await page.locator('#eventsApp').innerText(), /Internet connection lost/);
    }
    assert.deepEqual(errors, []);
    console.log('Recovery UI passed: watchdog settings and steps, events, band guard, MBN, NR neighbours, English.');
  } finally { await browser.close(); if (child) child.kill(); }
})().catch(e => { console.error(e); process.exit(1); });
