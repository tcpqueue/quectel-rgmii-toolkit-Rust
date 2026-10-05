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
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-traffic-direction-'));
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
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(base + '/login.html');
    await page.locator('#loginLanguage').selectOption('zh-CN');
    await page.locator('#username').fill('admin');
    await page.locator('#password').fill('admin');
    await page.locator('#loginButton').click();
    await page.waitForURL(base + '/');

    for (const [sent, received] of [[1024, 8192], [0, 4096], [4096, 0]]) {
      const result = await page.evaluate(async ({sent, received}) => {
        const response = await fetch('/api/mock_at', {
          method: 'POST', body: new URLSearchParams({
            action: 'set', kind: 'dashboard',
            payload: '+QSIMSTAT: 0,1\n+QGDNRCNT: ' + sent + ',' + received + '\nOK'
          })
        });
        if (!response.ok) throw new Error('mock fixture was rejected');
        const parsed = await (await fetch('/api/mock_at?action=parse&debug=1')).json();
        const dashboard = await (await fetch('/api/dashboard_data')).json();
        return {parsed, dashboard};
      }, {sent, received});
      assert.equal(result.parsed.nr_tx_bytes, sent);
      assert.equal(result.parsed.nr_rx_bytes, received);
      assert.equal(result.dashboard.nr_tx_bytes, sent);
      assert.equal(result.dashboard.nr_rx_bytes, received);
      assert(result.parsed.raw.includes('+QGDNRCNT: ' + sent + ',' + received));
      await page.waitForFunction(({download, upload}) =>
        Array.from(document.querySelectorAll('dt')).find(node => node.textContent === '累计下载')?.nextElementSibling?.textContent === download &&
        Array.from(document.querySelectorAll('dt')).find(node => node.textContent === '累计上传')?.nextElementSibling?.textContent === upload,
      {download: result.dashboard.nr_rx_human, upload: result.dashboard.nr_tx_human});
      assert.equal(await page.locator('dt').filter({hasText: /^累计下载$/}).count(), 1);
      assert.equal(await page.locator('dt').filter({hasText: /^累计上传$/}).count(), 1);
    }
    assert.deepEqual(errors, []);
    console.log('Traffic direction browser passed: raw AT, parsed counters, dashboard API and rendered download/upload totals.');
  } finally {
    if (browser) await browser.close();
    if (child.exitCode === null) {
      child.kill();
      await new Promise(resolve => child.once('exit', resolve));
    }
    fs.rmSync(temporary, {recursive: true, force: true});
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
