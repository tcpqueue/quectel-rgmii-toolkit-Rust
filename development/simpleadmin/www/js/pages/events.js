// Recent network events on the overview page: radio changes, internet outages, band-lock
// fallbacks and watchdog actions. The list lives in the module's memory (newest 200).
(function (global) {
  const STEPS = {
    reregister: ['重新注册网络', 'Re-register on the network'],
    radio: ['重启射频', 'Restart the radio'],
    sim_switch: ['切换 SIM 卡槽', 'Switch SIM slot'],
    reboot: ['重启模块', 'Restart the module']
  };
  const MODES = { lte_band: 'LTE', nsa_nr5g_band: 'NR5G-NSA', nr5g_band: 'NR5G-SA' };
  // [Chinese, English]; {name} is filled from the event detail.
  const TEXT = {
    mode_change: ['网络制式 {from} → {to}', 'Network {from} → {to}'],
    signal_lost: ['失去信号（原 {from}）', 'Signal lost (was {from})'],
    signal_restored: ['重新驻网：{to} {bands}', 'Registered again: {to} {bands}'],
    sim_removed: ['SIM 卡被移除或无法识别', 'SIM card removed or not recognised'],
    sim_inserted: ['检测到 SIM 卡', 'SIM card detected'],
    cell_change: ['主小区切换 PCI {from} → {to}（{band}）', 'Serving cell PCI {from} → {to} ({band})'],
    band_change: ['频段变化 {from} → {to}', 'Band {from} → {to}'],
    ca_change: ['载波聚合变化 {from} → {to}', 'Carrier aggregation {from} → {to}'],
    internet_lost: ['互联网中断', 'Internet connection lost'],
    internet_restored: ['互联网恢复，中断 {seconds} 秒', 'Internet restored after {seconds} s'],
    band_reverted: ['{mode} 锁定频段 {locked} 后 30 秒无信号，已恢复为 {previous}', '{mode} bands {locked} found no cell in 30 s; restored {previous}'],
    band_revert_failed: ['{mode} 锁定频段后无信号，恢复原频段 {previous} 失败', '{mode} bands found no cell; restoring {previous} failed'],
    watchdog_start: ['看门狗：断网 {seconds} 秒，开始自动恢复', 'Watchdog: offline for {seconds} s, starting recovery'],
    watchdog_step: ['看门狗：第 {tier} 级 {step}', 'Watchdog: step {tier}, {step}'],
    watchdog_step_failed: ['看门狗：第 {tier} 级 {step} 失败：{error}', 'Watchdog: step {tier}, {step} failed: {error}'],
    watchdog_recovered: ['看门狗：第 {tier} 级 {step} 后已恢复联网', 'Watchdog: back online after step {tier}, {step}'],
    watchdog_sim_back: ['看门狗：换卡槽无效，已切回卡槽 {slot}', 'Watchdog: the other slot did not help; back to slot {slot}'],
    watchdog_lock_restored: ['看门狗：射频重启后重新应用 {radio} 锁定', 'Watchdog: {radio} lock applied again after the radio restart'],
    watchdog_lock_restore_failed: ['看门狗：射频重启后重新应用 {radio} 锁定失败', 'Watchdog: applying the {radio} lock again failed'],
    freq_reverted: ['5G 频点锁定 {locked} 后 30 秒无可用小区，已恢复为 {previous}', '5G frequencies {locked} found no usable cell in 30 s; restored {previous}'],
    freq_revert_failed: ['5G 频点锁定 {locked} 后无可用小区，恢复失败', '5G frequencies {locked} found no usable cell; restoring failed'],
    watchdog_reboot_limit: ['看门狗：已连续重启 {limit} 次，跳过重启', 'Watchdog: {limit} restarts in a row, restart skipped'],
    watchdog_gave_up: ['看门狗：已启用的步骤都未恢复联网，冷却后再试', 'Watchdog: no enabled step brought the connection back; retrying after the cooldown']
  };
  const english = () => !!(global.SimpleAdmin.Lang && global.SimpleAdmin.Lang.getCurrentLanguage() !== 'zh-CN');

  function describe(event) {
    const en = english() ? 1 : 0;
    const d = Object.assign({}, event.detail || {});
    let kind = event.kind;
    if (d.step && STEPS[d.step]) d.step = STEPS[d.step][en];
    if (d.mode && MODES[d.mode]) d.mode = MODES[d.mode];
    if (d.radio) d.radio = ({ nr: en ? 'NR5G-SA cell' : 'NR5G-SA 小区', nr_freq: en ? '5G frequency' : '5G 频点', lte: en ? 'LTE cell' : 'LTE 小区' })[d.radio] || d.radio;
    if (kind === 'freq_reverted' && !d.previous) d.previous = en ? 'no lock' : '未锁定';
    if (kind === 'watchdog_lock_restored' && d.ok === false) kind = 'watchdog_lock_restore_failed';
    if (kind === 'watchdog_step' && d.count) d.step += en ? ` (#${d.count})` : `（第 ${d.count} 次）`;
    for (const key of ['from', 'to']) if (d[key] === '') d[key] = en ? 'none' : '无';
    const template = (TEXT[kind] || [kind, kind])[en];
    return template.replace(/\{(\w+)\}/g, (_, key) => (d[key] === undefined || d[key] === null ? '' : String(d[key]))).replace(/\s+）/, '）').trim();
  }
  function stamp(ms) {
    const time = new Date(ms);
    const pad = n => String(n).padStart(2, '0');
    const clock = `${pad(time.getHours())}:${pad(time.getMinutes())}:${pad(time.getSeconds())}`;
    return time.toDateString() === new Date().toDateString() ? clock : `${time.getMonth() + 1}/${time.getDate()} ${clock}`;
  }

  function mount() {
    const element = document.getElementById('eventsApp');
    if (!element || element.dataset.mounted) return;
    element.dataset.mounted = '1';
    let timer = null;
    const active = () => !document.hidden && !!document.querySelector('#page-events.active');
    const app = global.Vue.createApp({
      data: () => ({ events: [], loaded: false, error: '', expanded: false, language: 0 }),
      computed: {
        shown() { return this.expanded ? this.events : this.events.slice(0, 8); }
      },
      methods: {
        describe(event) { this.language; return describe(event); },
        stamp,
        t(zh, en) { this.language; return english() ? en : zh; },
        async refresh() {
          clearTimeout(timer);
          if (!active()) return;
          try {
            const data = await global.SimpleAdmin.Api.networkData({ action: 'events' });
            this.events = Array.isArray(data.events) ? data.events : [];
            this.error = '';
          } catch (error) {
            this.error = this.t('无法读取网络事件', 'Cannot read network events');
          } finally {
            this.loaded = true;
            if (active()) timer = setTimeout(() => this.refresh(), 10000);
          }
        },
        async clear() {
          if (!window.confirm(this.t('清空网络事件记录？', 'Clear the network event log?'))) return;
          try { await global.SimpleAdmin.Api.networkData({ action: 'events_clear' }); } catch (_) {}
          this.refresh();
        }
      }
    }).mount(element);
    global.addEventListener('simpleadmin:page-changed', () => app.refresh());
    global.addEventListener('simpleadmin:language-changed', () => { app.language++; });
    document.addEventListener('visibilitychange', () => app.refresh());
    app.refresh();
  }
  global.SimpleAdminEvents = { describe };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount);
  else mount();
})(window);
