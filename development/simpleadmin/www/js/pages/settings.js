function simpleSettings() {
      return {
        isLoading: false,
        showSuccess: false,
        showError: false,
        isClean: true,
        showModal: false,
        showImeiModal: false,
        isRebooting: false,
        atcmd: "",
        fetchATCommand: "",
        countdown: 0,
        atCommandResponse: "",
        ttldata: null,
        ttlvalue: 0,
        ttlStatus: false,
        newTTL: null,
        ipPassMode: "未指定",
        ipPassStatus: false,
        usbNetMode: "未指定",
        currentUsbNetMode: "未知",
        imei: "-",
        newImei: "",
        lanIpStart: "",  // 初始化 LAN IP 起始地址
        lanIpEnd: "",    // 初始化 LAN IP 结束地址
        lanGwIp: "",     // 初始化 网关 IP 地址
        isSavingLANIP: false,
        networkMessage: "",
        networkFailed: false,
        ttlMessage: "",
        lanIpSaveSuccess: false,
        lanIpSaveSuccessTimer: null,
        DNSV6ProxyStatus: true,
        DNSV4ProxyStatus: true,
        dmzMode: "0",
        dmzIP: "",
        isRebooted: true,
        language: SimpleAdmin.Lang ? SimpleAdmin.Lang.getCurrentLanguage() : "zh-CN",
        isSavingLanguage: false,
        languageSaveMessage: "",
        webUsername: "",
        webHttpPort: 80,
        webHttpEnabled: true,
        webuiCurrentPassword: "",
        isSavingWebui: false,
        webuiSaveMessage: "",
        webuiNewUrl: "",
        currentPassword: "",
        newPassword: "",
        confirmPassword: "",
        isSavingPassword: false,
        passwordSaveMessage: "",
        currentRootPassword: "",
        newRootPassword: "",
        confirmRootPassword: "",
        isSavingRootPassword: false,
        rootPasswordSaveMessage: "",
        rebootCountdownTimer: null,
        ota: { phase: 'idle' },
        otaForm: { mode: 'check', source: '', proxy: '', public_key: '' },
        otaFormLoaded: false,
        otaMessage: "",
        otaFailed: false,
        otaSaving: false,
        otaSaveMessage: "",
        otaPollTimer: null,
        otaRestartFrom: "",

        t(key) {
          return SimpleAdmin.Lang ? SimpleAdmin.Lang.t(key) : key;
        },

        fetchLanguageSetting() {
          if (!SimpleAdmin.Lang) return Promise.resolve(this.language);
          return SimpleAdmin.Lang.load().then((language) => {
            this.language = language;
            return language;
          });
        },

        resolveDmzMode(mode, ip) {
          const currentMode = String(mode || '').trim();
          const currentIp = String(ip || '').trim();
          if (currentIp && currentIp !== '-') {
            return '1';
          }
          return currentMode === '1' ? '1' : '0';
        },

        saveLanguageSetting() {
          if (!SimpleAdmin.Lang) return;
          this.isSavingLanguage = true;
          this.languageSaveMessage = "";
          SimpleAdmin.Lang.setLanguage(this.language)
            .then((language) => {
              this.language = language;
              this.languageSaveMessage = this.t("已保存");
              setTimeout(() => {
                this.languageSaveMessage = "";
              }, 3000);
            })
            .catch((error) => {
              console.error("保存语言设置失败：", error);
              this.languageSaveMessage = this.t("保存失败");
            })
            .finally(() => {
              this.isSavingLanguage = false;
            });
        },

        async changeRootPassword() {
          if(this.isSavingRootPassword) return;
          this.rootPasswordSaveMessage = '';
          if(!this.currentRootPassword || !this.newRootPassword || this.newRootPassword !== this.confirmRootPassword) {
            this.rootPasswordSaveMessage = '请填写当前 root 密码，并确认两次新密码一致'; return;
          }
          this.isSavingRootPassword = true;
          const controller = new AbortController();
          const deadline = setTimeout(() => controller.abort(), 40000);
          try {
            const response = await fetch('/api/set_root_password', {method:'POST', signal:controller.signal, headers:{'Content-Type':'application/x-www-form-urlencoded'}, body:new URLSearchParams({current_password:this.currentRootPassword,new_password:this.newRootPassword,confirm_password:this.confirmRootPassword})});
            if(response.status === 401) {window.location.replace('/login.html');return;}
            const data = await response.json();
            if(!response.ok) throw new Error(data.error || '保存失败');
            this.currentRootPassword = this.newRootPassword = this.confirmRootPassword = '';
            window.location.replace('/login.html');
          } catch(error) {this.rootPasswordSaveMessage = error.name === 'AbortError' ? '请求超时，请确认设备密码与挂载状态' : error.message;}
          finally {clearTimeout(deadline);this.isSavingRootPassword=false;}
        },

        async fetchWebuiSetting() {
          try {
            const response = await fetch('/api/webui_settings', {cache:'no-store'});
            if (!response.ok) throw new Error(this.t("读取 WebUI 设置失败"));
            const data = await response.json();
            this.webUsername = data.username;
            this.webHttpPort = data.http_port;
            this.webHttpEnabled = data.http_enabled;
          } catch(error) { this.webuiSaveMessage = error.message; }
        },

        async saveWebuiPort() {
          if (this.isSavingWebui) return;
          this.webuiSaveMessage = ""; this.webuiNewUrl = "";
          if (!this.webuiCurrentPassword || !/^[1-9][0-9]{0,4}$/.test(String(this.webHttpPort)) || Number(this.webHttpPort) > 65535) {
            this.webuiSaveMessage = this.t("请填写当前 Web 密码和 1–65535 的端口"); return;
          }
          this.isSavingWebui = true;
          const controller = new AbortController();
          const deadline = setTimeout(() => controller.abort(), 40000);
          try {
            const response = await fetch('/api/set_webui_port', {method:'POST',signal:controller.signal,
              headers:{'Content-Type':'application/x-www-form-urlencoded'},
              body:new URLSearchParams({current_password:this.webuiCurrentPassword,http_port:String(this.webHttpPort)})});
            const data = await response.json();
            if (!response.ok) {
              const errors = {"current password incorrect":"当前密码不正确","HTTP port is already in use or unavailable":"端口已被占用或不可用，原配置保持不变"};
              throw new Error(this.t(errors[data.error] || data.error || "保存失败"));
            }
            this.webuiCurrentPassword = "";
            this.webuiSaveMessage = this.t(data.changed ? "端口已保存并生效，无需重启模块" : "端口未改变");
            if (data.warning) this.webuiSaveMessage += " · " + data.warning;
            if (data.changed) {
              if (["127.0.0.1","localhost","[::1]"].includes(window.location.hostname)) {
                this.webuiSaveMessage += " · " + this.t("通过 ADB 转发访问时，请在安装器重新打开管理页面");
              } else {
                const url = new URL(window.location.href);
                url.protocol = "http:"; url.port = String(data.http_port); url.pathname = "/login.html"; url.search = ""; url.hash = "";
                this.webuiNewUrl = url.href;
              }
            }
          } catch(error) { this.webuiSaveMessage = error.name === 'AbortError' ? this.t("请求超时，请重新检查当前端口") : error.message; }
          finally { clearTimeout(deadline); this.isSavingWebui = false; }
        },

        changeLoginPassword() {
          if (this.isSavingPassword) return;
          this.passwordSaveMessage = "";
          if (!this.currentPassword || !this.webUsername) {
            this.passwordSaveMessage = this.t("请输入当前密码和 Web 账号");
            return;
          }
          if (this.newPassword !== this.confirmPassword) {
            this.passwordSaveMessage = this.t("两次输入的新密码不一致");
            return;
          }

          this.isSavingPassword = true;
          return SimpleAdmin.Api.setPassword(this.currentPassword, this.newPassword, this.confirmPassword, this.webUsername)
            .then((res) => {
              return res.json().catch(() => ({})).then((data) => ({ res, data }));
            })
            .then(({ res, data }) => {
              if (!res.ok) {
                const error = data && data.error ? data.error : "";
                if (res.status === 403 || error === "current password incorrect") {
                  throw new Error("当前密码不正确");
                }
                if (error === "new password is empty") {
                  throw new Error("新密码不能为空");
                }
                if (error === "password confirmation mismatch") {
                  throw new Error("密码确认不一致");
                }
                throw new Error(error || "密码保存失败");
              }
              this.currentPassword = "";
              this.newPassword = "";
              this.confirmPassword = "";
              this.passwordSaveMessage = this.t("密码已保存，请使用新密码重新登录。");
              window.location.replace('/login.html');
            })
            .catch((error) => {
              console.error("保存登录密码失败：", error);
              this.passwordSaveMessage = this.t(error.message || "密码保存失败");
            })
            .finally(() => {
              this.isSavingPassword = false;
            });
        },

        closeModal() {
          this.showModal = false;
        },

        closeImeiModal() {
          this.showImeiModal = false;
        },

        showRebootModal() {
          this.showModal = true;
        },

        sendATCommand() {
          if (!this.atcmd) {
            this.atcmd = "ATI";
          }
          this.isLoading = true;
          // ★ 返回 fetch 的 Promise，这样外层可以 .then(...)
          return SimpleAdmin.Api.settingsData({ action: 'manual_at', command: this.atcmd })
            .then((data) => data.response || '')
            .then((data) => {
              this.atCommandResponse = data;
              this.isLoading = false;
              this.isClean = false;
              //this.fetchCurrentSettings();
              return data; // ★ 把结果再传下去，链式调用更方便
            })
            .catch((error) => {
              console.error("错误: ", error);
              this.showError = true;
              this.isLoading = false;
              throw error; // ★ 继续抛出，方便调用方捕获
            });
        },

        clearResponses() {
          this.atCommandResponse = "";
          this.isClean = true;
        },

        startRebootCountdown(seconds = 40) {
          if (this.rebootCountdownTimer) {
            clearInterval(this.rebootCountdownTimer);
          }
          this.atCommandResponse = "";
          this.showModal = false;
          this.showImeiModal = false;
          this.isRebooting = true;
          this.countdown = seconds;
          this.isRebooted = false;  // 重启标志位，表示设备尚未重启完成

          // 进行倒计时
          this.rebootCountdownTimer = setInterval(() => {
            this.countdown--;
            if (this.countdown <= 0) {
              clearInterval(this.rebootCountdownTimer);
              this.rebootCountdownTimer = null;
              this.isRebooting = false;

              // 给设备一些时间重启后再执行初始化
              setTimeout(() => {
                this.isRebooted = true;  // 设置标志为已重启
                this.init();  // 重启后执行初始化（发送必要的 AT 命令）
              }, 5000);  // 延迟 5 秒，以确保设备完全重启
            }
          }, 1000);
        },

        handleRebootNotice(data) {
          if (!data || !(data.reboot || data.rebooting)) {
            return false;
          }
          const seconds = Number(data.rebootCountdownSeconds) || 40;
          this.startRebootCountdown(seconds);
          return true;
        },

        rebootDevice() {
          SimpleAdmin.Api.settingsData({ action: 'reboot' });
          this.startRebootCountdown(40);
        },


        openImeiModal() {
          const val = (this.newImei || '').trim();
          if (!val) {
            alert('没有提供新的 IMEI。');
            return;
          }
          if (val.length !== 15 || !/^\d+$/.test(val)) {
            alert('IMEI 无效');
            return;
          }
          if (this.imei !== '-' && val === this.imei) {
            alert('IMEI 与当前 IMEI 相同');
            return;
          }
          this.showImeiModal = true;
        },

        updateIMEI() {
          const val = (this.newImei || '').trim();
          this.showImeiModal = false;
          this.isLoading = true;
          SimpleAdmin.Api.settingsData({ action: 'set_imei', imei: val })
            .catch((error) => {
              console.info('设置 IMEI 后设备重启或连接断开，继续保持重启倒计时：', error);
            })
            .finally(() => {
              this.isLoading = false;
            });
          this.startRebootCountdown(40);
        },

        resetATCommands() {
          SimpleAdmin.Api.settingsData({ action: 'reset_at' });
          this.atCommandResponse = "";
          this.showRebootModal();
        },

        setNetworkMessage(message, failed) {
          this.networkMessage = message;
          this.networkFailed = Boolean(failed);
        },

        // Sends one network feature change and reports the outcome under the list.
        async runNetworkAction(params) {
          this.setNetworkMessage("", false);
          this.isLoading = true;
          try {
            const data = await SimpleAdmin.Api.settingsData(params);
            if (!data || data.ok === false) {
              throw new Error((data && (data.error || data.response)) || "");
            }
            this.setNetworkMessage(this.t("已保存"), false);
            this.fetchCurrentSettings();
            return data;
          } catch (error) {
            // Keep the entered values so they can be corrected and sent again.
            const detail = String((error && error.message) || "").trim();
            this.setNetworkMessage(this.t("保存失败") + (detail ? " · " + detail : ""), true);
            return null;
          } finally {
            this.isLoading = false;
          }
        },

        async ipPassThroughEnable() {
          if (this.ipPassMode === "未指定") {
            this.setNetworkMessage(this.t("请选择 IP 透传模式"), true);
            return;
          }
          await this.runNetworkAction({ action: 'ip_passthrough', enabled: '1', mode: this.ipPassMode });
        },

        ipPassThroughDisable() {
          this.showError = false;
          this.atCommandResponse = this.t("正在禁用 IP 透传，网口会重启，请等待倒计时结束。");
          this.startRebootCountdown(40);

          SimpleAdmin.Api.settingsData({ action: 'ip_passthrough', enabled: '0' })
            .then((data) => {
              if (data && data.response) {
                this.atCommandResponse = data.response;
              }
              if (!this.handleRebootNotice(data) && data && data.ok === false) {
                this.showError = true;
              }
            })
            .catch((error) => {
              console.info("禁用 IP 透传期间网口/WebSocket 断开，继续保持重启倒计时：", error);
              if (!this.isRebooting) {
                this.startRebootCountdown(40);
              }
            });
        },

        onBoardDNSV6ProxyEnable() {
          return this.runNetworkAction({ action: 'dns_proxy', family: '6', enabled: '1' });
        },
        onBoardDNSV4ProxyEnable() {
          return this.runNetworkAction({ action: 'dns_proxy', family: '4', enabled: '1' });
        },

        onBoardDNSV6ProxyDisable() {
          return this.runNetworkAction({ action: 'dns_proxy', family: '6', enabled: '0' });
        },
        onBoardDNSV4ProxyDisable() {
          return this.runNetworkAction({ action: 'dns_proxy', family: '4', enabled: '0' });
        },


        async usbNetModeChanger() {
          if (!["RMNET", "ECM", "MBIM", "RNDIS"].includes(this.usbNetMode)) {
            this.setNetworkMessage(this.t("请选择 USB 协议"), true);
            return;
          }
          // The new protocol applies after a reboot; only offer it when the module accepted the change.
          if (await this.runNetworkAction({ action: 'usbnet', mode: this.usbNetMode })) {
            this.showRebootModal();
          }
        },

        fetchCurrentSettings() {
          if (!this.isRebooted) {
            return;  // 如果设备还在重启，跳过
          }
          SimpleAdmin.Api.settingsData({ action: 'status' })
            .then((data) => {
              this.ipPassStatus = !!data.ipPassStatus;
              this.DNSV6ProxyStatus = !!data.DNSV6ProxyStatus;
              this.DNSV4ProxyStatus = !!data.DNSV4ProxyStatus;
              this.currentUsbNetMode = data.currentUsbNetMode || '未知';
              const currentDmzIp = String(data.dmzIP || '').trim();
              this.dmzIP = currentDmzIp;
              this.dmzMode = this.resolveDmzMode(data.dmzMode, currentDmzIp);
              this.lanIpStart = data.lanIpStart || '';
              this.lanIpEnd = data.lanIpEnd || '';
              this.lanGwIp = data.lanGwIp || '';
              const oldImei = this.imei;
              const currentImei = (data.imei || '').trim();
              if (/^\d{14,17}$/.test(currentImei)) {
                this.imei = currentImei;
                if (!this.newImei || this.newImei === '-' || this.newImei === oldImei) {
                  this.newImei = currentImei;
                }
              }
            })
            .catch((error) => {
              console.error("错误: ", error);
              this.showError = true;
            });
        },

        fetchTTL() {
          SimpleAdmin.Api.getTTLStatus()
            .then((res) => {
              return res.json();
            })
            .then((data) => {
              this.ttldata = data;
              this.ttlStatus = this.ttldata.isEnabled;
              this.ttlvalue = this.ttldata.ttl;
            })
            .catch((error) => {
              console.error("Error fetching TTL status: ", error); // 日志：捕获错误
            });
        },

        setTTL() {
          const text = String(this.newTTL === null || this.newTTL === undefined ? "" : this.newTTL).trim();
          const ttlval = Number(text);
          this.ttlMessage = "";
          if (!/^\d{1,3}$/.test(text) || ttlval > 255) {
            this.ttlMessage = this.t("请输入 0–255 的 TTL 值");
            return;
          }

          this.isLoading = true;
          SimpleAdmin.Api.setTTL(ttlval)
            .then((res) => res.json().catch(() => ({})).then((data) => {
              if (!res.ok) throw new Error(data.error || "");
              this.ttlMessage = this.t("已保存");
            }))
            .catch((error) => {
              const detail = String((error && error.message) || "").trim();
              this.ttlMessage = this.t("保存失败") + (detail ? " · " + detail : "");
            })
            .finally(() => {
              this.fetchTTL();
              this.isLoading = false;
            });
        },

        async setDMZEnable() {
          const ip = String(this.dmzIP || '').trim();
          if (!/^(\d{1,3}\.){3}\d{1,3}$/.test(ip) || ip.split('.').some((part) => Number(part) > 255)) {
            this.setNetworkMessage(this.t("请输入有效的 IP 地址"), true);
            return;
          }
          if (await this.runNetworkAction({ action: 'dmz', enabled: '1', ip })) {
            this.dmzMode = '1';
          }
        },

        async setDMZDisable() {
          if (await this.runNetworkAction({ action: 'dmz', enabled: '0' })) {
            this.dmzMode = '0';
          }
        },
        setLANIP() {
          this.lanIpSaveSuccess = false;
          if (this.lanIpSaveSuccessTimer) {
            clearTimeout(this.lanIpSaveSuccessTimer);
            this.lanIpSaveSuccessTimer = null;
          }

          // 网关为完整 IPv4，起始/结束只填最后一段，且起始不大于结束。
          const gateway = String(this.lanGwIp || '').trim();
          const gwIpParts = gateway.split('.');
          const octet = (value) => /^\d{1,3}$/.test(String(value).trim()) && Number(value) >= 1 && Number(value) <= 254;
          if (gwIpParts.length !== 4 || !gwIpParts.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255)
              || !octet(this.lanIpStart) || !octet(this.lanIpEnd) || Number(this.lanIpStart) > Number(this.lanIpEnd)) {
            this.setNetworkMessage(this.t("请填写有效的网关地址与 1–254 的起止地址"), true);
            return;
          }

          // 使用网关 IP 地址的前三位与用户输入的最后一位拼接成完整的起始和结束 IP
          const startIp = `${gwIpParts[0]}.${gwIpParts[1]}.${gwIpParts[2]}.${Number(this.lanIpStart)}`;
          const endIp = `${gwIpParts[0]}.${gwIpParts[1]}.${gwIpParts[2]}.${Number(this.lanIpEnd)}`;

          this.isSavingLANIP = true;
          return this.runNetworkAction({ action: 'lanip', start: startIp, end: endIp, gateway })
            .then((data) => {
              if (!data) return;
              this.lanIpSaveSuccess = true;
              this.lanIpSaveSuccessTimer = setTimeout(() => {
                this.lanIpSaveSuccess = false;
                this.lanIpSaveSuccessTimer = null;
              }, 3000);
            })
            .finally(() => {
              this.isSavingLANIP = false;
            });
        },

        // ---------- 在线更新 ----------
        otaError(text) {
          const message = String(text || '');
          const known = [
            ['update signature is not trusted', '更新签名无效或不受信任'],
            ['package checksum does not match', '安装包校验失败'],
            ['download interrupted', '下载中断'],
            ['download failed', '下载失败'],
            ['download is larger than expected', '下载内容大小异常'],
            ['already running', '已有更新任务在进行'],
            ['is already the latest release', '已是最新版本'],
            ['installation failed', '安装失败，请查看安装日志'],
            ['did not finish within', '安装超过 20 分钟仍未完成'],
            ['cannot start the installer', '无法在网页服务之外启动安装程序，请使用 Windows 设备助手安装此版本'],
            ['update source must be', '更新源需为 owner/repo 或 http(s) 地址'],
            ['GitHub proxy must be', 'GitHub 代理需为 http(s) 地址'],
            ['public key must be', '公钥需为 Base64 编码的 32 字节 Ed25519 公钥'],
            ['invalid update manifest', '更新清单无效'],
          ];
          const hit = known.find(([key]) => message.includes(key));
          if (!hit) return message;
          const detail = hit[1] === '下载失败' || hit[1] === '下载中断' ? message.split(': ').slice(1).join(': ') : '';
          return this.t(hit[1]) + (detail ? ' · ' + detail : '');
        },
        otaApply(data) {
          if (!data || !data.current) return;
          this.ota = data;
          if (!this.otaFormLoaded) {
            const settings = data.settings || {};
            this.otaForm = {
              mode: settings.mode || 'check',
              source: settings.source === data.default_source ? '' : (settings.source || ''),
              proxy: settings.proxy || '',
              public_key: settings.public_key || ''
            };
            this.otaFormLoaded = true;
          }
          if (data.phase === 'idle' && data.error) {
            this.otaFailed = true;
            this.otaMessage = this.otaError(data.error);
          }
        },
        async otaRequest(path, body, timeout) {
          const options = body === undefined ? { timeout } : { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), timeout };
          const response = await SimpleAdmin.Api.request(path, options);
          const data = await response.json().catch(() => ({}));
          if (!response.ok || data.ok === false) throw new Error(this.otaError(data.error) || this.t('请求失败'));
          return data;
        },
        async otaRefresh() {
          try {
            this.otaApply(await this.otaRequest('/api/ota'));
          } catch (_) {
            return;
          }
          if (this.ota.phase !== 'idle') { this.otaPoll(); return; }
          const last = this.ota.last_result;
          if (last && !this.otaMessage) {
            this.otaFailed = !last.ok;
            this.otaMessage = this.t(last.ok ? '上次在线更新成功' : '上次在线更新失败') + ' · v' + last.version;
          }
        },
        otaBusy() {
          return Boolean(this.otaRestartFrom) || (this.ota.phase && this.ota.phase !== 'idle');
        },
        otaHasUpdate() {
          return Boolean(this.ota.latest && this.ota.latest.newer);
        },
        otaLatestText() {
          if (this.ota.phase === 'checking') return this.t('检查中...');
          if (!this.ota.latest) return this.t(this.ota.checked_at ? '无法获取' : '尚未检查');
          return this.ota.latest.newer ? 'v' + this.ota.latest.version : this.t('已是最新版本');
        },
        otaPhaseText() {
          return this.t(this.ota.phase === 'downloading' ? '正在下载' : '正在安装');
        },
        otaPercent() {
          return Math.min(100, Math.floor(100 * (this.ota.received || 0) / Math.max(1, this.ota.total || 0))) + '%';
        },
        async otaCheck() {
          if (this.otaBusy()) return;
          this.otaMessage = '';
          this.otaFailed = false;
          this.ota = Object.assign({}, this.ota, { phase: 'checking' });
          try {
            const data = await this.otaRequest('/api/ota/check', {}, 90000);
            this.otaApply(data);
            if (!data.error) {
              this.otaMessage = this.t(data.latest && data.latest.newer ? '发现新版本' : '已是最新版本');
            }
          } catch (error) {
            this.otaFailed = true;
            this.otaMessage = error.message;
            await this.otaRefresh();
          }
        },
        async otaInstall() {
          if (!this.otaHasUpdate() || this.otaBusy()) return;
          if (!window.confirm('将下载并安装新版本，安装期间管理页面会断开约一分钟。继续吗？')) return;
          this.otaMessage = '';
          this.otaFailed = false;
          this.otaRestartFrom = this.ota.current;
          try {
            this.otaApply(await this.otaRequest('/api/ota/install', {}));
            this.otaPoll();
          } catch (error) {
            this.otaRestartFrom = '';
            this.otaFailed = true;
            this.otaMessage = error.message;
          }
        },
        // Plain HTTP polling: the WebSocket goes away while the installer restarts the service.
        otaPoll() {
          clearTimeout(this.otaPollTimer);
          this.otaPollTimer = setTimeout(async () => {
            let data = null;
            try {
              const response = await fetch('/api/ota', { cache: 'no-store', credentials: 'same-origin' });
              if (response.status === 401) { window.location.replace('/login.html'); return; }
              if (response.ok) data = await response.json();
            } catch (_) {
              // The service is restarting.
            }
            if (!data) {
              this.otaFailed = false;
              this.otaMessage = this.t('服务正在重启，请稍候…');
              this.otaPoll();
              return;
            }
            if (this.otaRestartFrom && data.current !== this.otaRestartFrom) {
              this.otaRestartFrom = '';
              this.otaApply(data);
              this.otaFailed = false;
              this.otaMessage = this.t('更新完成，正在刷新页面');
              setTimeout(() => window.location.reload(), 1500);
              return;
            }
            this.otaApply(data);
            if (data.phase !== 'idle') { this.otaPoll(); return; }
            if (data.error) { this.otaRestartFrom = ''; return; }
            if (data.mock) {
              this.otaRestartFrom = '';
              this.otaFailed = false;
              this.otaMessage = this.t('预览模式：安装包已校验并解包，未执行安装');
              return;
            }
            this.otaMessage = this.t('安装完成，等待服务重启');
            this.otaPoll();
          }, 1500);
        },
        async otaSave() {
          if (this.otaSaving) return;
          this.otaSaving = true;
          this.otaSaveMessage = '';
          try {
            const data = await this.otaRequest('/api/ota/settings', Object.assign({}, this.otaForm));
            this.otaFormLoaded = false;
            this.otaApply(data);
            this.otaSaveMessage = this.t('已保存');
          } catch (error) {
            this.otaSaveMessage = this.t('保存失败') + ' · ' + error.message;
          } finally {
            this.otaSaving = false;
          }
        },

        init() {
          if (!this.isRebooted) {
            return;  // 如果设备正在重启，跳过
          }

          this.fetchWebuiSetting();
          this.fetchLanguageSetting();  // 获取界面语言设置
          this.fetchCurrentSettings();  // 发送 AT 命令获取当前设置
          this.fetchTTL();  // 获取 TTL 状态
          this.otaRefresh();
        },
      };
    }

    if (window.SimpleAdminSpaMode) {
      (window.SimpleAdmin.Pages = window.SimpleAdmin.Pages || {}).settings = simpleSettings;
    } else {
      mountSimpleAdminVueApp(simpleSettings);
    }
