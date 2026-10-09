'use strict';

const $ = (id) => document.getElementById(id);
const state = { data: null, logCursor: 0, atCursor: 0, pendingShown: null, offline: 0, exited: false, inputError: null, routed: false, lastFailure: null };

// ---------- server calls ----------
async function api(path, body) {
  const options = body === undefined ? {} : { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) };
  const response = await fetch(path, { cache: 'no-store', ...options });
  let data = null;
  try { data = await response.json(); } catch (_) { /* empty body */ }
  if (!response.ok) throw new Error((data && data.error) || `请求失败（${response.status}）`);
  return data || {};
}
const post = (path, body = {}) => api(path, body);

// ---------- rendering helpers ----------
function setText(id, text) { const el = typeof id === 'string' ? $(id) : id; if (el.textContent !== text) el.textContent = text; }
function appendLog(id, payload, cursorKey, follow) {
  const el = $(id);
  if (payload.reset) el.textContent = '';
  if (payload.lines.length) {
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    el.textContent += payload.lines.join('\n') + '\n';
    if (el.textContent.length > 200000) el.textContent = el.textContent.slice(-150000);
    if (follow && (atBottom || follow === 'always')) el.scrollTop = el.scrollHeight;
  }
  state[cursorKey] = payload.next;
}
function fillSelect(select, items, valueOf, labelOf, placeholder) {
  const previous = select.value;
  const signature = JSON.stringify(items.map((item) => [valueOf(item), labelOf(item)]));
  if (select.dataset.signature !== signature) {
    select.dataset.signature = signature;
    select.replaceChildren(new Option(placeholder, ''), ...items.map((item) => new Option(labelOf(item), valueOf(item))));
  }
  const values = items.map(valueOf);
  if (values.includes(previous)) select.value = previous;
  else if (items.length === 1) select.value = values[0];
  else select.value = '';
}
const radio = (name) => document.querySelector(`input[name="${name}"]:checked`).value;

// ---------- navigation ----------
const PAGES = ['connect', 'install', 'network', 'info', 'at'];
const SERIAL_PAGES = new Set(['connect', 'network', 'info', 'at']);
function currentPage() {
  const name = location.hash.slice(1);
  return PAGES.includes(name) ? name : 'connect';
}
function showPage() {
  const page = currentPage();
  for (const name of PAGES) $('page-' + name).hidden = name !== page;
  document.querySelectorAll('.sidebar a').forEach((a) => a.classList.toggle('active', a.dataset.page === page));
  $('module-bar').hidden = !SERIAL_PAGES.has(page);
  document.title = `${$('page-' + page).dataset.title} · SimpleAdmin 设备助手`;
  window.scrollTo(0, 0);
}
function go(page) {
  if (location.hash !== '#' + page) location.hash = page; else showPage();
}
window.addEventListener('hashchange', () => { state.routed = true; showPage(); });

// ---------- install ----------
const STAGES = { device: 1, upload: 2, install: 3, verify: 4, diagnose: 2 };
const STAGE_TITLES = { device: '正在检查设备连接', upload: '正在上传安装文件', install: '正在安装并启动服务', verify: '正在验证管理页面', diagnose: '正在收集诊断信息' };
const RUNNING_DETAIL = {
  install: '请保持 USB 连接，安装期间不要断电。',
  diagnose: '只收集服务与网络状态，不读取短信、不执行 AT 指令，也不修改设备设置。',
  web: '正在检查登录页面，成功后通过 USB / ADB 打开浏览器。',
};
const STEP_LABELS = {
  install: ['检查连接', '上传文件', '安装程序', '验证网页'],
  diagnose: ['检查连接', '收集状态', '生成报告', '验证网页'],
  web: ['检查连接', '建立通道', '检查连接', '验证网页'],
};
const OPERATION_NAMES = { install: '安装 / 升级', diagnose: '故障诊断', web: '打开管理页面', serial: '串口操作' };
const deviceLabel = (d) => `${d.model ? d.model + ' · ' : ''}${d.serial}`;

