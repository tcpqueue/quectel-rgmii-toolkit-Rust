'use strict';

const $ = (id) => document.getElementById(id);
const state = { data: null, logCursor: 0, atCursor: 0, pendingShown: null, lastOutcome: null, offline: 0, exited: false, inputError: null };

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
function setText(id, text) { const el = $(id); if (el.textContent !== text) el.textContent = text; }
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

const STAGES = { device: 1, upload: 2, install: 3, verify: 4, diagnose: 2 };
const STAGE_TITLES = { device: '正在检查设备连接', upload: '正在上传安装文件', install: '正在安装并启动服务', verify: '正在验证管理页面', diagnose: '正在收集诊断信息' };
const RUNNING_DETAIL = {
  install: '请保持 USB 连接，安装期间不要断电。完成后会验证实际网页是否可用。',
  diagnose: '只收集服务与网络状态，不读取短信、不执行 AT 指令，也不修改设备设置。',
  web: '正在检查公开登录页面，成功后通过 USB / ADB 打开浏览器。',
};
const STEP_LABELS = {
  install: ['① 检查连接', '② 上传文件', '③ 安装程序', '④ 验证网页'],
  diagnose: ['① 检查连接', '② 收集状态', '③ 生成报告', '④ 验证网页'],
  web: ['① 检查连接', '② 建立通道', '③ 检查连接', '④ 验证网页'],
};
const deviceLabel = (d) => `${d.model ? d.model + '  ·  ' : ''}${d.serial}  —  ${d.state === 'device' ? '已连接' : d.state === 'unauthorized' ? '等待授权' : '离线'}`;
const busyName = { install: '安装 / 升级', diagnose: '故障诊断', web: '打开管理页面', serial: '串口操作' };

function selectedDevice() {
  const serial = $('devices').value;
  return state.data && state.data.devices.find((d) => d.serial === serial);
}

function renderInstall(data) {
  fillSelect($('devices'), data.devices, (d) => d.serial, deviceLabel, data.devices.length ? '请选择模块' : '未发现设备');
  const device = selectedDevice();
  const busy = !!data.busy;
  const ready = !busy && device && device.state === 'device';
  for (const id of ['install-button', 'diagnose', 'open-web']) $(id).disabled = !ready;
  $('devices').disabled = busy;
  $('refresh-devices').disabled = busy;
  for (const id of ['change-port', 'change-web', 'change-root']) $(id).disabled = busy;
  $('http-port').disabled = busy || !$('change-port').checked;
  $('web-username').disabled = $('web-password').disabled = busy || !$('change-web').checked;
  $('root-password').disabled = busy || !$('change-root').checked;

  const badge = $('device-badge');
  let badgeText; let hint; let tone = '';
  if (data.devicesError) { badgeText = '检测失败'; hint = data.devicesError; tone = 'warn'; }
  else if (!data.devicesScanned) { badgeText = '正在检测'; hint = '使用 USB 连接模块，设备会自动出现在这里。'; }
  else if (!device) {
    badgeText = data.devices.length ? '请选择设备' : '未连接';
    hint = data.devices.length ? '检测到多个设备，请明确选择本次操作的模块。' : '未发现设备，请检查 USB 连接和 ADB 驱动，然后点击刷新。没有 ADB 时，先在上方“连接与准备”中开启。';
  } else if (device.state === 'device') { badgeText = '● 已连接'; tone = 'ok'; hint = 'ADB 已连接；操作前会检查是否为兼容模块，请勿选择手机或模拟器。'; }
  else { badgeText = '需要处理'; tone = 'warn'; hint = device.state === 'unauthorized' ? '设备尚未授权 ADB，请完成设备端授权后刷新。' : '设备处于离线状态，请重新连接 USB 后刷新。'; }
  setText('device-badge', badgeText); badge.className = 'badge ' + tone;
  setText('device-hint', hint);

  const op = data.operation || 'install';
  const labels = STEP_LABELS[op] || STEP_LABELS.install;
  const running = busy && ['install', 'diagnose', 'web'].includes(data.busy);
  const outcome = running ? null : data.outcome;
  const reached = running ? (STAGES[data.stage] || 1) : outcome ? (outcome.ok ? 4 : 0) : 0;
  [...$('stages').children].forEach((li, i) => { li.textContent = labels[i]; li.classList.toggle('reached', i < reached); });
  const progress = $('progress');
  progress.className = 'progress' + (running ? ' running' : outcome ? (outcome.ok ? ' done' : ' failed') : '');
  const title = $('status-title');
  title.className = '';
  $('web-link').hidden = true;
  if (state.inputError) {
    setText('status-title', state.inputError.title); setText('progress-label', '尚未修改设备'); setText('status-detail', state.inputError.message);
    title.className = 'failed';
  } else if (running) {
    setText('status-title', STAGE_TITLES[data.stage] || '正在处理');
    setText('progress-label', op === 'install' ? `第 ${STAGES[data.stage] || 1} / 4 步` : '正在处理');
    setText('status-detail', RUNNING_DETAIL[op] || '');
  } else if (outcome) {
    if (outcome.ok) {
      title.className = 'ok';
      setText('status-title', op === 'install' ? '安装完成，管理页面已就绪' : op === 'diagnose' ? '诊断完成，网页检查通过' : '管理页面已就绪');
      setText('progress-label', '全部检查通过');
      let detail = outcome.reboot ? '需要手动重启模块以应用网络配置。重启后请重新打开管理页面。'
        : op === 'install' ? '程序与网页检查通过。点击“打开管理页面”即可使用，账号密码按本次选择保存；未勾选的项目保留原值。'
        : op === 'diagnose' ? 'ADB 通道访问正常。如果模块 IP 仍打不开，请查看报告中的网卡、路由和防火墙信息。'
        : '已通过 USB / ADB 建立访问通道。保持设备连接，即可在浏览器中使用。';
      if (outcome.http_port) detail += ` 局域网地址：http://模块IP${outcome.http_port === 80 ? '/' : ':' + outcome.http_port + '/'}。`;
      setText('status-detail', detail);
      if (op === 'web' && outcome.url) {
        const link = $('web-link').querySelector('a');
        link.href = outcome.url; link.textContent = `浏览器没有自动打开？点击访问 ${outcome.url}`;
        $('web-link').hidden = false;
      }
    } else {
      title.className = 'failed';
      setText('status-title', data.stage === 'device' ? '设备检查未通过' : data.stage === 'verify' ? '网页检查未通过' : op === 'install' ? '安装未通过检查' : '诊断未完成');
      setText('progress-label', '需要处理');
      setText('status-detail', outcome.error || '请查看实时日志或点击“查看报告”了解失败原因。');
    }
  } else {
    setText('status-title', ready ? '可以开始安装' : '等待连接设备');
    setText('progress-label', ready ? '准备就绪' : '准备就绪后即可开始');
    setText('status-detail', ready ? '也可以先运行故障诊断，或直接打开已安装的管理页面。' : busy ? `${busyName[data.busy] || '当前操作'}进行中，请稍候。` : '连接模块后选择设备，安装按钮会自动启用。');
  }
  $('view-report').disabled = !(data.outcome && data.outcome.report) || running;
  appendLog('log', data.log, 'logCursor', $('follow-log').checked);
}

