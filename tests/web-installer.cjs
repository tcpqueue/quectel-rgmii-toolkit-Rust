// End-to-end test of the web installer UI with a scripted adb and a simulated Quectel AT port.
// Runs on Linux: cargo build -p simpleadmin-installer && node tests/web-installer.cjs
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const net = require('node:net');
const readline = require('node:readline');
const {spawn} = require('node:child_process');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

const root = path.resolve(__dirname, '..');
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-web-installer-'));
const shots = process.env.SCREENSHOT_DIR || scratch;
const payload = path.join(scratch, 'payload');
fs.mkdirSync(path.join(payload, 'development'), {recursive: true});
fs.writeFileSync(path.join(payload, 'development/SHA256SUMS'), 'fixture');
fs.writeFileSync(path.join(payload, 'development/diagnose_simpleadmin.sh'), 'echo diagnose');
const write = (name, value) => fs.writeFileSync(path.join(scratch, name), String(value));
const read = name => fs.readFileSync(path.join(scratch, name), 'utf8');

// adb behaviour follows the "mode" file so each scenario can change it.
fs.writeFileSync(path.join(payload, 'adb'), `#!/bin/bash
dir="${scratch}"; mode="$(cat "$dir/mode")"; call="$*"
printf '%s\\n' "$call" >> "$dir/calls"
case "$call" in
  *"cat > /tmp/development/install-credentials.json"*) cat > "$dir/credentials"; exit 0;;
  *"sha256sum /tmp/development/install-credentials.json"*) echo "$(sha256sum < "$dir/credentials" | cut -d' ' -f1)  x"; exit 0;;
  *SIMPLEADMIN_PREFLIGHT=1*)
    printf 'SIMPLEADMIN_PREFLIGHT=1\\nSA_SYSTEM=Linux\\nSA_ARCH=armv7l\\nSA_UID=0\\nSA_BASH=1\\nSA_TMP=1\\n'
    if [ "$mode" = phone ]; then echo SA_MODULE=0; else echo SA_MODULE=1; fi
    echo SIMPLEADMIN_PREFLIGHT_DONE=1; exit 0;;
  "devices -l") printf 'List of devices attached\\nFAKE1  device usb:1-1 product:sdxlemur model:RM520N_EU device:sdxlemur\\n'; exit 0;;
  devices) printf 'List of devices attached\\nFAKE1\\tdevice\\n'; exit 0;;
  *"bash /tmp/development/install_simpleadmin_rust.sh"*)
    echo "[1/6] 校验安装文件"; sleep 1; echo "[6/6] 启动服务"
    if [ "$mode" = install-failed ]; then echo "simulated installation failure" >&2; exit 1; fi; exit 0;;
  *"cat /tmp/simpleadmin-install-result.env"*) echo INSTALL_STATUS=OK; exit 0;;
  *"/usrdata/simpleadmin/http_port"*) echo 80; exit 0;;
  *"forward tcp:0 tcp:80"*) cat "$dir/web-port"; echo; exit 0;;
esac
exit 0
`, {mode: 0o755});
write('mode', 'success');
write('calls', '');

async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const {port} = server.address();
  await new Promise(resolve => server.close(resolve));
  return port;
}