function selectedDevice() {
  const serial = $('devices').value;
  return state.data && state.data.devices.find((d) => d.serial === serial);
}
const readyDevices = (data) => data.devices.filter((d) => d.state === 'device');

function renderInstall(data) {
  fillSelect($('devices'), data.devices, (d) => d.serial, deviceLabel, data.devices.length ? '请选择模块' : '未发现设备');
  const device = selectedDevice();
  const busy = !!data.busy;
  const ready = !busy && device && device.state === 'device';
  for (const id of ['install-button', 'diagnose', 'open-web']) $(id).disabled = !ready;
  $('devices').disabled = busy;
  $('refresh-devices').disabled = busy;
  for (const id of ['change-port', 'change-web', 'change-root', 'http-port', 'web-username', 'web-password', 'root-password']) $(id).disabled = busy;
  document.querySelectorAll('[data-reveal]').forEach((row) => { row.hidden = !$(row.dataset.reveal).checked; });

  let badge; let hint; let tone = '';
  if (data.devicesError) { badge = '检测失败'; hint = data.devicesError; tone = 'warn'; }
  else if (!data.devicesScanned) { badge = '正在检测'; hint = '使用 USB 连接模块，设备会自动出现在这里。'; }
  else if (!device) {
    badge = data.devices.length ? '请选择' : '未连接';
    hint = data.devices.length ? '检测到多个设备，请选择本次操作的模块。' : '未发现 ADB 设备。模块尚未开启 ADB 时，请先在“连接模块”中开启。';
  } else if (device.state === 'device') { badge = '已连接'; tone = 'ok'; hint = '操作前会再次确认是兼容模块，请勿选择手机或模拟器。'; }
  else { badge = device.state === 'unauthorized' ? '等待授权' : '离线'; tone = 'warn'; hint = device.state === 'unauthorized' ? '设备尚未授权 ADB，请完成设备端授权后刷新。' : '设备处于离线状态，请重新连接 USB 后刷新。'; }
  setText('device-badge', badge); $('device-badge').className = 'tag ' + tone;
  setText('device-hint', hint);

  const op = data.operation || 'install';
  const labels = STEP_LABELS[op] || STEP_LABELS.install;
  const running = busy && ['install', 'diagnose', 'web'].includes(data.busy);
  const outcome = running ? null : data.outcome;
  const step = STAGES[data.stage] || 1;
  const card = $('verify');
  card.className = 'run-card';
  $('web-link').hidden = true;
  [...$('stages').children].forEach((li, i) => {
    setText(li.querySelector('span'), labels[i]);
    const n = i + 1;
    li.className = running ? (n < step ? 'reached' : n === step ? 'current' : '')
      : outcome ? (outcome.ok || n < step ? 'reached' : n === step ? 'failed' : '') : '';
  });
  $('stages').hidden = $('progress').hidden = !(running || outcome);
  $('progress').className = 'progress' + (running ? ' running' : outcome ? (outcome.ok ? ' done' : ' failed') : '');
  $('progress').firstElementChild.style.width = running ? `${((step - 0.5) / 4) * 100}%` : outcome ? `${outcome.ok ? 100 : ((step - 0.5) / 4) * 100}%` : '0';

  if (state.inputError) {
    setText('status-title', state.inputError.title); setText('status-detail', state.inputError.message);
    card.classList.add('failed');
  } else if (running) {
    setText('status-title', STAGE_TITLES[data.stage] || '正在处理');
    setText('status-detail', RUNNING_DETAIL[op] || '');
  } else if (outcome && outcome.ok) {
    card.classList.add('ok');
    setText('status-title', op === 'install' ? '安装完成，管理页面已就绪' : op === 'diagnose' ? '诊断完成，网页检查通过' : '管理页面已就绪');
    let detail = outcome.reboot ? '需要手动重启模块以应用网络配置，重启后再打开管理页面。'
      : op === 'install' ? '点击下方“打开管理页面”即可使用。未开启的选项保留原值。'
      : op === 'diagnose' ? 'ADB 通道访问正常。若模块 IP 仍打不开，请查看报告中的网卡、路由和防火墙信息。'
      : '已通过 USB / ADB 建立访问通道，保持设备连接即可使用。';
    if (outcome.http_port) detail += ` 局域网地址：http://模块IP${outcome.http_port === 80 ? '/' : ':' + outcome.http_port + '/'}`;
    setText('status-detail', detail);
    if (op === 'web' && outcome.url) {
      const link = $('web-link').querySelector('a');
      link.href = outcome.url; link.textContent = `浏览器没有自动打开？访问 ${outcome.url}`;
      $('web-link').hidden = false;
    }
  } else if (outcome) {
    card.classList.add('failed');
    setText('status-title', data.stage === 'device' ? '设备检查未通过' : data.stage === 'verify' ? '网页检查未通过' : op === 'install' ? '安装未通过检查' : '诊断未完成');
    setText('status-detail', outcome.error || '请查看运行日志或诊断报告了解原因。');
    // Show the log once for each new failure.
    if (state.lastFailure !== data.log.next) { state.lastFailure = data.log.next; $('log-card').open = true; }
  } else {
    setText('status-title', ready ? '可以开始安装' : '等待连接设备');
    setText('status-detail', ready ? '首次安装和升级都使用此按钮。' : busy ? `${OPERATION_NAMES[data.busy] || '当前操作'}进行中，请稍候。` : '连接模块后选择设备，即可开始安装。');
  }
  $('view-report').disabled = !(data.outcome && data.outcome.report) || running;
  appendLog('log', data.log, 'logCursor', $('follow-log').checked);
  $('nav-install').className = 'nav-badge' + (running ? ' busy' : outcome && outcome.ok ? ' ok' : '');
}