function renderPrepare(data) {
  fillSelect($('ports'), data.ports, (p) => p.name, (p) => p.name + (p.description ? ' · ' + p.description : ''), data.ports.length ? '请选择 AT 串口' : '未发现串口');
  const busy = !!data.busy;
  const port = $('ports').value;
  const identity = data.identity;
  const moduleReady = !busy && identity && identity.supported && identity.port === port;
  document.querySelectorAll('[data-needs-module]').forEach((b) => { b.disabled = !moduleReady; });
  $('identify').disabled = busy || !port;
  $('scan-ports').disabled = busy;
  $('ports').disabled = busy;
  for (const id of ['eth-profile', 'at-preset', 'custom-at', 'save-preset', 'load-preset']) $(id).disabled = busy;
  $('eth-driver').disabled = busy || $('eth-profile').value !== 'pcie';
  $('stop-at').disabled = data.busy !== 'serial';
  $('at-progress').hidden = data.busy !== 'serial';

  const status = data.prepare || (data.portsError ? { title: '串口列表读取失败', message: data.portsError, severity: 'error' } : { title: '尚未识别', message: '连接模块后刷新串口，选择端口并点击识别。', severity: 'info' });
  if (identity && identity.port !== port && !busy) Object.assign(status, { title: '已切换串口', message: '请重新识别模块。', severity: 'info' });
  const banner = $('prepare-status');
  banner.className = 'banner ' + status.severity;
  banner.querySelector('strong').textContent = status.title;
  banner.querySelector('span').textContent = status.message;

  $('identity').hidden = !identity;
  if (identity) for (const el of $('identity').querySelectorAll('dd')) el.textContent = identity[el.dataset.field] || '-';

  const info = $('info');
  const signature = JSON.stringify(data.info);
  if (info.dataset.signature !== signature) {
    info.dataset.signature = signature;
    info.replaceChildren(...data.info.map((item) => {
      const card = document.createElement('div');
      const label = document.createElement('strong'); label.textContent = item.label;
      const value = document.createElement('pre'); value.textContent = item.value;
      card.append(label, value);
      return card;
    }));
  }
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
  $('confirm-ok').className = pending.destructive ? 'danger' : 'primary';
  $('confirm-ok').disabled = pending.destructive;
  if (!dialog.open) dialog.showModal();
  $('confirm-cancel').focus();
}

function render(data) {
  state.data = data;
  setText('version', `设备助手 v${data.version}`);
  renderPrepare(data);
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
  try { await fn(); } catch (error) { flashPrepare('操作未完成', error.message, 'warning'); }
  poll();
}
function flashPrepare(title, message, severity) {
  if (!state.data) return;
  state.data.prepare = { title, message, severity };
  renderPrepare(state.data);
}
const port = () => $('ports').value;

