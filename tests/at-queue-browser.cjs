// AT queue on the AT terminal page against the --mock server: a slow scan holds the port, the
// terminal command waits behind it, both then show up as finished with their responses.
// Set ATQ_BASE_URL to use a mock server that is already running (the /tmp copy is then not checked).
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
  let base = process.env.ATQ_BASE_URL, child, temporary;
  if (!base) {
    const root = path.resolve(__dirname, '..');
    temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-atq-'));
    const port = await freePort(); base = 'http://127.0.0.1:' + port;
    child = spawn(path.join(root, 'target/debug/simpleadmin-httpd'), ['--mock', '--http', '127.0.0.1:' + port, '--static', path.join(root, 'development/simpleadmin/www'), '--auth-file', path.join(temporary, 'auth'), '--ttl-file', path.join(temporary, 'ttl')], {stdio: 'ignore'});
    for (let i = 0; i < 100; i++) { try { await fetch(base + '/login.html'); break; } catch { await new Promise(r => setTimeout(r, 100)); } }
  }
  const browser = await chromium.launch({headless: true, executablePath: process.env.CHROMIUM_EXECUTABLE_PATH || undefined});
  try {
    const page = await browser.newPage({viewport: {width: 1280, height: 900}, locale: 'zh-CN'});
    const errors = []; page.on('pageerror', e => errors.push(e.message));
    await page.goto(base + '/login.html');
    await page.locator('#loginLanguage').selectOption('zh-CN');
    await page.locator('#username').fill('admin'); await page.locator('#password').fill('admin');
    await page.locator('#loginButton').click(); await page.waitForURL(base + '/');

    // Only the AT terminal view shows the queue, and only it polls.
    await page.goto(base + '/#system');
    await page.waitForFunction(() => location.hash === '#system');
    assert.equal(await page.locator('#atQueueHeading').isVisible(), false, 'hidden outside the AT view');
    await page.locator('.sa-sidebar [data-page-link="at"]').click();
    await page.locator('#atQueueHeading').waitFor();
    for (const id of ['atTerminalHeading', 'atQueueHeading', 'atQueueDoneHeading']) assert(await page.locator('#' + id).isVisible(), id);
    await page.waitForFunction(() => /执行中 \d · 等待 \d+ · 最近 30 秒 \d+ 条/.test(document.querySelector('#atQueueSummary').textContent));

    // A slow scan holds the port; the terminal command waits behind it.
    await page.evaluate(() => SimpleAdmin.Api.mockAT({action: 'set', kind: 'scan_delay', payload: '4000'}));
    await page.evaluate(() => SimpleAdmin.Api.networkData({action: 'scan_start', kind: 'full', mode: 'Full Scan'}));
    await page.locator('#atQueueRunning', {hasText: 'AT+QSCAN=3,1'}).waitFor({timeout: 5000});
    await page.locator('#atCommandInput').fill('AT+CIMI');
    await page.locator('#atCommandInput').press('Enter');
    await page.locator('.sa-atq-waiting', {hasText: 'AT+CIMI'}).first().waitFor({timeout: 5000});
    assert.match(await page.locator('#atQueueSummary').textContent(), /执行中 1 · 等待 [1-9]/);

    // Both finish; the newest is on top and opens to its response.
    const done = page.locator('#atQueueDone details');
    await page.locator('#atQueueDone summary', {hasText: 'AT+CIMI'}).first().waitFor({timeout: 15000});
    await page.locator('#atQueueDone summary', {hasText: 'AT+QSCAN=3,1'}).first().waitFor({timeout: 5000});
    await page.locator('#atQueueIdle').waitFor({timeout: 10000});
    const model = done.filter({hasText: 'AT+CIMI'}).first();
    assert.equal(await model.locator('.ui-tag').textContent(), '成功');
    await model.locator('summary').click();
    assert.match(await model.locator('pre').textContent(), /420031234567890[\s\S]*OK/);
    assert.match(await model.locator('.sa-atq-detail small').textContent(), /排队 [\d.]+ (ms|s) · 执行/);
    // The opened entry stays open across refreshes.
    await page.waitForTimeout(2500);
    assert.equal(await model.evaluate(d => d.open), true, 'stays open while polling');

    // Pausing freezes the list.
    await page.locator('#atQueuePause').click();
    assert.equal(await page.locator('#atQueuePause').textContent(), '继续刷新');
    const frozen = await page.locator('#atQueueDone').innerHTML();
    await page.evaluate(() => SimpleAdmin.Api.settingsData({action: 'manual_at', command: 'AT+QTEMP'}));
    await page.waitForTimeout(2000);
    assert.equal(await page.locator('#atQueueDone').innerHTML(), frozen, 'no updates while paused');
    await page.locator('#atQueuePause').click();
    await page.locator('#atQueueDone summary', {hasText: 'AT+QTEMP'}).first().waitFor({timeout: 5000});

    // The API returns the same queue the page shows.
    const queue = await page.evaluate(() => SimpleAdmin.Api.settingsData({action: 'at_queue'}));
    assert(queue.done.some(item => item.command === 'AT+CIMI' && item.status === 'ok' && item.response.includes('420031234567890')));

    // The copy in the state directory (/tmp on the module) follows the queue.
    if (temporary) {
      const file = path.join(temporary, 'at-queue.json');
      let saved = {};
      for (let i = 0; i < 40 && !(saved.done || []).some(item => item.command === 'AT+QTEMP'); i++) {
        await page.waitForTimeout(250);
        try { saved = JSON.parse(fs.readFileSync(file, 'utf8')); } catch (_) {}
      }
      assert(saved.done.some(item => item.command === 'AT+QTEMP'), 'the file has the latest command');
      if (process.platform !== 'win32') assert.equal(fs.statSync(file).mode & 0o777, 0o600);
    }

    // The terminal output grows with a long reply, up to 70 % of the window.
    const box = page.locator('#atOutputBox');
    const height = () => box.evaluate(el => el.offsetHeight);
    assert.equal(await box.evaluate(el => getComputedStyle(el).resize), 'vertical', 'the box can be dragged');
    await page.locator('#atCommandInput').fill('AT+QSIMSTAT?;+CSQ;+QTEMP;+QUIMSLOT?;+QSPN;+QMAP="WWAN";+QENG="servingcell";+QCAINFO;+QGDNRCNT?;+QGDCNT?;+CGCONTRDP=1;+QRSRP');
    await page.locator('#atCommandInput').press('Enter');
    await page.waitForFunction(() => document.querySelector('#atOutputBox').value.includes('+QCAINFO'));
    await page.waitForTimeout(200);
    const grown = await height();
    assert(grown > 260 && grown <= 900 * 0.7 + 1, `fits the reply: ${grown}`);
    assert(await box.evaluate(el => el.scrollHeight <= el.clientHeight + 2 || el.offsetHeight >= window.innerHeight * 0.7 - 1), 'no inner scrolling below the cap');
    // Clearing shrinks it back; a size the user dragged to is kept for short replies.
    await page.locator('.sa-at-input button', {hasText: '清除'}).click();
    await page.waitForTimeout(200);
    assert.equal(await height(), 200);
    await box.evaluate(el => { el.style.height = '420px'; });
    await page.locator('#atCommandInput').fill('AT+CIMI');
    await page.locator('#atCommandInput').press('Enter');
    await page.waitForFunction(() => document.querySelector('#atOutputBox').value.includes('420031234567890'));
    await page.waitForTimeout(200);
    assert.equal(await height(), 420, 'keeps the dragged size');

    // Phone width: commands wrap instead of widening the page.
    await page.setViewportSize({width: 390, height: 844});
    await page.waitForTimeout(300);
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1), 'no horizontal scroll');
    assert.deepEqual(errors, []);
    console.log('at queue browser test passed');
  } finally {
    await browser.close();
    if (child) child.kill();
    if (temporary) fs.rmSync(temporary, {recursive: true, force: true});
  }
})().catch(error => { console.error(error); process.exit(1); });
