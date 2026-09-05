const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const {spawn} = require('node:child_process');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

(async () => {
  const root = path.resolve(__dirname, '..');
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-rust-test-'));
  const base = process.env.DEVICE_URL || 'http://127.0.0.1:18082';
  const docs = process.env.CAPTURE_DOCS && !process.env.DEVICE_URL ? path.join(root,'docs/images') : null;
  if (docs) fs.mkdirSync(docs,{recursive:true});
  const child = process.env.DEVICE_URL ? null : spawn(path.join(root, 'target/debug/simpleadmin-httpd'), ['--mock', '--http', '127.0.0.1:18082', '--static', path.join(root, 'development/simpleadmin/www'), '--auth-file', path.join(temporary, 'auth'), '--ttl-file', path.join(temporary, 'ttl')], {stdio: ['ignore', 'pipe', 'pipe']});
  let logs = '';
  if (child) {child.stdout.on('data', b => logs += b); child.stderr.on('data', b => logs += b);}
  let browser;
  try {
    for (let i = 0; i < 100; i++) {try {await fetch(base + '/login.html'); break;} catch {await new Promise(r => setTimeout(r, 100));}}
    browser = await chromium.launch({headless:true});
    for (const language of ['zh-CN','en','ru','ar']) {
      const context = await browser.newContext({viewport:{width:1440,height:1100},locale:language});
      const page = await context.newPage(); const errors = []; page.on('pageerror', e => errors.push(e.message));
      await page.goto(base + '/login.html');
      await page.locator('#loginLanguage').selectOption(language);
      if (docs && language==='zh-CN') await page.screenshot({path:path.join(docs,'login.png')});
      await page.locator('#username').fill('admin'); await page.locator('#password').fill('admin');
      await page.locator('#loginButton').click(); await page.waitForURL(base + '/');
      await page.waitForFunction(() => document.querySelectorAll('#monitorApp canvas').length === 3);
      await page.waitForTimeout(1500);
      assert.equal(await page.locator('html').getAttribute('lang'), language);
      assert.equal(await page.locator('html').getAttribute('dir'), language === 'ar' ? 'rtl' : 'ltr');
      const api = await page.evaluate(async () => {
        const paths = ['/api/dashboard_data','/api/device_info_data','/api/network_data','/api/settings_data','/api/sms_data','/api/telemetry'];
        const values = {};
        for (const path of paths) {const r = await SimpleAdmin.Api.request(path); values[path] = {status:r.status,data:await r.json()};}
        return values;
      });
      for (const [endpoint,result] of Object.entries(api)) assert.equal(result.status,200, endpoint);
      assert.equal(api['/api/device_info_data'].data.modelName,process.env.DEVICE_URL ? 'RM520N-EU' : 'RG520N-EB');
      assert(api['/api/telemetry'].data.ping.length > 0);
      assert(api['/api/telemetry'].data.signal.length > 0);
      for (const name of ['deviceinfo','network','sms','settings','index']) {
        const link = page.locator(`.sa-menu-link[data-page-link="${name}"]`);
        if (await link.count()) {await link.click(); await page.waitForTimeout(200);}
      }
      await page.locator('.sa-menu-link[data-page-link="settings"]').click();
      await page.reload(); await page.waitForTimeout(500);
      const overview = page.locator('.sa-menu-link').first(); await overview.click();
      await page.waitForFunction(() => document.querySelector('.art-update-label b')?.textContent.length > 0);
      const updated = await page.locator('.art-update-label b').textContent();
      await page.waitForTimeout(6000);
      assert.notEqual(await page.locator('.art-update-label b').textContent(), updated, 'overview must repaint after navigation');
      const history = await page.evaluate(async () => (await fetch('/api/telemetry')).json());
      assert(history.ping.at(-1).time > api['/api/telemetry'].data.ping.at(-1).time);
      assert(history.ping.length <= 300 && history.signal.length <= 60);
      for (const width of [1440,390,320]) {
        await page.setViewportSize({width,height:1100});
        await page.waitForTimeout(350);
        if (width < 992) {
          const sidebar = await page.locator('.sa-sidebar').boundingBox();
          assert(sidebar.x+sidebar.width<=1 || sidebar.x>=width-1, `${language}: mobile sidebar must be offscreen`);
        }
        await page.screenshot({path:path.join(temporary,`${language}-${width}.png`),fullPage:true});
        if (docs && language==='zh-CN' && width===1440) {
          await page.screenshot({path:path.join(docs,'overview.png')});
          await page.locator('#monitorApp').screenshot({path:path.join(docs,'monitoring.png')});
        }
        if (docs && width===390 && ['zh-CN','ar'].includes(language)) {
          await page.evaluate(()=>scrollTo(0,0));
          await page.screenshot({path:path.join(docs,language==='ar'?'arabic-mobile.png':'mobile.png')});
        }
        const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1);
        assert(!overflow, `${language} ${width}: horizontal overflow`);
        const painted = await page.locator('#monitorApp canvas').evaluateAll(canvases => canvases.map(canvas => {
          const pixels = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
          let colored = 0;
          for (let i = 0; i < pixels.length; i += 4) if (pixels[i + 3] && Math.max(...pixels.slice(i,i+3))-Math.min(...pixels.slice(i,i+3)) > 40) colored++;
          return colored;
        }));
        assert(painted.every(count => count > 100), `${language} ${width}: chart pixels missing`);
      }
      assert.deepEqual(errors,[]);
      await context.close();
      console.log(JSON.stringify({language,api:'passed',navigation:'passed',viewports:[1440,390,320]}));
    }
    console.log(JSON.stringify({screenshots:temporary,logs}));
  } finally {
    if (browser) await browser.close();
    if (child) {child.kill(); await new Promise(resolve => child.once('exit',resolve));}
  }
})().catch(error => {console.error(error);process.exitCode=1;});
