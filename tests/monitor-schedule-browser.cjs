const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const {spawn} = require('node:child_process');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}

(async () => {
  const root = path.resolve(__dirname, '..');
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-monitor-schedule-'));
  const screenshots = process.env.SCREENSHOT_DIR || temporary;
  const port = await freePort();
  const base = 'http://127.0.0.1:' + port;
  const child = spawn(path.join(root, 'target/debug/simpleadmin-httpd'), [
    '--mock', '--http', '127.0.0.1:' + port,
    '--static', path.join(root, 'development/simpleadmin/www'),
    '--auth-file', path.join(temporary, 'auth'), '--ttl-file', path.join(temporary, 'ttl')
  ], {stdio: 'ignore'});
  let browser;
  try {
    for (let i = 0; i < 100; i++) {
      try { await fetch(base + '/login.html'); break; }
      catch { await new Promise(resolve => setTimeout(resolve, 100)); }
    }
    browser = await chromium.launch({headless: true});
    const page = await browser.newPage({viewport: {width: 1440, height: 1000}});
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(base + '/login.html');
    await page.locator('#loginLanguage').selectOption('zh-CN');
    await page.locator('#username').fill('admin');
    await page.locator('#password').fill('admin');
    await page.locator('#loginButton').click();
    await page.waitForURL(base + '/');

    const toggle = page.locator('.art-monitor-settings-toggle');
    await toggle.click();
    const panel = page.locator('#monitorSettings');
    await panel.waitFor();
    assert.equal(await toggle.getAttribute('aria-expanded'), 'true');
    // Defaults: 1 s ICMP probe (about 9.9 MB/day) and 5 s background sampling.
    assert.match(await panel.locator('.art-monitor-cost').textContent(), /9\.9/);
    await panel.screenshot({path: path.join(screenshots, 'monitor-settings.png')});

    const [pingSwitch, sampleSwitch] = await panel.locator('input[type=checkbox]').all();
    const [pingInterval, sampleInterval] = await panel.locator('input[type=number]').all();
    const save = panel.locator('button[type=submit]');
    assert.equal(await save.isDisabled(), true, 'nothing to save before an edit');

    await sampleInterval.fill('1');
    await save.click();
    await page.getByText('延迟间隔为 1–300 秒，采样间隔为 2–300 秒').waitFor();

    await sampleInterval.fill('30');
    await pingSwitch.uncheck();
    assert.equal(await pingInterval.isDisabled(), true);
    await save.click();
    await page.getByText('监测设置已保存').waitFor();
    const snapshot = await page.evaluate(async () => (await fetch('/api/telemetry')).json());
    assert.deepEqual(snapshot.schedule, {ping_enabled: false, ping_interval: 1, sample_enabled: true, sample_interval: 30});
    await page.locator('#pingChartTitle + span').filter({hasText: '延迟监测已关闭'}).waitFor();
    await page.locator('#signalChartTitle + span').filter({hasText: '30 秒采样'}).waitFor();
    await page.locator('.art-live-state').filter({hasText: '延迟监测已关闭'}).waitFor();
    assert.equal(await page.evaluate(async () => (await fetch('/api/get_ping')).text()), 'OK');

    await sampleSwitch.uncheck();
    await save.click();
    await page.getByText('监测设置已保存').waitFor();
    await page.locator('#trafficChartTitle + span').filter({hasText: '后台采样已关闭'}).waitFor();
    // With background sampling off, dashboard refreshes still record live samples.
    await page.waitForFunction(async () => (await (await fetch('/api/telemetry')).json()).signal.length > 0);

    // Settings survive a restart through monitor.json.
    const saved = JSON.parse(fs.readFileSync(path.join(temporary, 'monitor.json'), 'utf8'));
    assert.equal(saved.ping_enabled, false);
    assert.equal(saved.sample_enabled, false);
    assert.equal(saved.target, 'www.baidu.com');

    await page.evaluate(() => SimpleAdmin.Lang.setLanguage ? SimpleAdmin.Lang.setLanguage('en') : null);
    await page.locator('#pingChartTitle + span').filter({hasText: 'Latency probe off'}).waitFor();

    await page.setViewportSize({width: 390, height: 900});
    await page.emulateMedia({colorScheme: 'dark'});
    await page.evaluate(() => document.documentElement.setAttribute('data-bs-theme', 'dark'));
    await panel.screenshot({path: path.join(screenshots, 'monitor-settings-mobile-dark.png')});
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    assert(overflow <= 0, 'monitor settings must not cause horizontal scrolling on phones');

    assert.deepEqual(errors, []);
    console.log('Monitor schedule browser passed: validation, save, labels, connectivity fallback, page-driven samples, persistence, i18n and mobile layout. Screenshots: ' + screenshots);
  } finally {
    if (browser) await browser.close();
    child.kill();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
