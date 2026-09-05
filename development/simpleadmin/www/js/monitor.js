(function (global) {
  'use strict';

  const t = text => global.SimpleAdmin && global.SimpleAdmin.Lang ? global.SimpleAdmin.Lang.t(text) : text;
  const statusNames = { ok: '链路正常', timeout: '应答超时', dns_error: 'DNS 解析失败', permission_error: 'ICMP 权限不足', unavailable: '网络不可达', pending: '等待采样' };
  const number = value => value === null || value === undefined || !Number.isFinite(Number(value)) ? '--' : Number(value).toFixed(1);
  const clock = value => new Date(value).toLocaleTimeString('en-GB', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });

  function chartOptions(snapshot, radio, colors, compact) {
    const axis = { axisLine: { show: false }, axisTick: { show: false }, axisLabel: { color: colors.muted, fontSize: 11 }, splitLine: { lineStyle: { color: colors.border, type: 'dashed' } } };
    const base = () => ({
      animation: false,
      textStyle: { fontFamily: 'Microsoft YaHei, sans-serif' },
      grid: { top: 54, bottom: 34, left: 54, right: 50 },
      legend: { top: 6, left: 20, itemWidth: 8, itemHeight: 8, icon: 'circle', textStyle: { color: colors.text, fontSize: 12 } },
      tooltip: { trigger: 'axis', renderMode: 'richText', confine: true, backgroundColor: colors.surface, borderColor: colors.border, textStyle: { color: colors.text, fontSize: 12 }, formatter(params) {
        if (!params.length) return '';
        return clock(params[0].value[0]) + '\n' + params.map(item => item.seriesName + '  ' + (item.seriesName === t('失败') ? t(statusNames[item.value[2]] || '检测失败') : number(item.value[1]))).join('\n');
      } },
      xAxis: { ...axis, type: 'time', min: snapshot.serverTime - 300000, max: snapshot.serverTime, splitNumber: compact ? 3 : 5, splitLine: { show: false }, axisLabel: { ...axis.axisLabel, formatter: value => clock(value).slice(0, 5), hideOverlap: true } }
    });
    const line = (name, data, color, axisIndex = 0) => ({ name, type: 'line', data, yAxisIndex: axisIndex, showSymbol: data.filter(item => item[1] !== null).length < 2, symbolSize: 5, connectNulls: false, smooth: false, lineStyle: { width: 2, color }, itemStyle: { color }, emphasis: { focus: 'series' } });
    const points = key => snapshot.signal.map(sample => [sample.time, sample[key]]);
    const range = (key, min, max) => {
      const values = snapshot.signal.map(sample => sample[key]).filter(value => value !== null && value !== undefined);
      return { min: Math.min(min, ...values.map(value => Math.floor(value / 10) * 10)), max: Math.max(max, ...values.map(value => Math.ceil(value / 10) * 10)) };
    };
    const signal = {
      ...base(),
      legend: { ...base().legend, data: ['RSRP', 'SINR'] },
      yAxis: [
        { ...axis, type: 'value', name: 'RSRP (dBm)', nameTextStyle: { color: '#5d87ff', fontSize: 11 }, ...range('rsrp' + radio, -140, -40), position: 'left' },
        { ...axis, type: 'value', name: 'SINR (dB)', nameTextStyle: { color: '#13b99a', fontSize: 11 }, ...range('sinr' + radio, -20, 40), position: 'right', splitLine: { show: false } }
      ],
      series: [line('RSRP', points('rsrp' + radio), '#5d87ff'), line('SINR', points('sinr' + radio), '#13b99a', 1)]
    };
    const temperature = {
      ...base(), grid: { top: 54, bottom: 34, left: 46, right: 22 },
      yAxis: { ...axis, type: 'value', name: '°C', min: value => Number.isFinite(value.min) ? Math.floor((value.min - 3) / 5) * 5 : 20, max: value => Number.isFinite(value.max) ? Math.ceil((value.max + 3) / 5) * 5 : 80 },
      series: [{ ...line(t('温度'), points('temperature'), '#ef9063'), areaStyle: { color: '#ef9063', opacity: 0.07 } }]
    };
    const ping = {
      ...base(), grid: { top: 50, bottom: 32, left: 48, right: 24 },
      yAxis: { ...axis, type: 'value', name: 'ms', min: 0 },
      series: [
        line(t('延迟'), snapshot.ping.map(sample => [sample.time, sample.rtt]), '#5d87ff'),
        line(t('抖动'), snapshot.ping.map(sample => [sample.time, sample.jitter]), '#13b99a'),
        { name: t('失败'), type: 'scatter', symbolSize: 7, itemStyle: { color: '#e95766' }, data: snapshot.ping.filter(sample => sample.status !== 'ok').map(sample => [sample.time, 0, sample.status]) }
      ]
    };
    return { signal, temperature, ping };
  }

  function availableRadioModes(snapshot) {
    const latest = snapshot.signal[snapshot.signal.length - 1];
    return latest ? ['NR', 'LTE'].filter(radio => ['rsrp', 'sinr'].some(key => latest[key + radio] !== null && latest[key + radio] !== undefined && Number.isFinite(latest[key + radio]))) : [];
  }
  global.SimpleAdminMonitor = { chartOptions, number, statusNames, availableRadioModes };

  let mounted = false;
  function mount() {
    // The dashboard must own its DOM before Teleport inserts reactive summary cards.
    if (mounted || !global.SimpleAdmin.Vue.apps || !global.SimpleAdmin.Vue.apps['#dashboardApp']) return;
    const element = document.getElementById('monitorApp');
    if (!element) return;
    mounted = true;
    global.removeEventListener('simpleadmin:vue-mounted', mount);
    const charts = {};
    let timer = null;
    let controller = null;
    let fetching = false;
    let requestEpoch = 0;
    let app;
    const active = () => !document.hidden && !!document.querySelector('#page-dashboard.active');
    const stop = () => { clearTimeout(timer); timer = null; if (controller) controller.abort(); };
    const colors = () => {
      const style = getComputedStyle(document.documentElement);
      return Object.fromEntries(['muted', 'border', 'text', 'surface'].map(key => [key, style.getPropertyValue('--sa-' + key).trim()]));
    };
    const render = () => {
      if (!active()) return;
      const options = chartOptions(app.snapshot, app.radio, colors(), element.clientWidth < 600);
      for (const key of ['signal', 'temperature', 'ping']) {
        if (!charts[key]) charts[key] = global.SimpleAdminCharts.init(document.getElementById(key + 'Chart'));
        charts[key].resize();
        charts[key].setOption(options[key], { notMerge: true });
      }
    };
    const update = data => {
      app.snapshot = data;
      if (!app.targetDirty) app.targetInput = data.target;
      const radios = availableRadioModes(data);
      if (radios.length && !radios.includes(app.radio)) app.radio = radios[0];
      app.$nextTick(render);
    };
    app = global.Vue.createApp({
      data: () => ({ snapshot: { target: 'www.baidu.com', generation: 0, serverTime: Date.now(), ping: [], signal: [], summary: {}, mock: false }, radio: 'NR', targetInput: 'www.baidu.com', targetDirty: false, targetMessage: '', saving: false, error: '' }),
      computed: {
        availableRadios() { return availableRadioModes(this.snapshot); },
        latestPing() { return this.snapshot.ping[this.snapshot.ping.length - 1] || null; },
        latestSignal() { return this.snapshot.signal[this.snapshot.signal.length - 1] || null; },
        signalTime() { return this.latestSignal ? clock(this.latestSignal.time) : '等待采样'; },
        hasSignal() { return this.snapshot.signal.some(sample => sample['rsrp' + this.radio] !== null || sample['sinr' + this.radio] !== null); },
        hasTemperature() { return this.snapshot.signal.some(sample => sample.temperature !== null); },
        failedSamples() { return this.snapshot.ping.filter(sample => sample.status !== 'ok').length; }
      },
      methods: {
        number,
        statusLabel(status) { return statusNames[status] || '等待采样'; },
        setRadio(radio) { this.radio = radio; render(); },
        async refresh() {
          if (!active() || fetching || this.saving) return;
          clearTimeout(timer);
          fetching = true;
          controller = new AbortController();
          const epoch = requestEpoch;
          const deadline = setTimeout(() => controller && controller.abort(), 5000);
          try {
            const response = await fetch('/api/telemetry', { cache: 'no-store', signal: controller.signal });
            if (response.status === 401) { stop(); global.location.replace('/login.html'); return; }
            if (!response.ok) throw new Error('monitor request failed');
            const data = await response.json();
            if (epoch !== requestEpoch) return;
            this.error = '';
            update(data);
          } catch (error) {
            if (active() && epoch === requestEpoch) this.error = '无法读取监控数据';
          } finally {
            clearTimeout(deadline);
            fetching = false;
            controller = null;
            if (active() && !this.saving) timer = setTimeout(() => this.refresh(), 1000);
          }
        },
        async saveTarget() {
          if (this.saving) return;
          this.saving = true;
          this.targetMessage = '';
          requestEpoch++;
          stop();
          const saveController = new AbortController();
          const deadline = setTimeout(() => saveController.abort(), 5000);
          try {
            const response = await fetch('/api/telemetry/target', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ target: this.targetInput.trim() }), signal: saveController.signal });
            if (response.status === 401) { global.location.replace('/login.html'); return; }
            if (!response.ok) throw new Error(response.status === 400 ? '请输入有效域名或 IP，不包含协议、端口或路径' : '目标保存失败，请重试');
            this.targetDirty = false;
            this.error = '';
            update(await response.json());
            this.targetMessage = '目标已保存';
          } catch (error) {
            this.targetMessage = error.name === 'AbortError' ? '请求超时，请确认目标是否已更新' : error.message;
          } finally {
            clearTimeout(deadline);
            this.saving = false;
            if (active()) this.refresh();
          }
        }
      }
    }).mount(element);
    const visibility = () => { stop(); if (active()) { app.$nextTick(render); app.refresh(); } };
    global.addEventListener('simpleadmin:page-changed', visibility);
    global.addEventListener('simpleadmin:language-changed', render);
    document.addEventListener('visibilitychange', visibility);
    const observer = new ResizeObserver(() => { if (active()) render(); });
    observer.observe(element);
    const themeObserver = new MutationObserver(render);
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['data-bs-theme'] });
    global.addEventListener('pagehide', event => {
      stop();
      if (!event.persisted) { observer.disconnect(); themeObserver.disconnect(); Object.values(charts).forEach(chart => chart.dispose()); }
    });
    global.addEventListener('pageshow', event => { if (event.persisted) visibility(); });
    if (global.SimpleAdminIcons) global.SimpleAdminIcons();
    visibility();
  }
  global.addEventListener('simpleadmin:vue-mounted', mount);
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount, { once: true }); else mount();
})(window);
