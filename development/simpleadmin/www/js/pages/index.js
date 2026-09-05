const NETWORK_COMPACT_KEY = 'simpleadmin.dashboard.compactNetworkInfo';

function readNetworkCompactState() {
  try {
    return localStorage.getItem(NETWORK_COMPACT_KEY) === '1';
  } catch (err) {
    return false;
  }
}

function saveNetworkCompactState(enabled) {
  try {
    localStorage.setItem(NETWORK_COMPACT_KEY, enabled ? '1' : '0');
  } catch (err) {
    // Ignore unavailable localStorage so the page can still work in restricted browsers.
  }
}

function getStaticNetworkInfo() {
  return {
    sim: '未激活',
    temperature: '0',
    active_sim: '-',
    network_provider: '-',
    mccmnc: '-',
    apn: '-',
    network_mode: '-',
    ipv4: '-',
    ipv6: '-',
    bands: '-',
    bandwidth: '-',
    prxqrsrp: '-',
    drxqrsrp: '-',
    rx2qrsrp: '-',
    rx3qrsrp: '-',
    earfcns: '-',
    pcc_pci: '-',
    scc_pci: '-',
    signalAssessment: '-',
    csq: '-',
    rssi: '-',
    cellID: '-',
    eNBID: '-',
    tac: '-',
    rsrqLTE: '-',
    rsrqNR: '-',
    rsrqLTEPercentage: 0,
    rsrqNRPercentage: 0,
    rsrpLTE: '-',
    rsrpNR: '-',
    rsrpLTEPercentage: 0,
    rsrpNRPercentage: 0,
    sinrLTE: '-',
    sinrNR: '-',
    sinrLTEPercentage: 0,
    sinrNRPercentage: 0,
    signalPercentage: 0,
    cpuUsagePercent: 0,
    ramUsagePercent: 0,
    ramUsedHuman: '-',
    ramTotalHuman: '-',
    internetConnection: '未连接',
    lastUpdate: new Date().toLocaleString(),
    newRefreshRate: null,
    refreshRate: 2,
    intervalId: null,
    uptime: '未知',
    _uptimeParts: null,
    nr_rx_bytes: 0,
    nr_tx_bytes: 0,
    nr_rx_human: '-',
    nr_tx_human: '-',
    nr_dl_speed: '-',
    nr_ul_speed: '-',
    _prev_nr_rx: null,
    _prev_nr_tx: null,
    _prev_nr_t: null,
    sensitiveVisible: SimpleAdmin.Mask.getSensitiveVisible(),
    compactNetworkInfo: readNetworkCompactState(),
    _dashboardActive: false,
    _dashboardPageChangeHandler: null,

    hasRadioSignal(radio) {
      return ['rsrp', 'rsrq', 'sinr'].some(key => {
        const value = this[key + radio];
        return value !== null && value !== undefined && String(value).trim() !== '' && Number.isFinite(Number(value));
      });
    },

    toggleNetworkCompact() {
      this.compactNetworkInfo = !this.compactNetworkInfo;
      saveNetworkCompactState(this.compactNetworkInfo);
    },

    toggleSensitiveVisible() {
      this.sensitiveVisible = SimpleAdmin.Mask.setSensitiveVisible(!this.sensitiveVisible);
    },

    formatSensitiveValue(value, type) {
      return SimpleAdmin.Mask.format(this.sensitiveVisible, value, type);
    },

    formatActiveSimStatus() {
      const simState = String(this.sim || '').trim();
      const slot = String(this.active_sim || '').trim();
      if (simState === '已激活') {
        if (slot && slot !== '-') {
          return `已激活卡${slot.replace(/^卡/, '')}`;
        }
        return '已激活';
      }
      if (slot && slot !== '-' && simState && simState !== '-') {
        return `${simState}卡${slot.replace(/^卡/, '')}`;
      }
      return simState || '-';
    },

    getProgressBarClass(percentage) {
      const value = Number(percentage);
      if (value >= 60) return 'progress-bar bg-success';
      if (value >= 40) return 'progress-bar bg-warning';
      return 'progress-bar bg-danger';
    },

    clampPercent(value) {
      const number = Number(value);
      if (!Number.isFinite(number)) return 0;
      return Math.max(0, Math.min(100, Math.round(number)));
    },

    gaugeDashOffset(value) {
      const arcLength = 301.6;
      return String((arcLength * (100 - this.clampPercent(value)) / 100).toFixed(1));
    },

    formatPercent(value) {
      return `${this.clampPercent(value)}%`;
    },

    formatRamUsage() {
      if (this.ramUsedHuman && this.ramUsedHuman !== '-' && this.ramTotalHuman && this.ramTotalHuman !== '-') {
        return `${this.ramUsedHuman} / ${this.ramTotalHuman}`;
      }
      return this.t('获取中...');
    },

    fetchNetworkInfo() {
      if (!this._dashboardActive || !this.isDashboardPageActive()) {
        this.stopDashboardRefresh();
        return;
      }
      SimpleAdmin.Api.getDashboardData({})
        .then((data) => {
          if (!this._dashboardActive || !this.isDashboardPageActive()) return;
          this.applyDashboardData(data || {});
          if (data && data.uptimeParts) {
            this.setUptimeParts(data.uptimeParts);
          }
        })
        .catch((err) => {
          if (!this._dashboardActive || !this.isDashboardPageActive()) return;
          console.error('dashboard_data error:', err);
          this.lastUpdate = new Date().toLocaleString();
        });
    },

    applyDashboardData(data) {
      const textKeys = [
        'sim', 'temperature', 'active_sim', 'network_provider', 'mccmnc', 'apn',
        'network_mode', 'ipv4', 'ipv6', 'bands', 'bandwidth', 'prxqrsrp',
        'drxqrsrp', 'rx2qrsrp', 'rx3qrsrp', 'earfcns', 'pcc_pci', 'scc_pci',
        'signalAssessment', 'csq', 'rssi', 'cellID', 'eNBID', 'tac',
        'rsrqLTE', 'rsrqNR', 'rsrpLTE', 'rsrpNR', 'sinrLTE', 'sinrNR',
        'internetConnection', 'lastUpdate', 'nr_rx_human', 'nr_tx_human',
        'ramUsedHuman', 'ramTotalHuman'
      ];

      textKeys.forEach((key) => {
        if (Object.prototype.hasOwnProperty.call(data, key)) {
          this[key] = data[key];
        }
      });

      const percentKeys = [
        'rsrqLTEPercentage', 'rsrqNRPercentage', 'rsrpLTEPercentage',
        'rsrpNRPercentage', 'sinrLTEPercentage', 'sinrNRPercentage',
        'signalPercentage', 'cpuUsagePercent', 'ramUsagePercent'
      ];
      percentKeys.forEach((key) => {
        if (Object.prototype.hasOwnProperty.call(data, key)) {
          this[key] = Number(data[key]) || 0;
        }
      });

      this.updateTraffic(data);
      this.lastUpdate = data.lastUpdate || new Date().toLocaleString();
    },

    updateTraffic(data) {
      const rx = Number(data.nr_rx_bytes);
      const tx = Number(data.nr_tx_bytes);
      if (!Number.isFinite(rx) || !Number.isFinite(tx)) return;

      const now = Date.now();
      if (this._prev_nr_rx !== null && this._prev_nr_tx !== null && this._prev_nr_t !== null) {
        const dt = Math.max(0.001, (now - this._prev_nr_t) / 1000);
        const dRx = rx >= this._prev_nr_rx ? (rx - this._prev_nr_rx) : 0;
        const dTx = tx >= this._prev_nr_tx ? (tx - this._prev_nr_tx) : 0;
        this.nr_dl_speed = data.nr_dl_speed && data.nr_dl_speed !== '-' ? data.nr_dl_speed : this.humanBytesPerSec(dRx / dt);
        this.nr_ul_speed = data.nr_ul_speed && data.nr_ul_speed !== '-' ? data.nr_ul_speed : this.humanBytesPerSec(dTx / dt);
      } else {
        this.nr_dl_speed = data.nr_dl_speed || '-';
        this.nr_ul_speed = data.nr_ul_speed || '-';
      }

      this.nr_rx_bytes = rx;
      this.nr_tx_bytes = tx;
      if (!data.nr_rx_human) this.nr_rx_human = this.humanBytes(rx);
      if (!data.nr_tx_human) this.nr_tx_human = this.humanBytes(tx);
      this._prev_nr_rx = rx;
      this._prev_nr_tx = tx;
      this._prev_nr_t = now;
    },

    resetNetworkInfo() {
      this.applyDashboardData({
        sim: '未激活',
        active_sim: '-',
        network_provider: '-',
        mccmnc: '-',
        apn: '-',
        network_mode: '未插卡',
        ipv4: '-',
        ipv6: '-',
        bands: '-',
        bandwidth: '-',
        prxqrsrp: '-',
        drxqrsrp: '-',
        rx2qrsrp: '-',
        rx3qrsrp: '-',
        earfcns: '-',
        pcc_pci: '-',
        scc_pci: '-',
        signalAssessment: '未知',
        csq: '-',
        rssi: '-',
        cellID: '-',
        eNBID: '-',
        tac: '-',
        rsrqLTE: '-',
        rsrqNR: '-',
        rsrpLTE: '-',
        rsrpNR: '-',
        sinrLTE: '-',
        sinrNR: '-',
        internetConnection: '未连接',
        nr_rx_human: '-',
        nr_tx_human: '-',
        ramUsedHuman: '-',
        ramTotalHuman: '-',
        rsrqLTEPercentage: 0,
        rsrqNRPercentage: 0,
        rsrpLTEPercentage: 0,
        rsrpNRPercentage: 0,
        sinrLTEPercentage: 0,
        sinrNRPercentage: 0,
        signalPercentage: 0,
        cpuUsagePercent: 0,
        ramUsagePercent: 0,
        nr_rx_bytes: 0,
        nr_tx_bytes: 0
      });
      this.nr_dl_speed = '-';
      this.nr_ul_speed = '-';
      this._prev_nr_rx = null;
      this._prev_nr_tx = null;
      this._prev_nr_t = null;
    },

    humanBytes(bytes) {
      const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
      let value = Number(bytes);
      if (!Number.isFinite(value) || value < 0) return '-';
      let i = 0;
      while (value >= 1024 && i < units.length - 1) {
        value /= 1024;
        i += 1;
      }
      const n = (i > 0 && value < 10) ? value.toFixed(1) : Math.round(value);
      return `${n} ${units[i]}`;
    },

    humanBytesPerSec(bytesPerSecond) {
      const value = this.humanBytes(bytesPerSecond);
      return value === '-' ? '-' : `${value}/s`;
    },

    t(key) {
      return SimpleAdmin.Lang ? SimpleAdmin.Lang.t(key) : key;
    },

    parseUptimeParts(data) {
      const text = String(data || '');
      const days = text.match(/(\d+)\s+day/);
      const hours = text.match(/(\d+)\s+hour/);
      const minutes = text.match(/(\d+)\s+min/);
      const hm = text.match(/(\d+):(\d+),/);
      if (!days && !hours && !minutes && !hm) return null;
      return {
        days: days ? Number(days[1]) : 0,
        hours: hm ? Number(hm[1]) : (hours ? Number(hours[1]) : 0),
        minutes: hm ? Number(hm[2]) : (minutes ? Number(minutes[1]) : 0)
      };
    },

    formatUptimeUnit(value, unit, language) {
      const amount = Number(value);
      if (!Number.isFinite(amount) || amount < 0) return '';
      if (amount === 0 && unit !== '分钟') return '';
      if (language === 'en') {
        const unitMap = { 天: 'day', 小时: 'hour', 分钟: 'minute' };
        const label = unitMap[unit] || unit;
        return `${amount} ${label}${amount === 1 ? '' : 's'}`;
      }
      return `${amount} ${this.t(unit)}`;
    },

    formatUptimeParts(parts) {
      if (!parts) return this.t('未知时间');
      const language = SimpleAdmin.Lang ? SimpleAdmin.Lang.getCurrentLanguage() : 'zh-CN';
      const items = [
        this.formatUptimeUnit(parts.days, '天', language),
        this.formatUptimeUnit(parts.hours, '小时', language),
        this.formatUptimeUnit(parts.minutes, '分钟', language)
      ].filter(Boolean);
      return items.length ? items.join(' ') : this.formatUptimeUnit(0, '分钟', language);
    },

    setUptimeParts(parts) {
      this._uptimeParts = parts;
      this.uptime = this.formatUptimeParts(parts);
    },

    updateRefreshRate() {
      const value = Number(this.newRefreshRate);
      this.refreshRate = Number.isFinite(value) ? Math.max(2, Math.min(60, value)) : this.refreshRate;
      localStorage.setItem('refreshRate', String(this.refreshRate));
      if (this._dashboardActive && this.isDashboardPageActive()) {
        this.startInterval();
      }
    },

    isDashboardPageActive() {
      if (!window.SimpleAdminSpaMode) return true;
      const section = document.querySelector('.sa-page[data-page="dashboard"]');
      return !!(section && section.classList.contains('active'));
    },

    startDashboardRefresh() {
      if (!this.isDashboardPageActive()) return;
      const wasActive = this._dashboardActive;
      this._dashboardActive = true;
      if (!wasActive) {
        this.tickOnce();
      }
      this.startInterval();
    },

    stopDashboardRefresh() {
      this._dashboardActive = false;
      if (this.intervalId) {
        clearInterval(this.intervalId);
        this.intervalId = null;
      }
    },

    init() {
      const stored = Number(localStorage.getItem('refreshRate'));
      if (Number.isFinite(stored) && stored >= 2) {
        this.refreshRate = stored;
      }
      window.addEventListener('simpleadmin:language-changed', () => {
        this.uptime = this._uptimeParts ? this.formatUptimeParts(this._uptimeParts) : this.t('未知时间');
      });
      this._dashboardPageChangeHandler = (event) => {
        if (event && event.detail && event.detail.page === 'dashboard') {
          this.startDashboardRefresh();
        } else {
          this.stopDashboardRefresh();
        }
      };
      if (window.SimpleAdminSpaMode) {
        window.addEventListener('simpleadmin:page-changed', this._dashboardPageChangeHandler);
      }
      this.startDashboardRefresh();
      window.addEventListener('beforeunload', () => {
        this.stopDashboardRefresh();
      }, { once: true });
    },

    startInterval() {
      if (this.intervalId) clearInterval(this.intervalId);
      if (!this._dashboardActive || !this.isDashboardPageActive()) {
        this.stopDashboardRefresh();
        return;
      }
      this.intervalId = setInterval(() => this.tickOnce(), this.refreshRate * 1000);
    },

    tickOnce() {
      if (!this._dashboardActive || !this.isDashboardPageActive()) {
        this.stopDashboardRefresh();
        return;
      }
      this.fetchNetworkInfo();
    },

    fetchUpTime() {
      return SimpleAdmin.Api.getUptime()
        .then((response) => response.text())
        .then((data) => this.setUptimeParts(this.parseUptimeParts(data)))
        .catch(() => this.setUptimeParts(null));
    },

    requestPing() {
      return SimpleAdmin.Api.getPing().then((response) => response.text());
    },

    requestPingWithTimeout(timeout = 5000) {
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('Ping request timed out')), timeout);
        this.requestPing()
          .then((res) => {
            clearTimeout(timer);
            resolve(res);
          })
          .catch((err) => {
            clearTimeout(timer);
            reject(err);
          });
      });
    }
  };
}

if (window.SimpleAdminSpaMode) {
  (window.SimpleAdmin.Pages = window.SimpleAdmin.Pages || {}).dashboard = getStaticNetworkInfo;
} else {
  mountSimpleAdminVueApp(getStaticNetworkInfo);
}
