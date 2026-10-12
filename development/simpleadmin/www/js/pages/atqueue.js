// AT queue on the AT terminal page: the command on the port now, those waiting behind it and
// those finished in the last 30 seconds with their responses. Polled once a second while shown.
(function (global) {
  const STATUS = {
    ok: ['成功', 'OK', 'is-good'],
    error: ['模块返回错误', 'Modem error', 'is-warn'],
    failed: ['失败', 'Failed', 'is-bad'],
    cancelled: ['已取消', 'Cancelled', '']
  };
  const english = () => !!(global.SimpleAdmin.Lang && global.SimpleAdmin.Lang.getCurrentLanguage() !== 'zh-CN');

  function clock(ms) {
    const time = new Date(ms);
    const pad = (n, w = 2) => String(n).padStart(w, '0');
    return `${pad(time.getHours())}:${pad(time.getMinutes())}:${pad(time.getSeconds())}.${pad(time.getMilliseconds(), 3)}`;
  }
  function seconds(ms) {
    if (ms === undefined || ms === null || ms < 0) return '-';
    return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
  }

  function mount() {
    const element = document.getElementById('atQueueApp');
    if (!element || element.dataset.mounted) return;
    element.dataset.mounted = '1';
    let timer = null;
    const spa = () => global.SimpleAdmin.Spa;
    const active = () => !document.hidden && !!document.querySelector('#page-settings.active') && !!spa() && spa().current() === 'at';
    const app = global.Vue.createApp({
      data: () => ({ queue: null, error: '', paused: false, open: {}, language: 0 }),
      computed: {
        waiting() { return (this.queue && this.queue.waiting) || []; },
        done() { return (this.queue && this.queue.done) || []; },
        running() { return this.queue && this.queue.running; },
        summary() {
          if (!this.queue) return '';
          const busy = this.running ? 1 : 0;
          return this.t(`执行中 ${busy} · 等待 ${this.waiting.length} · 最近 30 秒 ${this.done.length} 条`,
            `Running ${busy} · waiting ${this.waiting.length} · last 30 s ${this.done.length}`);
        }
      },
      methods: {
        t(zh, en) { this.language; return english() ? en : zh; },
        clock,
        seconds,
        status(item) { const s = STATUS[item.status] || [item.status, item.status, '']; return this.t(s[0], s[1]); },
        statusClass(item) { return (STATUS[item.status] || ['', '', ''])[2]; },
        // Durations come from the module's monotonic clock, so a clock step cannot skew them.
        took(item) { return seconds(item.run_ms); },
        waited(item) { return seconds(item.wait_ms); },
        toggle(item, event) { this.open[item.id] = event.target.open; },
        pause() {
          this.paused = !this.paused;
          if (!this.paused) this.refresh();
        },
        async refresh() {
          clearTimeout(timer);
          if (!active() || this.paused) return;
          try {
            const data = await global.SimpleAdmin.Api.settingsData({ action: 'at_queue' });
            if (!data || !Array.isArray(data.done)) throw new Error('');
            this.queue = data;
            this.error = '';
            const shown = new Set(data.done.map(item => item.id));
            for (const id of Object.keys(this.open)) if (!shown.has(Number(id))) delete this.open[id];
          } catch (_) {
            this.error = this.t('无法读取 AT 队列', 'Cannot read the AT queue');
          } finally {
            if (active() && !this.paused) timer = setTimeout(() => this.refresh(), 1000);
          }
        }
      }
    }).mount(element);
    global.addEventListener('simpleadmin:page-changed', () => app.refresh());
    global.addEventListener('simpleadmin:language-changed', () => { app.language++; });
    document.addEventListener('visibilitychange', () => app.refresh());
    app.refresh();
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount);
  else mount();
})(window);