// ---------- connect ----------
function renderConnect(data) {
  const ready = readyDevices(data);
  let title; let detail; let tone = '';
  if (data.devicesError) { title = 'ADB 检测失败'; detail = data.devicesError; tone = 'warn'; }
  else if (!data.devicesScanned) { title = '正在检测 ADB 设备…'; detail = '使用 USB 连接模块，设备会自动出现。'; }
  else if (ready.length) {
    tone = 'ready';
    title = ready.length === 1 ? `已通过 ADB 连接 ${ready[0].model || ready[0].serial}` : `已连接 ${ready.length} 台 ADB 设备`;
    detail = '无需串口操作，可以直接安装。';
  } else if (data.devices.length) {
    tone = 'warn'; title = 'ADB 设备需要处理';
    detail = data.devices[0].state === 'unauthorized' ? '设备尚未授权 ADB，请在设备端授权。' : '设备处于离线状态，请重新连接 USB。';
  } else { title = '未检测到 ADB 设备'; detail = '模块尚未开启 ADB 时，在下方通过 AT 串口开启，重启模块后会自动出现。'; }
  $('adb-hero').className = 'hero ' + tone;
  setText('hero-title', title); setText('hero-detail', detail);
  $('hero-go').hidden = !ready.length;
  $('nav-connect').className = 'nav-badge' + (ready.length ? ' ok' : '');
  $('dot-adb').className = 'dot' + (ready.length ? ' ok' : data.devices.length ? ' warn' : '');
  setText('foot-adb', ready.length ? 'ADB 已连接' : data.devices.length ? 'ADB 需处理' : 'ADB 未连接');
}

