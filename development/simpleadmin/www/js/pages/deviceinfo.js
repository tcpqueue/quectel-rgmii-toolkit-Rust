function fetchDeviceInfo() {
        const DEVICE_INFO_RETRY_DELAY_MS = 1500;
        const DEVICE_INFO_AUTO_REFRESH_DELAY_MS = 5000;
        const DEVICE_INFO_RETRY_LIMIT = 10;

        return {
          // 可见状态
          manufacturer: "-",
          modelName: "-",
          firmwareVersion: "-",
          simStatus: "未知",
          imsi: "-",
          iccid: "-",
          imei: "-",
          lanIp: "-",
          wwanIpv4: "-",
          wwanIpv6: "-",
          phoneNumber: "-",

          // UI 状态
          isLoading: false,
          sensitiveVisible: SimpleAdmin.Mask.getSensitiveVisible(),

          // 内部状态与并发控制
          _inflightMap: new Map(),   // key: atcmd, val: Promise<string>
          _loadingDeviceInfo: false, // 本地锁：设备信息查询
          _didInitialFetch: false,   // init() 只跑一次
          _deviceInfoRetryTimer: null, // 设备信息自动刷新定时器
          _deviceInfoRetryCount: 0,  // 后台缓存未就绪时的快速重试次数
          _loadingCount: 0,          // 全局加载计数（更稳地控制 isLoading）
          _deviceInfoActive: false,  // 只在设备信息页可见时刷新
          _pageChangeHandler: null,  // SPA 页面切换监听

          toggleSensitiveVisible() {
            this.sensitiveVisible = SimpleAdmin.Mask.setSensitiveVisible(!this.sensitiveVisible);
          },

          formatSensitiveValue(value, type) {
            return SimpleAdmin.Mask.format(this.sensitiveVisible, value, type);
          },

          isDeviceInfoPageActive() {
            if (!window.SimpleAdminSpaMode) return true;
            const section = document.querySelector('.sa-page[data-page="deviceinfo"]');
            return !!(section && section.classList.contains('active'));
          },

          startDeviceInfoRefresh() {
            if (!this.isDeviceInfoPageActive()) return;
            this._deviceInfoActive = true;
            clearTimeout(this._deviceInfoRetryTimer);
            this.fetchATCommand();
          },

          stopDeviceInfoRefresh() {
            this._deviceInfoActive = false;
            this._deviceInfoRetryCount = 0;
            clearTimeout(this._deviceInfoRetryTimer);
            this._deviceInfoRetryTimer = null;
          },

          // 批量获取设备信息（仅在停留设备信息页时刷新）
          async fetchATCommand() {
            if (!this._deviceInfoActive || !this.isDeviceInfoPageActive()) {
              this.stopDeviceInfoRefresh();
              return;
            }
            if (this._loadingDeviceInfo) return; // 已在拉取，忽略重复
            this._loadingDeviceInfo = true;
            let nextRefreshDelay = DEVICE_INFO_AUTO_REFRESH_DELAY_MS;
            try {
              this._loadingCount++;
              this.isLoading = true;
              const data = await SimpleAdmin.Api.getDeviceInfo({ force: false });
              Object.assign(this, {
                manufacturer: data.manufacturer || '-',
                modelName: data.modelName || '-',
                firmwareVersion: data.firmwareVersion || '-',
                simStatus: data.simStatus || '未知',
                imsi: data.imsi || '-',
                iccid: data.iccid || '-',
                imei: data.imei || '-',
                lanIp: data.lanIp || '-',
                wwanIpv4: data.wwanIpv4 || '-',
                wwanIpv6: data.wwanIpv6 || '-',
                phoneNumber: data.phoneNumber || '未插卡'
              });
              const needFastRetry = (!data.imei || data.imei === '-' || data.pending) && this._deviceInfoRetryCount < DEVICE_INFO_RETRY_LIMIT;
              if (needFastRetry) {
                this._deviceInfoRetryCount++;
                nextRefreshDelay = DEVICE_INFO_RETRY_DELAY_MS;
              } else {
                this._deviceInfoRetryCount = 0;
              }
            } catch (err) {
              console.warn('设备信息自动刷新失败：', err);
            } finally {
              this._loadingCount--;
              this.isLoading = this._loadingCount > 0;
              this._loadingDeviceInfo = false;
            }

            if (!this._deviceInfoActive || !this.isDeviceInfoPageActive()) {
              this.stopDeviceInfoRefresh();
              return;
            }
            clearTimeout(this._deviceInfoRetryTimer);
            this._deviceInfoRetryTimer = setTimeout(() => {
              this.fetchATCommand();
            }, nextRefreshDelay);
          },


          // init 只执行一次；设备信息页可见时才启动自动刷新
          init() {
            if (this._didInitialFetch) return;
            this._didInitialFetch = true;
            clearTimeout(this._deviceInfoRetryTimer);
            this._pageChangeHandler = (event) => {
              if (event && event.detail && event.detail.page === 'deviceinfo') {
                this.startDeviceInfoRefresh();
              } else {
                this.stopDeviceInfoRefresh();
              }
            };
            window.addEventListener('simpleadmin:page-changed', this._pageChangeHandler);
            if (this.isDeviceInfoPageActive()) {
              this.startDeviceInfoRefresh();
            }
          },
        };
      }

    if (window.SimpleAdminSpaMode) {
    (window.SimpleAdmin.Pages = window.SimpleAdmin.Pages || {}).deviceinfo = fetchDeviceInfo;
  } else {
    mountSimpleAdminVueApp(fetchDeviceInfo);
  }
