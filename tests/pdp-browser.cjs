// PDP contexts and network feature switches on the network page, against the --mock server.
// Set PDP_BASE_URL to use a server that is already running instead of starting one.
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
  let base = process.env.PDP_BASE_URL, child;
  if (!base) {
    const root = path.resolve(__dirname, '..');
    const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-pdp-'));
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
    await page.goto(base + '/login.html');
    await page.locator('#loginLanguage').selectOption('zh-CN');
    await page.locator('#username').fill('admin'); await page.locator('#password').fill('admin');
    await page.locator('#loginButton').click(); await page.waitForURL(base + '/');
    await page.locator('.sa-menu-link[data-page-link="network"]').click();
    const rows = page.locator('#pdpTableBody .sa-pdp-row');
    await rows.first().waitFor();
    const settle = async () => { await page.waitForFunction(() => !SimpleAdmin.Vue.apps['#networkApp'].pdpBusy); };
    const sent = (...parts) => frames.some(f => parts.every(p => f.includes(p)));

    // Display.
    assert.equal(await rows.count(), 3);
    const row1 = rows.nth(0);
    assert.match(await row1.textContent(), /3gnet/);
    assert.match(await row1.locator('.sa-pdp-address').textContent(), /10\.0\.0\.197/);
    assert.match(await row1.locator('.sa-pdp-address').textContent(), /2001:db8::1/);
    assert.equal((await row1.locator('.sa-pdp-state').textContent()).trim(), '已激活');
    assert.equal(await row1.locator('.sa-pdp-toggle').isChecked(), true, 'an active context shows its switch on');
    assert.equal((await rows.nth(1).locator('.sa-pdp-state').textContent()).trim(), '未激活');
    assert.equal(await rows.nth(1).locator('.sa-pdp-toggle').isChecked(), false);
    assert.equal((await rows.nth(1).locator('.sa-pdp-address').textContent()).trim(), '-');
    assert.equal((await page.locator('#pdpSummary').textContent()).trim(), '3 个上下文 · 2 个已激活');
    assert.equal((await page.locator('#imsState').textContent()).trim(), '强制开启 · VoLTE 可用');
    const switches = page.locator('.sa-network-switches .ui-row');
    assert.match(await switches.nth(1).textContent(), /已关闭/);
    assert.equal(await page.locator('#simDetectSwitch').isChecked(), false);
    assert.match(await switches.nth(2).textContent(), /已开启/);
    assert.equal(await page.locator('#roamingSwitch').isChecked(), true);
    assert.match(await switches.nth(3).textContent(), /正在使用卡槽 1/);
    // The current choice is the highlighted segment.
    assert.equal((await switches.nth(0).locator('button.active').textContent()).trim(), '强制开启');
    assert.equal((await switches.nth(3).locator('button.active').textContent()).trim(), '卡槽 1');

    // Deactivating CID 1 asks first.
    await row1.locator('.sa-pdp-toggle').click(); await settle();
    assert.match(dialogs.at(-1), /CID 1/);
    assert.ok(sent('action=pdp_deactivate&', 'cid=1'), frames.join('\n'));
    assert.equal((await page.locator('#pdpMessage').textContent()).trim(), 'CID 1 已去激活');
    // Activating needs no confirmation.
    const before = dialogs.length;
    await rows.nth(1).locator('.sa-pdp-toggle').click(); await settle();
    assert.equal(dialogs.length, before);
    assert.ok(sent('action=pdp_activate&', 'cid=2'));
    // Edit keeps the CID and sends the new APN.
    await rows.nth(1).locator('.sa-pdp-edit').click();
    assert.equal(await page.locator('#pdpFormCid').isDisabled(), true);
    await page.locator('#pdpFormApn').fill('test.apn');
    await page.locator('#pdpFormSave').click(); await settle();
    assert.ok(sent('action=pdp_save&', 'cid=2', 'test.apn'));
    assert.equal(await page.locator('#pdpFormCid').count(), 0, 'form closes after saving');
    // New context takes the first free CID; a bad APN never leaves the page.
    await page.locator('#pdpNew').click();
    assert.equal(await page.locator('#pdpFormCid').inputValue(), '3');
    const sentBefore = frames.length;
    await page.locator('#pdpFormApn').fill('bad"apn');
    await page.locator('#pdpFormSave').click();
    assert.match(await page.locator('#pdpMessage').textContent(), /APN/);
    assert.equal(frames.filter(f => f.includes('action=pdp_save&')).length, frames.slice(0, sentBefore).filter(f => f.includes('action=pdp_save&')).length);
    await page.locator('#pdpFormApn').fill('internet');
    await page.locator('#pdpFormSave').click(); await settle();
    assert.ok(sent('action=pdp_save&', 'cid=3', 'internet', 'IPV4V6'));
    // Delete asks first.
    await rows.nth(1).locator('.sa-pdp-delete').click(); await settle();
    assert.ok(sent('action=pdp_delete&', 'cid=2'));
    // Switches.
    await switches.nth(0).locator('button', {hasText: '强制关闭'}).click(); await settle();
    assert.match(dialogs.at(-1), /重启/);
    assert.ok(sent('action=ims', 'mode=2'));
    await page.locator('#simDetectSwitch').click(); await settle();
    assert.ok(sent('action=sim_detect&', 'enabled=1'));
    assert.equal((await page.locator('#pdpMessage').textContent()).trim(), '已保存，重启模块后生效');
    await page.locator('#roamingSwitch').click(); await settle();
    assert.ok(sent('action=roaming', 'enabled=0'));
    // Cancelling the slot switch sends nothing.
    answer = false;
    await switches.nth(3).locator('button', {hasText: '卡槽 2'}).click();
    await page.waitForTimeout(300);
    assert.ok(!sent('action=sim_slot&'));
    answer = true;
    await switches.nth(3).locator('button', {hasText: '卡槽 2'}).click(); await settle();
    assert.ok(sent('action=sim_slot&', 'slot=2'));
    assert.equal(await page.locator('#pdpMessage').evaluate(el => el.classList.contains('text-danger')), false, await page.locator('#pdpMessage').textContent());

    // English leaves no Chinese in the two groups.
    await page.evaluate(() => SimpleAdmin.Lang.setLanguage ? SimpleAdmin.Lang.setLanguage('en') : null);
    await page.waitForTimeout(500);
    const lang = await page.evaluate(() => SimpleAdmin.Lang.getCurrentLanguage());
    if (lang === 'en') {
      for (const id of ['pdpTitle', 'networkSwitchTitle']) {
        const text = await page.locator(`section[aria-labelledby="${id}"]`).innerText();
        const cjk = text.match(/[一-鿿][^\n]*/g);
        assert.equal(cjk, null, `${id}: ${cjk}`);
      }
    }
    assert.deepEqual(errors, []);

    console.log(`PDP UI passed (language switch checked: ${lang === 'en'})`);
  } finally { await browser.close(); if (child) child.kill(); }
})().catch(e => { console.error(e); process.exit(1); });