// ---------- serial ----------
const PROFILE_HINTS = {
  pcie: '转接板通过 PCIe 网卡提供网口，模块自动拨号。',
  ecm: '模块作为 USB 网卡（ECM，Linux / macOS 免驱），模块自动拨号。',
  rndis: '模块作为 USB 网卡（RNDIS，Windows 免驱），模块自动拨号。',
};
function renderSerial(data) {
  fillSelect($('ports'), data.ports, (p) => p.name, (p) => p.name + (p.description ? ' · ' + p.description : ''), data.ports.length ? '请选择 AT 串口' : '未发现串口');
  const busy = !!data.busy;
  const serialBusy = data.busy === 'serial';
  const port = $('ports').value;
  const identity = data.identity;
  const current = !!identity && identity.port === port;
  const supported = current && identity.supported;
  document.querySelectorAll('[data-needs-module]').forEach((b) => { b.disabled = busy || !supported; });
  $('identify').disabled = busy || !port;
  $('scan-ports').disabled = busy;
  $('ports').disabled = busy;
  for (const id of ['at-preset', 'custom-at', 'save-preset', 'load-preset']) $(id).disabled = busy;
  document.querySelectorAll('input[name="eth-profile"], input[name="eth-driver"]').forEach((input) => { input.disabled = busy; });
  $('stop-at').hidden = !serialBusy;
  $('at-progress').hidden = !serialBusy;

  // Module context bar.
  $('module-bar').classList.toggle('ready', supported);
  if (current) {
    setText('module-name', identity.model || '未知型号');
    setText('module-sub', `${identity.port} · ${identity.firmware || '固件未知'}`);
  } else {
    setText('module-name', '未识别模块');
    setText('module-sub', port ? `${port} · 尚未识别` : '在“连接模块”中选择串口并识别');
  }
  $('dot-serial').className = 'dot' + (serialBusy ? ' busy' : supported ? ' ok' : current ? ' warn' : '');
  setText('foot-serial', current ? `${identity.model} · ${identity.port}` : '串口未识别');

  const status = identity && !current && !busy ? { title: '已切换串口', message: '请重新识别模块。', severity: 'info' }
    : data.prepare || (data.portsError ? { title: '串口列表读取失败', message: data.portsError, severity: 'error' } : null);
  const el = $('prepare-status');
  el.hidden = !status;
  if (status) {
    el.className = 'status ' + status.severity;
    setText(el.querySelector('strong'), status.title);
    setText(el.querySelector('span'), status.message);
    el.title = `${status.title}：${status.message}`;
  }

  $('identity').hidden = !current;
  if (current) for (const field of $('identity').querySelectorAll('[data-field]')) setText(field, identity[field.dataset.field] || '-');

  const profile = radio('eth-profile');
  $('eth-driver-group').hidden = profile !== 'pcie';
  setText('eth-profile-hint', PROFILE_HINTS[profile]);

  const info = $('info');
  const signature = JSON.stringify(data.info);
  if (info.dataset.signature !== signature) {
    info.dataset.signature = signature;
    info.replaceChildren(...data.info.map((item) => {
      const card = document.createElement('div');
      if (/固件未提供/.test(item.value)) card.className = 'missing';
      const label = document.createElement('strong'); label.textContent = item.label;
      const value = document.createElement('pre'); value.textContent = item.value;
      card.append(label, value);
      return card;
    }));
  }
  $('info-empty').hidden = data.info.length > 0;
  appendLog('at-log', data.atLog, 'atCursor', 'always');
  renderPending(data.pending, identity);
}

function renderPending(pending, identity) {
  const dialog = $('confirm');
  if (!pending) { if (dialog.open) dialog.close(); state.pendingShown = null; return; }
  if (state.pendingShown === pending.id) return;
  state.pendingShown = pending.id;
  setText('confirm-title', pending.title);
  setText('confirm-module', identity ? `${identity.model} · ${identity.port}` : '');
  setText('confirm-description', pending.description);
  setText('confirm-commands', pending.commands.join('\n'));
  $('confirm-agree-row').hidden = !pending.destructive;
  $('confirm-agree').checked = false;
  $('confirm-ok').textContent = pending.destructive ? '恢复出厂' : '确认执行';
  $('confirm-ok').className = pending.destructive ? 'prominent destructive' : 'prominent';
  $('confirm-ok').disabled = pending.destructive;
  if (!dialog.open) dialog.showModal();
  $('confirm-cancel').focus();
}