(async () => {
  const web = http.createServer((request, response) => {
    if (request.url === '/') { response.writeHead(303, {Location: '/login.html'}); return response.end(); }
    response.end(request.url.includes('locales') ? 'root.Lang' : 'loginLanguage');
  });
  await new Promise(resolve => web.listen(0, '127.0.0.1', resolve));
  write('web-port', web.address().port);

  const modem = spawn('python3', [path.join(root, 'tests/fixtures/fake-quectel-modem.py')]);
  const tty = await new Promise(resolve => readline.createInterface({input: modem.stdout}).once('line', resolve));

  const port = await freePort();
  const opened = path.join(scratch, 'opened');
  const installer = spawn(path.join(root, 'target/debug/SimpleAdmin-Setup'), [
    '--payload-dir', payload, '--data-dir', path.join(scratch, 'data'), '--port', String(port), '--no-browser'
  ], {env: {...process.env, SIMPLEADMIN_NO_OPEN: opened, SIMPLEADMIN_TEST_PORTS: tty}, stdio: ['ignore', 'pipe', 'inherit']});
  const launch = await new Promise(resolve => readline.createInterface({input: installer.stdout}).once('line', line => resolve(line.match(/http:\S+/)[0])));
  const exited = new Promise(resolve => installer.once('exit', resolve));

  const browser = await chromium.launch({headless: true});
  const errors = [];
  try {
    const context = await browser.newContext({viewport: {width: 1280, height: 1000}});
    const page = await context.newPage();
    page.on('pageerror', error => errors.push(error.message));
    // Rejected inputs answer 400 on purpose; the browser logs those as resource errors.
    page.on('console', message => { if (message.type() === 'error' && !message.text().startsWith('Failed to load resource')) errors.push(message.text()); });
    await page.goto(launch);
    assert.equal(new URL(page.url()).search, '', 'launch token must leave the address bar');
    const cookies = await context.cookies();
    assert(cookies.some(c => c.name === 'sa_setup' && c.httpOnly && c.sameSite === 'Strict'));


    // ---------- serial preparation ----------
    await page.locator('#ports option', {hasText: tty}).waitFor({state: 'attached'});
    await page.selectOption('#ports', tty);
    await page.click('#identify');
    await page.locator('#prepare-status strong', {hasText: '已识别移远高通模块'}).waitFor();
    assert.equal(await page.textContent('#identity [data-field=model]'), 'RM520N-EU');
    assert.equal(await page.textContent('#identity [data-field=imei]'), '•••••••••••0123');

    await page.click('#unlock-adb');
    await page.locator('#confirm[open]').waitFor();
    assert.equal(await page.textContent('#confirm-commands'), 'AT+QCFG="usbcfg",0x2C7C,0x0801,1,1,1,1,1,2,0');
    await page.screenshot({path: path.join(shots, 'installer-confirm.png')});
    await page.click('#confirm-ok');
    await page.locator('#prepare-status strong', {hasText: 'ADB 配置已验证'}).waitFor();
    await page.click('#unlock-adb');
    await page.locator('#prepare-status strong', {hasText: 'ADB 接口已开启'}).waitFor();

    await page.click('#at-tools summary');
    await page.fill('#custom-at', 'AT+QTEMP\nAT+CSQ');
    await page.click('#run-custom');
    await page.locator('#confirm[open]').waitFor();
    await page.click('#confirm-ok');
    await page.locator('#prepare-status strong', {hasText: '指令已被模块接受'}).waitFor();
    assert.match(await page.textContent('#at-log'), /已确认：AT\+CSQ/);
    await page.fill('#custom-at', 'AT+CMGS="10086"');
    await page.click('#run-custom');
    await page.locator('#prepare-status strong', {hasText: '请检查指令'}).waitFor();

    await page.click('#factory');
    await page.locator('#confirm[open]').waitFor();
    assert.equal(await page.isDisabled('#confirm-ok'), true, 'factory reset needs the extra confirmation');
    await page.click('#confirm-cancel');
    await page.locator('#prepare-status strong', {hasText: '已取消'}).waitFor();

    await page.click('#eth-enable');
    await page.locator('#confirm[open]').waitFor();
    assert.match(await page.textContent('#confirm-commands'), /AT\+QETH="eth_driver","r8125",1\nAT\+QMAPWAC=1$/);
    await page.click('#confirm-cancel');

    await page.click('#info-card summary');
    await page.click('#read-info');
    await page.locator('#prepare-status strong', {hasText: '设备信息读取完成'}).waitFor();
    assert.equal(await page.locator('#info > div').count(), 31);
    assert.match(await page.textContent('#info'), /固件未提供/);

    // ---------- installation ----------
    await page.locator('#devices option', {hasText: 'RM520N EU'}).waitFor({state: 'attached'});
    await page.locator('#device-badge', {hasText: '已连接'}).waitFor();
    await page.click('#change-port');
    await page.fill('#http-port', '65536');
    await page.click('#install-button');
    await page.locator('#status-title', {hasText: '请检查 HTTP 端口'}).waitFor();
    await page.fill('#http-port', '80');
    await page.click('#change-port');
    await page.click('#change-web');
    await page.fill('#web-username', 'bad:name');
    await page.click('#install-button');
    await page.locator('#status-title', {hasText: '请检查账号密码'}).waitFor();
    assert.equal(read('calls').includes('push'), false, 'invalid input must not touch the device');

    await page.fill('#web-username', 'owner');
    await page.fill('#web-password', 'web-secret:"$value');
    await page.click('#install-button');
    await page.locator('#status-title', {hasText: '正在'}).waitFor();
    assert.equal(await page.isDisabled('#install-button'), true);
    await page.screenshot({path: path.join(shots, 'installer-running.png')});
    await page.locator('#status-title', {hasText: '安装完成，管理页面已就绪'}).waitFor({timeout: 30000});
    assert.match(await page.textContent('#log'), /\[6\/6\] 启动服务/);
    assert.equal(JSON.parse(read('credentials')).web_password, 'web-secret:"$value');
    assert.equal(read('calls').includes('web-secret'), false);
    await page.screenshot({path: path.join(shots, 'installer-success.png'), fullPage: true});
    if (process.env.CAPTURE_DOCS) {
      // README images: show a Windows-style port name instead of the test pseudo-terminal.
      await page.evaluate(() => { for (const option of document.querySelectorAll('#ports option')) option.textContent = option.textContent.replace(/^\/dev\/pts\/\d+/, 'COM8'); });
      const images = path.join(root, 'docs/images');
      await page.setViewportSize({width: 1280, height: 900});
      for (const [scheme, suffix] of [['light', ''], ['dark', '-dark']]) {
        await page.emulateMedia({colorScheme: scheme});
        await page.evaluate(() => document.getElementById('install').scrollIntoView());
        await page.waitForTimeout(300);
        await page.screenshot({path: path.join(images, `windows-installer${suffix}.png`)});
        assert.equal(await page.textContent('.steps a.active'), '04安装升级');
      }
      await page.emulateMedia({colorScheme: 'light'});
      await page.evaluate(() => window.scrollTo(0, 0));
      await page.waitForTimeout(300);
      await page.screenshot({path: path.join(images, 'windows-prepare.png')});
      assert.equal(await page.textContent('.steps a.active'), '01识别模块');
      await page.setViewportSize({width: 1280, height: 1000});
    }

    await page.click('#view-report');
    await page.locator('#report[open]').waitFor();
    assert.match(await page.textContent('#report-text'), /PC_HTTP_CHECK=OK/);
    assert.equal((await page.textContent('#report-text')).includes('web-secret'), false);
    await page.click('#report button[type=submit]');

    await page.click('#open-web');
    await page.locator('#status-title', {hasText: /^管理页面已就绪$/}).waitFor();
    assert.match(fs.readFileSync(opened, 'utf8'), /http:\/\/127\.0\.0\.1:\d+\//);

    write('mode', 'install-failed');
    await page.click('#change-web');
    await page.click('#install-button');
    await page.locator('#status-title', {hasText: '安装未通过检查'}).waitFor({timeout: 30000});
    assert.match(await page.textContent('#status-detail'), /安装未通过检查/);
    write('mode', 'phone');
    await page.click('#diagnose');
    await page.locator('#status-title', {hasText: '设备检查未通过'}).waitFor();

    // ---------- layout ----------
    for (const [name, width, scheme] of [['installer-mobile.png', 390, 'light'], ['installer-dark.png', 1280, 'dark']]) {
      await page.setViewportSize({width, height: 900});
      await page.emulateMedia({colorScheme: scheme});
      await page.evaluate(() => window.scrollTo(0, 0));
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      assert(overflow <= 0, `${name}: horizontal overflow ${overflow}px`);
      await page.screenshot({path: path.join(shots, name)});
    }
    assert.deepEqual(errors, []);

    // Closing the page ends the assistant once the grace period passes.
    await page.close();
    const code = await Promise.race([exited, new Promise(resolve => setTimeout(() => resolve('timeout'), 30000))]);
    assert.equal(code, 0, 'assistant must exit after its page closes');
    console.log('Web installer passed: token cookie, serial identify/unlock/custom/factory/network/info, install success and failure, validation, credentials, report, open web, layouts and auto-exit. Screenshots: ' + shots);
  } finally {
    await browser.close();
    installer.kill();
    modem.kill();
    web.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
