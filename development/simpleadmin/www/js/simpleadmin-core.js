(function (global) {
  'use strict';

  const root = global.SimpleAdmin || {};

  root.Api = root.Api || (function () {
    const wsPath = '/api/ws';
    let ws = null;
    let connectPromise = null;
    let nextId = 1;
    const pending = new Map();

    function wsUrl() {
      const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
      return `${scheme}://${location.host}${wsPath}`;
    }

    function rejectAll(error) {
      pending.forEach((item) => item.reject(error));
      pending.clear();
    }

    function makeResponse(message) {
      const status = Number(message.status) || 0;
      const body = message.body || '';
      return {
        ok: status >= 200 && status < 300,
        status,
        headers: message.headers || {},
        text() {
          return Promise.resolve(body);
        },
        json() {
          return Promise.resolve().then(() => (body ? JSON.parse(body) : null));
        }
      };
    }


    function dispatchServerEvent(message) {
      if (typeof global.dispatchEvent !== 'function' || typeof global.CustomEvent !== 'function') return;
      global.dispatchEvent(new CustomEvent('simpleadmin:server-event', { detail: message }));
      global.dispatchEvent(new CustomEvent(`simpleadmin:${message.event}`, { detail: message.data || {} }));
    }

    function connect() {
      if (ws && ws.readyState === WebSocket.OPEN) return Promise.resolve(ws);
      if (connectPromise) return connectPromise;

      connectPromise = new Promise((resolve, reject) => {
        const socket = new WebSocket(wsUrl());
        ws = socket;

        socket.onopen = () => {
          connectPromise = null;
          resolve(socket);
        };
        socket.onerror = () => {
          const error = new Error('WebSocket connection failed');
          connectPromise = null;
          reject(error);
          rejectAll(error);
        };
        socket.onclose = () => {
          if (ws === socket) ws = null;
          connectPromise = null;
          rejectAll(new Error('WebSocket connection closed'));
        };
        socket.onmessage = (event) => {
          let message;
          try {
            message = JSON.parse(String(event.data || '{}'));
          } catch (error) {
            console.error('Invalid WebSocket API response:', error);
            return;
          }
          if (message.type === 'event' && message.event) {
            dispatchServerEvent(message);
            return;
          }
          const item = pending.get(message.id);
          if (!item) return;
          pending.delete(message.id);
          if (message.error && !message.status) {
            item.reject(new Error(message.error));
            return;
          }
          item.resolve(makeResponse(message));
        };
      });

      return connectPromise;
    }

    function normalizeHeaders(headers) {
      const normalized = {};
      if (!headers) return normalized;
      if (headers instanceof Headers) {
        headers.forEach((value, key) => { normalized[key] = value; });
        return normalized;
      }
      Object.keys(headers).forEach((key) => {
        if (headers[key] !== undefined && headers[key] !== null) {
          normalized[key] = String(headers[key]);
        }
      });
      return normalized;
    }

    function serializeBody(body) {
      if (body === undefined || body === null) return '';
      if (body instanceof URLSearchParams) return body.toString();
      if (typeof body === 'string') return body;
      return String(body);
    }

    function request(path, options = {}) {
      if (!String(path || '').startsWith('/api/')) {
        return fetch(path, options);
      }
      const method = String(options.method || 'GET').toUpperCase();
      const headers = normalizeHeaders(options.headers);
      const body = serializeBody(options.body);
      if (body && !Object.keys(headers).some((key) => key.toLowerCase() === 'content-type')) {
        headers['Content-Type'] = 'application/x-www-form-urlencoded; charset=UTF-8';
      }

      return connect().then((socket) => new Promise((resolve, reject) => {
        const id = String(nextId++);
        pending.set(id, { resolve, reject });
        try {
          socket.send(JSON.stringify({ id, method, path, headers, body }));
        } catch (error) {
          pending.delete(id);
          reject(error);
        }
      }));
    }

    function postForm(path, params = {}) {
      const body = params instanceof URLSearchParams ? params : new URLSearchParams(params);
      return request(path, {
        method: 'POST',
        cache: 'no-store',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded; charset=UTF-8' },
        body
      });
    }

    return {
      request,
      text(url, options) {
        return request(url, options).then((response) => response.text());
      },
      json(url, options) {
        return request(url, options).then((response) => response.json());
      },
      url(path, params) {
        const query = params ? new URLSearchParams(params).toString() : '';
        return query ? `${path}?${query}` : path;
      },
      postForm,
      postJSON(path, params = {}) {
        return postForm(path, params).then((response) => response.json());
      },
      getDashboardData(params = {}) {
        return this.postJSON('/api/dashboard_data', Object.assign({ action: 'get' }, params));
      },
      mockAT(params = {}) {
        return this.postJSON('/api/mock_at', params);
      },
      getDeviceInfo(params = {}) {
        return this.postJSON('/api/device_info_data', Object.assign({ action: 'get' }, params));
      },
      setDeviceImei(imei) {
        return this.postJSON('/api/settings_data', { action: 'set_imei', imei });
      },
      networkData(params = {}) {
        return this.postJSON('/api/network_data', params);
      },
      settingsData(params = {}) {
        return this.postJSON('/api/settings_data', params);
      },
      smsData(params = {}) {
        return this.postJSON('/api/sms_data', params);
      },
      getAT(atcmd, options = {}) {
        const params = new URLSearchParams({ atcmd });
        if (options.force) params.set('force', '1');
        if (options.wait !== undefined) params.set('wait', options.wait ? '1' : '0');
        return request('/api/get_atcache', {
          method: 'POST',
          cache: 'no-store',
          headers: { 'Content-Type': 'application/x-www-form-urlencoded; charset=UTF-8' },
          body: params
        });
      },
      refreshAT(atcmd) {
        return this.getAT(atcmd, { force: true, wait: true });
      },
      getATText(atcmd, options = {}) {
        return this.getAT(atcmd, options).then((response) => response.text());
      },
      getUptime() {
        return request('/api/get_uptime');
      },
      getPing() {
        return request('/api/get_ping');
      },
      getTTLStatus() {
        return request('/api/get_ttl_status');
      },
      setTTL(ttlvalue) {
        return request(this.url('/api/set_ttl', { ttlvalue }));
      },
      getLanguage() {
        return request('/api/get_language', { cache: 'no-store' });
      },
      setLanguage(language) {
        return request(this.url('/api/set_language', { language }), { method: 'POST' });
      },
      setPassword(currentPassword, newPassword, confirmPassword) {
        const params = new URLSearchParams({
          current_password: currentPassword || '',
          new_password: newPassword || '',
          confirm_password: confirmPassword || ''
        });
        return fetch('/api/set_password', {
          method: 'POST',
          cache: 'no-store',
          headers: { 'Content-Type': 'application/x-www-form-urlencoded; charset=UTF-8' },
          body: params
        });
      }
    };
  })();

  root.Brand = root.Brand || (function () {
    const storageKey = 'simpleadmin.moduleModelName';
    let currentName = '';
    let currentPageTitle = '';
    let refreshPromise = null;

    function normalizeModel(value) {
      const text = String(value === undefined || value === null ? '' : value).replace(/\s+/g, ' ').trim();
      if (!text || text === '-' || text === '未知' || text === '获取中...') return '';
      return text;
    }

    function cachedModel() {
      try {
        return normalizeModel(global.localStorage.getItem(storageKey));
      } catch (_) {
        return '';
      }
    }

    function cacheModel(value) {
      const model = normalizeModel(value);
      if (!model) return;
      try {
        global.localStorage.setItem(storageKey, model);
      } catch (_) {
        // localStorage 不可用时只更新当前页面显示。
      }
    }

    function markFromName(name) {
      const model = normalizeModel(name);
      const chars = Array.from(model.replace(/[^0-9A-Za-z\u4e00-\u9fa5]/g, ''));
      if (chars.length === 0) return '--';
      return chars.slice(0, 2).join('').toUpperCase();
    }

    function setText(selector, value) {
      document.querySelectorAll(selector).forEach((element) => {
        element.textContent = value;
      });
    }

    function updateDocumentTitle() {
      const brandName = currentName || cachedModel() || 'SimpleAdmin';
      if (currentPageTitle) {
        document.title = `${currentPageTitle} - ${brandName}`;
        return;
      }
      document.title = brandName;
    }

    function apply(name) {
      const model = normalizeModel(name) || cachedModel() || 'SimpleAdmin';
      currentName = model;
      cacheModel(model);
      setText('[data-simpleadmin-brand-title]', model);
      setText('[data-simpleadmin-brand-mark]', markFromName(model));
      updateDocumentTitle();
      return model;
    }

    function refresh(force) {
      if (refreshPromise) return refreshPromise;
      const url = `/api/module_model${force === false ? '' : '?force=1'}`;
      refreshPromise = fetch(url, {
        cache: 'no-store',
        credentials: 'same-origin'
      }).then((response) => {
        if (!response.ok) throw new Error(`module model status ${response.status}`);
        return response.json();
      }).then((data) => {
        const model = normalizeModel(data && data.model);
        if (model) apply(model);
        return model || currentName || cachedModel() || '';
      }).catch(() => {
        if (!currentName) apply(cachedModel() || 'SimpleAdmin');
        return currentName || '';
      }).finally(() => {
        refreshPromise = null;
      });
      return refreshPromise;
    }

    function init() {
      apply(cachedModel() || '获取中...');
      refresh(true);
    }

    function setPageTitle(title) {
      currentPageTitle = String(title || '').trim();
      updateDocumentTitle();
    }

    return {
      init,
      refresh,
      apply,
      setPageTitle,
      getName() {
        return currentName || cachedModel() || '';
      }
    };
  })();

  root.Text = root.Text || {
    lines(value) {
      return String(value || '')
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter(Boolean);
    },
    findStartsWith(lines, prefix) {
      return (lines || []).find((line) => line.startsWith(prefix));
    },
    findAllStartsWith(lines, prefix) {
      return (lines || []).filter((line) => line.startsWith(prefix));
    },
    splitCsv(value) {
      return (String(value || '').match(/"[^"]*"|[^,]+/g) || [])
        .map((item) => item.trim().replace(/^"|"$/g, ''));
    },
    hexToUtf16BE(hex) {
      const clean = String(hex || '').replace(/\s+/g, '');
      if (!clean || clean.length % 2 !== 0 || !/^[0-9a-fA-F]+$/.test(clean)) return '';
      let text = '';
      for (let i = 0; i < clean.length; i += 4) {
        const code = parseInt(clean.substr(i, 4), 16);
        if (!Number.isNaN(code)) text += String.fromCharCode(code);
      }
      return text;
    },
    compactHex(value) {
      return String(value || '').replace(/\s+/g, '');
    }
  };

  root.Mask = root.Mask || (function () {
    const emptyValues = new Set(['', '-', '未知', '未插卡', '无本机号码', '获取中...']);
    const sensitiveVisibleStorageKey = 'zbimsSensitiveVisible';

    function normalize(value) {
      return String(value === undefined || value === null ? '' : value).trim();
    }

    function shouldKeepRaw(value) {
      const text = normalize(value);
      return emptyValues.has(text);
    }

    function getSensitiveVisible() {
      try {
        return global.localStorage.getItem(sensitiveVisibleStorageKey) === '1';
      } catch (error) {
        return false;
      }
    }

    function setSensitiveVisible(visible) {
      const nextVisible = Boolean(visible);
      try {
        global.localStorage.setItem(sensitiveVisibleStorageKey, nextVisible ? '1' : '0');
      } catch (error) {
        // localStorage 不可用时只保留当前页面状态。
      }
      return nextVisible;
    }

    function maskMiddle(value, left = 3, right = 4) {
      const text = normalize(value);
      if (shouldKeepRaw(text)) return text || '-';
      if (text.length <= left + right) return '●'.repeat(Math.max(4, text.length));
      return `${text.slice(0, left)}${'●'.repeat(6)}${text.slice(-right)}`;
    }

    function maskIPv4(value) {
      const text = normalize(value);
      if (shouldKeepRaw(text)) return text || '-';
      return text.replace(/\b(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\b/g, '$1.$2.●●●.●●●');
    }

    function maskIPv6(value) {
      const text = normalize(value);
      if (shouldKeepRaw(text)) return text || '-';
      const parts = text.split(':');
      if (parts.length < 3) return maskMiddle(text, 4, 4);
      return `${parts.slice(0, 2).join(':')}:●●●●:${parts.slice(-1)[0]}`;
    }

    function maskPhone(value) {
      return maskMiddle(value, 3, 4);
    }

    function format(visible, value, type) {
      const text = normalize(value);
      if (visible || shouldKeepRaw(text)) return text || '-';
      switch (type) {
        case 'ipv4':
          return maskIPv4(text);
        case 'ipv6':
          return maskIPv6(text);
        case 'phone':
          return maskPhone(text);
        case 'digit':
          return maskMiddle(text, 3, 4);
        case 'id':
        default:
          return maskMiddle(text, 3, 3);
      }
    }

    return {
      format,
      getSensitiveVisible,
      setSensitiveVisible
    };
  })();

  root.Time = root.Time || {
    parseSmsDate(value) {
      const dateStr = String(value || '').replace(/\+\d{2}$/, '');
      const parts = dateStr.split(',');
      if (parts.length !== 2) return new Date(NaN);
      const dateParts = parts[0].split('/').map(Number);
      const timeParts = parts[1].split(':').map(Number);
      if (dateParts.length !== 3 || timeParts.length !== 3) return new Date(NaN);
      const [day, month, year] = dateParts;
      const [hour, minute, second] = timeParts;
      return new Date(Date.UTC(2000 + year, month - 1, day, hour, minute, second));
    },
    formatDateTime(date) {
      const pad = (value) => value.toString().padStart(2, '0');
      return `${date.getUTCFullYear()}/${pad(date.getUTCMonth() + 1)}/${pad(date.getUTCDate())} ${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}:${pad(date.getUTCSeconds())}`;
    }
  };

  root.Sms = root.Sms || {
    parseConcatHeader(hex) {
      const normalized = root.Text.compactHex(hex).toUpperCase();
      if (!normalized || !/^[0-9A-F]+$/.test(normalized)) return null;
      if (normalized.startsWith('050003') && normalized.length > 12) {
        const total = parseInt(normalized.slice(8, 10), 16);
        const seq = parseInt(normalized.slice(10, 12), 16);
        if (total > 1 && seq >= 1 && seq <= total) {
          return { ref: normalized.slice(6, 8), total, seq, bodyHex: normalized.slice(12) };
        }
      }
      if (normalized.startsWith('060804') && normalized.length > 14) {
        const total = parseInt(normalized.slice(10, 12), 16);
        const seq = parseInt(normalized.slice(12, 14), 16);
        if (total > 1 && seq >= 1 && seq <= total) {
          return { ref: normalized.slice(6, 10), total, seq, bodyHex: normalized.slice(14) };
        }
      }
      return null;
    },
    decodeUcs2(hex) {
      return root.Text.hexToUtf16BE(hex);
    },
    encodeUcs2(text) {
      return Array.from(String(text || ''))
        .map((char) => char.charCodeAt(0).toString(16).padStart(4, '0').toUpperCase())
        .join('');
    }
  };


  root.UI = root.UI || {
    setText(selectorOrElement, value) {
      const element = typeof selectorOrElement === 'string'
        ? document.querySelector(selectorOrElement)
        : selectorOrElement;
      if (element) element.textContent = value;
    },
    initDarkMode(buttonId) {
      const html = document.documentElement;
      if (!html) return;

      const storageKey = 'theme';
      const normalizeTheme = (theme) => theme === 'dark' ? 'dark' : 'light';
      const applyTheme = (theme) => {
        const normalized = normalizeTheme(theme);
        html.setAttribute('data-bs-theme', normalized);
        html.style.colorScheme = normalized;
        html.classList.toggle('theme-dark', normalized === 'dark');
        html.classList.toggle('theme-light', normalized === 'light');
        if (document.body) {
          document.body.setAttribute('data-bs-theme', normalized);
          document.body.classList.toggle('theme-dark', normalized === 'dark');
          document.body.classList.toggle('theme-light', normalized === 'light');
        }
        localStorage.setItem(storageKey, normalized);

        const label = normalized === 'dark' ? '浅色模式' : '暗夜模式';
        document.querySelectorAll('.sa-titlebar-theme-toggle').forEach((currentToggle) => {
          currentToggle.dataset.simpleadminI18nKey = label;
          currentToggle.textContent = root.Lang ? root.Lang.t(label) : label;
        });
        const legacyToggle = document.getElementById(buttonId || 'darkModeToggle');
        if (legacyToggle) {
          legacyToggle.dataset.simpleadminI18nKey = label;
          legacyToggle.textContent = root.Lang ? root.Lang.t(label) : label;
        }
      };

      applyTheme(localStorage.getItem(storageKey) || html.getAttribute('data-bs-theme') || 'light');

      const toggles = Array.from(document.querySelectorAll('.sa-titlebar-theme-toggle'));
      const legacyToggle = document.getElementById(buttonId || 'darkModeToggle');
      if (legacyToggle && !toggles.includes(legacyToggle)) toggles.push(legacyToggle);
      toggles.forEach((toggle) => {
        if (!toggle || toggle.dataset.simpleadminDarkModeBound === '1') return;
        toggle.dataset.simpleadminDarkModeBound = '1';
        toggle.addEventListener('click', () => {
          applyTheme(html.getAttribute('data-bs-theme') === 'dark' ? 'light' : 'dark');
        });
      });
    }
  };

  root.MockAT = root.MockAT || (function () {
    function normalizePayload(payload) {
      if (Array.isArray(payload)) return payload.join('\n');
      return String(payload == null ? '' : payload);
    }

    function logResult(label, result) {
      if (label === 'parse') {
        console.log('[SimpleAdmin MockAT parse]', result);
      } else {
        console.log('[SimpleAdmin MockAT]', result);
      }
      return result;
    }

    function request(params) {
      return root.Api.mockAT(params).then((result) => {
        if (result && result.ok === false) {
          console.warn('[SimpleAdmin MockAT]', result.error || result);
        }
        return result;
      });
    }

    function refreshDashboard() {
      const apps = root.Vue && root.Vue.apps;
      const app = (apps && apps['#dashboardApp']) || (root.Vue && root.Vue.currentApp);
      if (app && typeof app.fetchNetworkInfo === 'function') {
        app.fetchNetworkInfo();
      }
    }

    const api = {
      help() {
        const text = [
          'SimpleAdmin MockAT browser console commands:',
          '  SimpleAdmin.MockAT.at(`paste full dashboard AT response here`)',
          '  SimpleAdmin.MockAT.qca(`paste QCAINFO response here`)',
          '  SimpleAdmin.MockAT.qeng(`paste QENG response here`)',
          '  SimpleAdmin.MockAT.parse()   // print backend parsed dashboard JSON',
          '  SimpleAdmin.MockAT.show()    // show current mock payload status',
          '  SimpleAdmin.MockAT.clear()   // clear all; or clear("at"/"qca"/"qeng")',
          'Aliases: saAt(`...`), saQca(`...`), saQeng(`...`), saParseAT(), saShowAT(), saClearAT()'
        ].join('\n');
        console.log(text);
        return text;
      },
      set(kind, payload) {
        return request({ action: 'set', kind, payload: normalizePayload(payload) })
          .then((result) => {
            refreshDashboard();
            return logResult('set', result);
          });
      },
      at(payload) { return this.set('at', payload); },
      dashboard(payload) { return this.set('dashboard', payload); },
      qca(payload) { return this.set('qca', payload); },
      qcainfo(payload) { return this.set('qcainfo', payload); },
      qeng(payload) { return this.set('qeng', payload); },
      clear(kind = '') {
        return request({ action: 'clear', kind })
          .then((result) => {
            refreshDashboard();
            return logResult('clear', result);
          });
      },
      show() {
        return request({ action: 'show' }).then((result) => logResult('show', result));
      },
      parse(options = {}) {
        const params = Object.assign({ action: 'parse' }, options || {});
        return request(params).then((result) => logResult('parse', result));
      }
    };

    return api;
  })();

  global.saAt = function (payload) { return root.MockAT.at(payload); };
  global.saQca = function (payload) { return root.MockAT.qca(payload); };
  global.saQeng = function (payload) { return root.MockAT.qeng(payload); };
  global.saParseAT = function (options) { return root.MockAT.parse(options); };
  global.saShowAT = function () { return root.MockAT.show(); };
  global.saClearAT = function (kind) { return root.MockAT.clear(kind); };


  root.Logout = root.Logout || (function () {
    function start() {
      const finish = () => { global.location.replace('/login.html'); };
      if (global.fetch) {
        global.fetch('/api/logout', {
          method: 'POST',
          cache: 'no-store',
          credentials: 'same-origin'
        }).then(finish).catch(finish);
        return;
      }
      global.location.replace('/api/logout');
    }

    return { start };
  })();

  root.Vue = root.Vue || {};


  global.SimpleAdmin = root;

  if (root.Brand && typeof root.Brand.init === 'function') {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', root.Brand.init, { once: true });
    } else {
      root.Brand.init();
    }
  }

})(window);