$('scan-ports').addEventListener('click', () => action(() => post('/api/ports')));
$('identify').addEventListener('click', () => action(() => post('/api/at/identify', { port: port() })));
$('unlock-adb').addEventListener('click', () => action(() => post('/api/at/preview', { port: port(), action: 'adb' })));
$('restart').addEventListener('click', () => action(() => post('/api/at/preview', { port: port(), action: 'restart' })));
$('factory').addEventListener('click', () => action(() => post('/api/at/preview', { port: port(), action: 'factory' })));
$('run-custom').addEventListener('click', () => action(() => post('/api/at/preview', { port: port(), action: 'custom', commands: $('custom-at').value })));
for (const [id, enable] of [['eth-enable', true], ['eth-disable', false]]) {
  $(id).addEventListener('click', () => action(() => post('/api/at/preview', { port: port(), action: 'ethernet', enable, profile: $('eth-profile').value, driver: $('eth-profile').value === 'pcie' ? $('eth-driver').value : null })));
}
$('eth-profile').addEventListener('change', () => { $('eth-driver').disabled = $('eth-profile').value !== 'pcie'; });
$('read-info').addEventListener('click', () => { $('info-card').open = true; action(() => post('/api/at/info', { port: port() })); });
$('stop-at').addEventListener('click', () => action(() => post('/api/at/stop')));
$('save-at-log').addEventListener('click', () => action(() => post('/api/at/log')));
$('save-preset').addEventListener('click', () => action(() => post('/api/at/preset', { text: $('custom-at').value })));
$('load-preset').addEventListener('click', () => action(async () => { $('custom-at').value = (await api('/api/at/preset')).text; }));
$('at-preset').addEventListener('change', () => { if ($('at-preset').value) $('custom-at').value = $('at-preset').value; });
$('ports').addEventListener('change', () => state.data && renderPrepare(state.data));

$('confirm-agree').addEventListener('change', () => { $('confirm-ok').disabled = !$('confirm-agree').checked; });
$('confirm-cancel').addEventListener('click', () => { $('confirm').close(); action(() => post('/api/at/dismiss')); });
$('confirm').addEventListener('cancel', () => action(() => post('/api/at/dismiss')));
$('confirm-ok').addEventListener('click', () => {
  const id = state.pendingShown;
  $('confirm').close();
  action(() => post('/api/at/confirm', { id }));
});

$('refresh-devices').addEventListener('click', () => action(() => post('/api/devices')));
$('devices').addEventListener('change', () => { state.inputError = null; state.data && renderInstall(state.data); });
for (const id of ['change-port', 'change-web', 'change-root']) $(id).addEventListener('change', () => { state.inputError = null; state.data && renderInstall(state.data); });

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
    $('log-card').open = true;
    $('verify').scrollIntoView({ block: 'nearest' });
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
  } catch (error) { state.inputError = { title: '查看报告', message: error.message }; renderInstall(state.data); }
});
$('report-notepad').addEventListener('click', () => post('/api/open', { target: 'report' }).catch(() => {}));
$('open-reports').addEventListener('click', () => post('/api/open', { target: 'reports' }).catch(() => {}));

$('quit').addEventListener('click', async () => {
  if (!confirm('退出设备助手？退出后此页面将不可用。')) return;
  try { await post('/api/quit'); showOverlay('设备助手已退出', '可以关闭此页面。需要时重新双击 SimpleAdmin-Setup.exe。'); }
  catch (error) { alert(error.message); }
});

// Highlight the last step whose section has reached the upper part of the window.
const sections = ['prepare', 'adb', 'network', 'install', 'verify'].map((id) => $(id));
let highlightQueued = false;
function highlightStep() {
  highlightQueued = false;
  const line = window.innerHeight * 0.3;
  const atBottom = window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 4;
  let current = sections[0].id;
  for (const section of sections) if (section.getBoundingClientRect().top <= line) current = section.id;
  if (atBottom) current = sections[sections.length - 1].id;
  document.querySelectorAll('.steps a').forEach((a) => a.classList.toggle('active', a.dataset.step === current));
}
window.addEventListener('scroll', () => { if (!highlightQueued) { highlightQueued = true; requestAnimationFrame(highlightStep); } }, { passive: true });
window.addEventListener('resize', highlightStep);
highlightStep();

// Tell the assistant the page is going away so it can exit once nothing is running.
window.addEventListener('pagehide', () => { if (!state.exited) fetch('/api/bye', { method: 'POST', keepalive: true }).catch(() => {}); });
document.addEventListener('visibilitychange', () => { if (!document.hidden) poll(); });

poll();
post('/api/devices').catch(() => {}).finally(poll);
post('/api/ports').catch(() => {}).finally(poll);
scheduleDevices();