function render(data) {
  state.data = data;
  setText('version', `v${data.version}`);
  // On first open, start where the user can make progress: install when ADB is already there.
  if (!state.routed && data.devicesScanned) {
    state.routed = true;
    if (readyDevices(data).length) go('install');
  }
  renderSerial(data);
  renderConnect(data);
  renderInstall(data);
}

// ---------- polling ----------
let pollTimer = null;
async function poll() {
  clearTimeout(pollTimer);
  if (state.exited) return;
  try {
    render(await api(`/api/state?log=${state.logCursor}&at=${state.atCursor}`));
    state.offline = 0;
  } catch (error) {
    if (++state.offline >= 3) return showOverlay('设备助手已关闭', '与设备助手的连接已断开。需要时重新双击 SimpleAdmin-Setup.exe。');
  }
  pollTimer = setTimeout(poll, document.hidden ? 5000 : (state.data && state.data.busy ? 500 : 1000));
}
let deviceTimer = null;
function scheduleDevices() {
  clearTimeout(deviceTimer);
  deviceTimer = setTimeout(async () => {
    if (!document.hidden && state.data && !state.data.busy) await post('/api/devices').catch(() => {});
    scheduleDevices();
  }, 4000);
}
function showOverlay(title, text) {
  state.exited = true;
  clearTimeout(pollTimer); clearTimeout(deviceTimer);
  setText('overlay-title', title); setText('overlay-text', text);
  $('overlay').hidden = false;
}

// ---------- actions ----------
async function action(fn) {
  try { await fn(); } catch (error) { flashSerial('操作未完成', error.message, 'warning'); }
  poll();
}
function flashSerial(title, message, severity) {
  if (!state.data) return;
  state.data.prepare = { title, message, severity };
  renderSerial(state.data);
}
async function spin(button, fn) {
  button.classList.add('spinning');
  try { return await fn(); } finally { setTimeout(() => button.classList.remove('spinning'), 400); }
}
const port = () => $('ports').value;

$('scan-ports').addEventListener('click', () => action(() => spin($('scan-ports'), () => post('/api/ports'))));
$('identify').addEventListener('click', () => action(() => post('/api/at/identify', { port: port() })));
$('unlock-adb').addEventListener('click', () => action(() => post('/api/at/preview', { port: port(), action: 'adb' })));
$('restart').addEventListener('click', () => action(() => post('/api/at/preview', { port: port(), action: 'restart' })));
$('factory').addEventListener('click', () => action(() => post('/api/at/preview', { port: port(), action: 'factory' })));
$('run-custom').addEventListener('click', () => action(() => post('/api/at/preview', { port: port(), action: 'custom', commands: $('custom-at').value })));
for (const [id, enable] of [['eth-enable', true], ['eth-disable', false]]) {
  $(id).addEventListener('click', () => action(() => {
    const profile = radio('eth-profile');
    return post('/api/at/preview', { port: port(), action: 'ethernet', enable, profile, driver: profile === 'pcie' ? radio('eth-driver') : null });
  }));
}
document.querySelectorAll('input[name="eth-profile"]').forEach((input) => input.addEventListener('change', () => state.data && renderSerial(state.data)));
$('read-info').addEventListener('click', () => action(() => post('/api/at/info', { port: port() })));
$('stop-at').addEventListener('click', () => action(() => post('/api/at/stop')));
$('save-at-log').addEventListener('click', () => action(() => post('/api/at/log')));
$('save-preset').addEventListener('click', () => action(() => post('/api/at/preset', { text: $('custom-at').value })));
$('load-preset').addEventListener('click', () => action(async () => { $('custom-at').value = (await api('/api/at/preset')).text; }));
$('at-preset').addEventListener('change', () => { if ($('at-preset').value) $('custom-at').value = $('at-preset').value; $('at-preset').value = ''; });
$('ports').addEventListener('change', () => state.data && renderSerial(state.data));

