function fetchSMS() {
  return {
    isLoading: false,
    messages: [],
    senders: [],
    dates: [],
    selectedMessages: [],
    phoneNumber: '',
    messageToSend: '',
    messageIndices: [],
    serviceCenters: [],
    activeMessageIndex: null,
    smsDetailGlobalHandlersBound: false,
    smsAutoRefreshTimer: null,
    smsAutoRefreshInFlight: false,
    smsAutoRefreshIntervalMs: 5000,
    smsListSignature: '',
    smsIndexSignature: '',
    smsPendingData: null,
    smsPendingIndexSignature: '',
    smsPendingListSignature: '',
    smsPendingStableCount: 0,
    smsPendingFirstSeenAt: 0,
    smsStablePollsRequired: 2,
    smsPendingMaxWaitMs: 15000,

    clearData() {
      this.messages = [];
      this.senders = [];
      this.dates = [];
      this.selectedMessages = [];
      this.messageIndices = [];
      this.smsListSignature = '';
      this.smsIndexSignature = '';
      this.resetSMSPendingRefresh();
      this.activeMessageIndex = null;
      this.setMessageDetailOpenState(false);
      this.syncSelectAllCheckbox();
    },

    requestSMS(options = {}) {
      const silent = options.silent === true;
      const params = { action: 'list' };
      if (options.force === true) params.force = '1';
      if (!silent) this.isLoading = true;
      return SimpleAdmin.Api.smsData(params)
        .then((data) => {
          this.applySMSData(data, {
            keepDetail: options.keepDetail === true,
            keepSelection: options.keepSelection === true
          });
        })
        .finally(() => {
          if (!silent) this.isLoading = false;
        });
    },

    applySMSData(data, options = {}) {
      const activeKey = options.keepDetail === true ? this.currentMessageKey(this.activeMessageIndex) : '';
      const selectedKeys = options.keepSelection === true
        ? this.selectedMessages.map((index) => this.currentMessageKey(index)).filter(Boolean)
        : [];

      this.messages = [];
      this.senders = [];
      this.dates = [];
      this.selectedMessages = [];
      this.messageIndices = [];
      this.serviceCenters = data.serviceCenters || [];
      (data.messages || []).forEach((msg) => {
        const date = msg.date ? this.parseCustomDate(String(msg.date).replace(/\+\d{2}$/, '')) : new Date(NaN);
        this.pushSMSMessage(
          msg.sender || '',
          Number.isNaN(date.getTime()) ? new Date() : date,
          msg.text || '',
          msg.indices || [],
          Array.isArray(msg.textLines) ? msg.textLines : []
        );
      });

      this.smsListSignature = this.makeSMSListSignature(data);
      this.smsIndexSignature = this.makeSMSIndexSignature(data);
      this.resetSMSPendingRefresh();
      this.restoreSelectionByKeys(selectedKeys);
      this.restoreActiveMessageByKey(activeKey);
      this.syncSelectAllCheckbox();
    },

    pushSMSMessage(sender, date, text, indices, textLines = []) {
      const normalizedText = this.normalizeMessageText(text);
      const lines = this.normalizeMessageLines(normalizedText, textLines);
      this.messageIndices.push(indices);
      this.senders.push(sender);
      this.dates.push(this.formatDate(date));
      this.messages.push({ text: normalizedText, lines, sender, date, indices });
    },

    normalizeMessageText(text) {
      return String(text || '')
        .replace(/\r\n/g, '\n')
        .replace(/\r/g, '\n')
        .replace(/[\v\f\u0085\u2028\u2029]/g, '\n')
        .replace(/\\r\\n/g, '\n')
        .replace(/\\n/g, '\n')
        .replace(/\\r/g, '\n');
    },

    normalizeMessageLines(text, textLines = []) {
      if (Array.isArray(textLines) && textLines.length > 0) {
        const lines = [];
        textLines.forEach((line) => {
          this.normalizeMessageText(line).split('\n').forEach((part) => {
            lines.push(part);
          });
        });
        return lines;
      }
      return this.normalizeMessageText(text).split('\n');
    },

    getMessagePreview(message) {
      const text = Array.isArray(message?.lines) ? message.lines.join(' ') : message?.text;
      return String(text || '').replace(/\s+/g, ' ').trim() || '（无内容）';
    },

    makeSMSListSignature(data) {
      return JSON.stringify((data.messages || []).map((msg) => [
        msg.sender || '',
        msg.date || '',
        msg.text || '',
        Array.isArray(msg.indices) ? msg.indices.join(',') : '',
        Array.isArray(msg.textLines) ? msg.textLines.join('\n') : ''
      ]));
    },

    makeSMSIndexSignature(data) {
      const indices = [];
      (data.messages || []).forEach((msg) => {
        if (!Array.isArray(msg.indices)) return;
        msg.indices.forEach((index) => {
          const n = Number(index);
          if (Number.isFinite(n)) indices.push(n);
        });
      });
      indices.sort((a, b) => a - b);
      return indices.join(',');
    },

    hasIncompleteMultipartSMS(data) {
      return (data.messages || []).some((msg) => {
        const total = Number(msg.concatTotal || 0);
        if (!Number.isFinite(total) || total <= 1) return false;
        const indices = Array.isArray(msg.indices) ? msg.indices : [];
        return indices.length < total;
      });
    },

    resetSMSPendingRefresh() {
      this.smsPendingData = null;
      this.smsPendingIndexSignature = '';
      this.smsPendingListSignature = '';
      this.smsPendingStableCount = 0;
      this.smsPendingFirstSeenAt = 0;
    },

    messageKey(message, sender, dateText) {
      if (!message) return '';
      const indices = Array.isArray(message.indices) ? message.indices.join(',') : '';
      return [indices, sender || message.sender || '', dateText || '', message.text || ''].join('|');
    },

    currentMessageKey(index) {
      if (index === null || index === undefined) return '';
      return this.messageKey(this.messages[index], this.senders[index], this.dates[index]);
    },

    restoreSelectionByKeys(keys) {
      if (!Array.isArray(keys) || keys.length === 0) return;
      const keySet = new Set(keys);
      this.selectedMessages = this.messages
        .map((_, index) => index)
        .filter((index) => keySet.has(this.currentMessageKey(index)));
    },

    restoreActiveMessageByKey(key) {
      if (!key) {
        this.activeMessageIndex = null;
        this.setMessageDetailOpenState(false);
        return;
      }
      const nextIndex = this.messages.findIndex((_, index) => this.currentMessageKey(index) === key);
      if (nextIndex >= 0) {
        this.activeMessageIndex = nextIndex;
        this.setMessageDetailOpenState(true);
        return;
      }
      this.activeMessageIndex = null;
      this.setMessageDetailOpenState(false);
    },

    syncSelectAllCheckbox() {
      const selectAllCheckbox = document.getElementById('selectAllCheckbox');
      if (!selectAllCheckbox) return;
      const hasMessages = this.messages.length > 0;
      selectAllCheckbox.checked = hasMessages && this.selectedMessages.length === this.messages.length;
    },

    bindMessageDetailGlobalHandlers() {
      if (this.smsDetailGlobalHandlersBound) return;
      this.smsDetailGlobalHandlersBound = true;
      window.addEventListener('simpleadmin:page-changed', (event) => {
        if (event?.detail?.page === 'sms') {
          this.startSMSAutoRefresh();
          return;
        }
        this.stopSMSAutoRefresh();
        this.closeMessageDetail();
      });
      window.addEventListener('keydown', (event) => {
        if (event.key === 'Escape' && this.activeMessageIndex !== null) {
          this.closeMessageDetail();
        }
      });
    },

    isSMSPageActive() {
      const page = document.querySelector('.sa-page[data-page="sms"]');
      return !!page && page.classList.contains('active');
    },

    startSMSAutoRefresh() {
      if (!this.isSMSPageActive()) return;
      if (this.smsAutoRefreshTimer) return;
      this.smsAutoRefreshTimer = setInterval(() => {
        this.autoRefreshSMS();
      }, this.smsAutoRefreshIntervalMs);
    },

    stopSMSAutoRefresh() {
      if (this.smsAutoRefreshTimer) {
        clearInterval(this.smsAutoRefreshTimer);
        this.smsAutoRefreshTimer = null;
      }
      this.smsAutoRefreshInFlight = false;
      this.resetSMSPendingRefresh();
    },

    autoRefreshSMS() {
      if (!this.isSMSPageActive()) {
        this.stopSMSAutoRefresh();
        return;
      }
      if (this.smsAutoRefreshInFlight || this.isLoading) return;
      this.smsAutoRefreshInFlight = true;
      SimpleAdmin.Api.smsData({ action: 'list_meta', force: '1' })
        .then((data) => {
          if (!this.isSMSPageActive()) return null;
          return this.handlePolledSMSMeta(data);
        })
        .catch(() => {
          // Keep the timer alive; the next interval can retry.
        })
        .finally(() => {
          this.smsAutoRefreshInFlight = false;
        });
    },

    makeSMSMetaSignature(data) {
      return JSON.stringify((data.messages || []).map((msg) => [
        msg.sender || '',
        msg.date || '',
        Array.isArray(msg.indices) ? msg.indices.join(',') : '',
        msg.concatTotal || '',
        msg.concatRef || '',
        msg.concatSeq || ''
      ]));
    },

    handlePolledSMSMeta(data) {
      const indexSignature = this.makeSMSIndexSignature(data);
      if (indexSignature === this.smsIndexSignature) {
        this.resetSMSPendingRefresh();
        return null;
      }

      const metaSignature = this.makeSMSMetaSignature(data);
      const now = Date.now();
      if (indexSignature === this.smsPendingIndexSignature && metaSignature === this.smsPendingListSignature) {
        this.smsPendingStableCount += 1;
      } else {
        this.smsPendingData = data;
        this.smsPendingIndexSignature = indexSignature;
        this.smsPendingListSignature = metaSignature;
        this.smsPendingStableCount = 1;
        this.smsPendingFirstSeenAt = now;
      }

      const waited = now - this.smsPendingFirstSeenAt;
      const incompleteMultipart = this.hasIncompleteMultipartSMS(this.smsPendingData || data);
      if (incompleteMultipart && waited < this.smsPendingMaxWaitMs) return null;
      if (this.smsPendingStableCount < this.smsStablePollsRequired && waited < this.smsPendingMaxWaitMs) return null;

      return this.fetchStableSMSList(this.smsPendingIndexSignature);
    },

    fetchStableSMSList(expectedIndexSignature) {
      return SimpleAdmin.Api.smsData({ action: 'list', force: '1' })
        .then((data) => {
          if (!this.isSMSPageActive()) return;
          const indexSignature = this.makeSMSIndexSignature(data);
          if (indexSignature === this.smsIndexSignature) {
            this.resetSMSPendingRefresh();
            return;
          }
          if (expectedIndexSignature && indexSignature !== expectedIndexSignature) {
            this.smsPendingData = data;
            this.smsPendingIndexSignature = indexSignature;
            this.smsPendingListSignature = this.makeSMSMetaSignature(data);
            this.smsPendingStableCount = 1;
            this.smsPendingFirstSeenAt = Date.now();
            return;
          }
          this.applySMSData(data, { keepDetail: true, keepSelection: true });
        });
    },

    setMessageDetailOpenState(isOpen) {
      if (document.body) {
        document.body.classList.toggle('sa-sms-detail-open', isOpen);
      }
    },

    openMessageDetail(index) {
      this.activeMessageIndex = index;
      this.setMessageDetailOpenState(true);
      if (typeof this.$nextTick === 'function') {
        this.$nextTick(() => {
          const closeButton = document.querySelector('.sa-sms-detail-modal .btn-close');
          if (closeButton) closeButton.focus({ preventScroll: true });
        });
      }
    },

    closeMessageDetail() {
      this.activeMessageIndex = null;
      this.setMessageDetailOpenState(false);
    },

    parseCustomDate(dateStr) {
      const [datePart, timePart] = String(dateStr || '').split(',');
      if (!datePart || !timePart) return new Date(NaN);
      const [day, month, year] = datePart.split('/').map((part) => parseInt(part, 10));
      const [hour, minute, second] = timePart.split(':').map((part) => parseInt(part, 10));
      return new Date(Date.UTC(2000 + year, month - 1, day, hour, minute, second));
    },

    formatDate(date) {
      const year = date.getUTCFullYear() - 2000;
      const month = (date.getUTCMonth() + 1).toString().padStart(2, '0');
      const day = date.getUTCDate().toString().padStart(2, '0');
      const hour = date.getUTCHours().toString().padStart(2, '0');
      const minute = date.getUTCMinutes().toString().padStart(2, '0');
      const second = date.getUTCSeconds().toString().padStart(2, '0');
      return `${day}/${month}/${year},${hour}:${minute}:${second}`;
    },

    deleteSelectedSMS() {
      if (this.selectedMessages.length === 0) {
        console.warn('没有选中的短信');
        return;
      }

      if (!this.messageIndices || this.messageIndices.length === 0) {
        console.error('短信索引未正确初始化或为空');
        return;
      }

      const isAllSelected = this.selectedMessages.length === this.messages.length;
      if (isAllSelected) {
        this.deleteAllSMS();
        return;
      }

      const indicesToDelete = [];
      this.selectedMessages.forEach((index) => {
        indicesToDelete.push(...this.messages[index].indices);
      });

      if (indicesToDelete.length === 0) {
        console.warn('没有有效的短信索引');
        return;
      }

      SimpleAdmin.Api.smsData({ action: 'delete_indices', indices: indicesToDelete.join(',') })
        .finally(() => {
          this.selectedMessages = [];
          this.requestSMS({ force: true });
        });
    },

    deleteAllSMS() {
      SimpleAdmin.Api.smsData({ action: 'delete_all' })
        .finally(() => {
          this.init();
        });
    },

    async sendSMS() {
      if (!this.phoneNumber || !this.messageToSend) {
        this.showNotification('号码或内容不能为空', 'warning');
        return;
      }

      try {
        const simStatus = await SimpleAdmin.Api.smsData({ action: 'sim_status' });
        if (!simStatus.inserted) {
          this.showNotification('未检测到 SIM 卡', 'danger');
          return;
        }
      } catch { }

      try {
        const result = await SimpleAdmin.Api.smsData({
          action: 'send',
          number: this.phoneNumber,
          message: this.messageToSend
        });
        if (result.ok) {
          this.showNotification('短信发送成功！', 'success');
          return;
        }
        this.showNotification(`短信发送失败：${result.error || '未知错误'}`, 'danger');
      } catch (error) {
        this.showNotification(`短信发送失败：${error?.message || '网络错误'}`, 'danger');
      }
    },

    showNotification(message, type = 'info') {
      const n = document.getElementById('notification');
      const translated = SimpleAdmin.Lang ? SimpleAdmin.Lang.t(message) : message;
      n.innerText = translated;
      n.dataset.simpleadminI18nKey = message;
      n.className = `alert alert-${type}`;
      n.style.display = 'block';
      setTimeout(() => { n.style.display = 'none'; }, 3000);
    },

    init() {
      this.bindMessageDetailGlobalHandlers();
      this.clearData();
      this.requestSMS({ force: true })
        .finally(() => {
          this.startSMSAutoRefresh();
        });
    },

    toggleAll(event) {
      this.selectedMessages = event.target.checked ? this.messages.map((_, index) => index) : [];
      this.syncSelectAllCheckbox();
    }
  };
}

if (window.SimpleAdminSpaMode) {
  (window.SimpleAdmin.Pages = window.SimpleAdmin.Pages || {}).sms = fetchSMS;
} else {
  mountSimpleAdminVueApp(fetchSMS);
}