$('confirm-agree').addEventListener('change', () => { $('confirm-ok').disabled = !$('confirm-agree').checked; });
$('confirm-cancel').addEventListener('click', () => { $('confirm').close(); action(() => post('/api/at/dismiss')); });
$('confirm').addEventListener('cancel', () => action(() => post('/api/at/dismiss')));
$('confirm-ok').addEventListener('click', () => {
  const id = state.pendingShown;
  $('confirm').close();
  action(() => post('/api/at/confirm', { id }));
});

$('hero-go').addEventListener('click', () => go('install'));
$('refresh-devices').addEventListener('click', () => spin($('refresh-devices'), () => post('/api/devices').catch(() => {})).then(poll));
$('devices').addEventListener('change', () => { state.inputError = null; state.data && renderInstall(state.data); });
for (const id of ['change-port', 'change-web', 'change-root']) {
  $(id).addEventListener('change', () => {
    state.inputError = null;
    if (state.data) renderInstall(state.data);
    const field = document.querySelector(`[data-reveal="${id}"] input`);
    if ($(id).checked && field) field.focus();
  });
}

async function runOperation(operation) {
  const device = selectedDevice();
  if (!device) return;
  state.inputError = null;
  const body = { operation, serial: device.serial };
  if (operation === 'install') {
    if ($('change-port').checked) {
      const value = $('http-port').value.trim();
      if (!/^[1-9][0-9]{0,4}$/.test(value) || Number(value) > 65535) {
        state.inputError = { title: '请检查 HTTP 端口', message: '请输入 1–65535 的整数，例如 80 或 8080。尚未修改设备。' };
        renderInstall(state.data); $('http-port').focus(); return;
      }
      body.http_port = value;
    }
    if ($('change-web').checked) { body.web_username = $('web-username').value; body.web_password = $('web-password').value; }
    if ($('change-root').checked) body.root_password = $('root-password').value;
  }
  try {
    $('log').textContent = '';
    await post('/api/run', body);
    $('verify').scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  } catch (error) {
    state.inputError = { title: /密码|账号/.test(error.message) ? '请检查账号密码' : /端口|65535/.test(error.message) ? '请检查 HTTP 端口' : '无法开始', message: error.message };
    renderInstall(state.data);
  }
  poll();
}
$('install-button').addEventListener('click', () => runOperation('install'));
$('diagnose').addEventListener('click', () => runOperation('diagnose'));
$('open-web').addEventListener('click', () => runOperation('web'));

$('view-report').addEventListener('click', async () => {
  try {
    const report = await api('/api/report');
    setText('report-path', report.path);
    setText('report-text', report.text);
    $('report').showModal();
  } catch (error) { state.inputError = { title: '无法查看报告', message: error.message }; renderInstall(state.data); }
});
$('report-notepad').addEventListener('click', () => post('/api/open', { target: 'report' }).catch(() => {}));
$('open-reports').addEventListener('click', () => post('/api/open', { target: 'reports' }).catch(() => {}));

$('quit').addEventListener('click', () => { $('ask').returnValue = ''; $('ask').showModal(); });
$('ask').addEventListener('close', async () => {
  if ($('ask').returnValue !== 'ok') return;
  try {
    await post('/api/quit');
    showOverlay('设备助手已退出', '可以关闭此页面。需要时重新双击 SimpleAdmin-Setup.exe。');
  } catch (error) {
    state.inputError = { title: '无法退出', message: error.message };
    go('install');
    renderInstall(state.data);
  }
});

// Tell the assistant the page is going away so it can exit once nothing is running.
window.addEventListener('pagehide', () => { if (!state.exited) fetch('/api/bye', { method: 'POST', keepalive: true }).catch(() => {}); });
document.addEventListener('visibilitychange', () => { if (!document.hidden) poll(); });

if (location.hash) state.routed = true;
showPage();
poll();
post('/api/devices').catch(() => {}).finally(poll);
post('/api/ports').catch(() => {}).finally(poll);
scheduleDevices();
