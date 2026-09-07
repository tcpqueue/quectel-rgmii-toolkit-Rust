function cellLocking() {
      return {
        // ---------- 状态 ----------
        isLoading: false,
        showModal: false,
        countdown: 0,
        networkModeCell: "-",
        // 兼容现有绑定（10 组 EARFCN/PCI）
        earfcn1: null, pci1: null,
        earfcn2: null, pci2: null,
        earfcn3: null, pci3: null,
        earfcn4: null, pci4: null,
        earfcn5: null, pci5: null,
        earfcn6: null, pci6: null,
        earfcn7: null, pci7: null,
        earfcn8: null, pci8: null,
        earfcn9: null, pci9: null,
        earfcn10: null, pci10: null,
        scs: null,
        band: null,
        apn: "-",
        newApn: null,
        prefNetwork: "-",
        prefNetworkMode: null,
        nrModeControl: "-",
        cellNum: null,
        lte_bands: null,
        nsa_bands: null,
        sa_bands: null,
        locked_lte_bands: null,
        locked_nsa_bands: null,
        locked_sa_bands: null,
        currentNetworkMode: "-",
        updatedLockedBands: null,
        updatedLockedBandsByMode: { LTE: [], NSA: [], SA: [] },
        cellLockStatus: "未知",
        lockPersistence: 'temporary', lockAutoUnlock: true, lockBusy: false, lockMessage: '', lockRadios: [],
        lockTimer: null, lockEventsBound: false, lockRefreshBusy: false,
        lockStatusText(state) {
          return SimpleAdmin.Lang.t(({temporary:'临时锁频',persistent:'持久化锁频',connected:'已拨号，锁频保持',unlocked:'未锁定',fallback:'已超时回退，开机恢复已关闭',fallback_error:'回退失败，正在重试'})[state.phase] || (state.persistent?'等待开机恢复':'未锁定'));
        },
        async refreshCellLocks() {
          if(this.lockRefreshBusy) return;
          this.lockRefreshBusy=true;
          try {const data=await SimpleAdmin.Api.networkData({action:'cell_lock_status'});this.lockRadios=data.radios||[];} finally {this.lockRefreshBusy=false;}
        },
        async requestCellLock(params) {
          this.lockBusy=true;this.lockMessage='';
          try {
            const data=await SimpleAdmin.Api.networkData({persistence:this.lockPersistence,auto_unlock:this.lockAutoUnlock?'1':'0',...params});
            if(data.ok===false) throw new Error(data.error||data.response||'锁频失败');
            this.lockRadios=data.cell_lock.radios;this.lockMessage=data.warning||SimpleAdmin.Lang.t('已保存');
            return data;
          } catch(error) {this.lockMessage=error.message;throw error;} finally {this.lockBusy=false;}
        },
        async restoreSavedLock(state) {
          const v=state.values;if(!v)return;
          const params=state.radio==='lte'?{action:'lock_lte_manual',cellNum:v[0],pairs:Array.from({length:v[0]},(_,i)=>v.slice(1+i*2,3+i*2).join(',')).join(';')}:{action:'lock_nr_manual',pci:v[0],earfcn:v[1],scs:v[2],band:v[3]};
          try {await this.requestCellLock({...params,persistence:'persistent',auto_unlock:state.auto_unlock?'1':'0'});}catch(_){}
        },
        async cancelSavedLock(state) {try {await this.requestCellLock({action:state.radio==='lte'?'unlock_lte':'unlock_nr'});}catch(_){}},
        bands: "获取频段中...",
        isGettingBands: false,
        pdpType: "-",
        newPdpType: null,
        nrModeControlCurrent: null,
        nrModeControlNew: null,
        _bandsFetchInFlight: {},
        _bandsRetryTimers: {},
        _bandsRetryCount: {},
        _initialized: false,
        _currentSettingsFetchInFlight: null,
        model: "-",
        _modelFetchInFlight: null,
        selectedCells: [],
        nr5g_cells: [],
        lte_cells: [],
        nr5g_cells_parsed: [],
        lte_cells_parsed: [],
        atcmd: "",
        tableRows: [],
        nr5g_neighbourCells: [],
        lte_neighbourCells: [],
        nr5g_neighbourCellsParsed: [],
        lte_neighbourCellsParsed: [],
        neighbourCellsTableRows: [],
        cellScanMode: "Unspecified",
        neighbourCellsScanMode: "Unspecified",
        isLoading: false,
        isCellScanning: false,
        isneighbourScanning: false,
        resultDoneCell: false,
        resultDoneNeighbourCell: false,

        startCellScan() {
          this.clearCellScanData();

          this.isLoading = true;
          this.isCellScanning = true;

          SimpleAdmin.Api.networkData({ action: 'scan', mode: this.cellScanMode })
            .then(data => {
              this.nr5g_cells_parsed = data.nr5g_cells_parsed || [];
              this.lte_cells_parsed = data.lte_cells_parsed || [];
              this.generateTableRow();
            })
            .then(() => {
              this.isLoading = false;
              this.isCellScanning = false;
              this.resultDoneCell = true;
            })
            .catch(error => {
              console.error("Error processing scan data:", error);
              this.isLoading = false;
              this.isCellScanning = false;
            });

        },
        generateTableRow() {
          const tableBody = document.getElementById("cellScanTableBody");
          //tableBody.innerHTML = "";
          if (tableBody.rows.length === 0) {
            // 只在表格为空时插入表头
            const tableHeader = `
              <thead>
                <tr>
                  <th scope="col">选择</th>
                  <th scope="col">网络</th>
                  <th scope="col">运营商</th>
                  <th scope="col">频段</th>
                  <th scope="col">频点</th>
                  <th scope="col">PCI</th>
                  <th scope="col">RSRP</th>
                  <th scope="col">信号</th>
                </tr>
              </thead>
            `;
            // 插入表头，只在表格为空时插入
            tableBody.insertAdjacentHTML("beforebegin", tableHeader);
          }
          this.tableRows = [];

          const cells = this.cellScanMode === "Full Scan"
            ? [...this.nr5g_cells_parsed, ...this.lte_cells_parsed]
            : this.cellScanMode === "NR5G Only"
              ? this.nr5g_cells_parsed
              : this.lte_cells_parsed;


          cells.forEach(cell => {
            const signalSvg = this.signalIconSVG(cell.rsrp);
            const isChecked = this.selectedCells.some(selectedCell =>
              selectedCell.pci === cell.pci && selectedCell.provider === cell.provider
            );

            this.tableRows.push(`
              <tr class="table-row" data-pci="${cell.pci}" data-provider="${cell.provider}">
                <th scope="row">
                  <!-- 复选框不可点击，仅用于显示状态 -->
                  <input type="checkbox" class="checkbox-cell" data-pci="${cell.pci}" data-provider="${cell.provider}"
                        ${isChecked ? 'checked' : ''} disabled /> <!-- 使用disabled属性使其不可点击 -->
                </th>
                <td>${cell.type}</td>
                <td>${cell.provider}</td>
                <td>${cell.band}</td>
                <td>${cell.freq}</td>
                <td>${cell.pci}</td>
                <td>${cell.rsrp}</td>
                <td>${signalSvg}</td>
              </tr>
            `);
          });

          tableBody.innerHTML = this.tableRows.join('');

          const rows = tableBody.querySelectorAll('.table-row');
          rows.forEach(row => {
            row.addEventListener('click', (e) => {
              const checkbox = row.querySelector('.checkbox-cell');

              if (e.target !== checkbox) {
                checkbox.checked = !checkbox.checked;
              }

              this.toggleCellSelection(checkbox.dataset.pci, checkbox.dataset.provider);
            });
          });
        },
        toggleCellSelection(pci, provider) {
          if (this.cellScanMode === "NR5G Only") {
            if(this.selectedCells.length!==1){alert('NR5G-SA 每次只能锁定一个小区');return;}
            // 如果是 NR5G Only 模式，只能选择一个小区
            const index = this.selectedCells.findIndex(cell => cell.pci === pci && cell.provider === provider);

            if (index === -1) {
              // 如果没有选择当前小区，则清空之前的选择，只保留当前选择
              this.selectedCells = [{ pci, provider }];
            } else {
              // 如果当前小区已经选择，取消选择
              this.selectedCells = [];
            }
          } else if (this.cellScanMode === "LTE Only") {
            // 如果是 LTE Only 模式，允许选择多个小区
            const index = this.selectedCells.findIndex(cell => cell.pci === pci && cell.provider === provider);

            if (index === -1) {
              // 如果没有选择当前小区，则添加到 selectedCells 数组
              this.selectedCells.push({ pci, provider });
            } else {
              // 如果已经选择当前小区，则从 selectedCells 数组中删除
              this.selectedCells.splice(index, 1);
            }
          }

          // 更新复选框的选择状态
          this.updateTableRowSelection();
        },
        updateTableRowSelection() {
          const checkboxes = document.querySelectorAll("input.checkbox-cell");
          checkboxes.forEach(checkbox => {
            const pci = checkbox.getAttribute("data-pci");
            const provider = checkbox.getAttribute("data-provider");

            const isChecked = this.selectedCells.some(cell => cell.pci === pci && cell.provider === provider);
            checkbox.checked = isChecked;
          });
        },
        signalIconSVG(rsrp) {
          const levels = [
            { min: -55, svg: `<line x1="2" y1="20" x2="2" y2="20" /><line x1="7" y1="20" x2="7" y2="16" /><line x1="12" y1="20" x2="12" y2="12" /><line x1="17" y1="20" x2="17" y2="8" /><line x1="22" y1="20" x2="22" y2="4" />` },
            { min: -85, svg: `<line x1="2" y1="20" x2="2" y2="20" /><line x1="7" y1="20" x2="7" y2="16" /><line x1="12" y1="20" x2="12" y2="12" /><line x1="17" y1="20" x2="17" y2="8" />` },
            { min: -95, svg: `<line x1="2" y1="20" x2="2" y2="20" /><line x1="7" y1="20" x2="7" y2="16" /><line x1="12" y1="20" x2="12" y2="12" />` },
            { min: -Infinity, svg: `<line x1="2" y1="20" x2="2" y2="20" /><line x1="7" y1="20" x2="7" y2="16" />` }
          ];
          const level = levels.find(l => rsrp >= l.min);
          return level ? `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${level.svg}</svg>` : '';
        },
        clearCellScanData() {
          this.nr5g_cells = [];
          this.lte_cells = [];
          this.nr5g_cells_parsed = [];
          this.lte_cells_parsed = [];
          this.tableRows = [];
        },
        clearTableRowsBodyCellScan() {
          this.clearCellScanData();
          this.resultDoneCell = false;
          const tableBody = document.getElementById("cellScanTableBody");
          tableBody.innerHTML = `
            <tr>
              <th scope="row">空</th>
              <td>空</td>
              <td>空</td>
              <td>空</td>
              <td>空</td>
              <td>空</td>
              <td>空</td>
              <td>空</td>
            </tr>
          `;
        },
        async lockSelectedCells() {
          if (this.selectedCells.length === 0) {
            alert("请至少选择一个小区进行锁定.");
            return;
          }

          if (this.cellScanMode === "NR5G Only") {
            // 仅 NR5G 模式
            for (const { pci, provider } of this.selectedCells) {
              const { earfcn1: earfcn, pci1: cellPci, scs, band } = this.getCellDetails(pci, provider,"NR5G");

              await this.requestCellLock({ action: 'lock_scanned_cells', mode: this.cellScanMode, pci: cellPci, earfcn, scs, band })
                .catch(error => { console.error('发送锁定命令失败:', error); });
            }
          } else if (this.cellScanMode === "LTE Only") {
            // 仅 LTE 模式
            if (this.selectedCells.length > 10) {
              alert("最多只能选择 10 条小区进行锁定");
              return; // 如果选择超过 10 条小区，提示并退出
            }
            const pairs = [];
            const pcias = [];

            // 使用 getCellDetails 来获取 LTE 小区的详细信息
            for (const { pci, provider } of this.selectedCells) {
              const { earfcn1: earfcn, pci1: cellPci } = this.getCellDetails(pci, provider,"LTE");

              pairs.push(earfcn);  // 收集频段（earfcn）
              pcias.push(cellPci); // 收集 PCI
            }

            // 确保频段（earfcn）和 PCI 交替排列
            const orderedPairs = [];
            for (let i = 0; i < pairs.length; i++) {
              orderedPairs.push(pairs[i]);
              orderedPairs.push(pcias[i]);
            }

            const cellNum = pairs.length;  // 设置正确的小区数
            // 提交小区锁定参数，由后端生成锁定指令
            await this.requestCellLock({
              action: 'lock_scanned_cells',
              mode: this.cellScanMode,
              earfcn: pairs.join(','),
              pci: pcias.join(',')
            })
              .catch(error => { console.error('发送锁定命令失败:', error); });
          }

          await this.startModalCountdown(3);
          await this.init();
        },

        getCellDetails(pci, provider, type) {
          // 根据传递的 type 参数决定从哪个数据源查找
          let cell;
          if (type === "NR5G") {
            // 如果 type 为 NR5G，则从 nr5g_cells_parsed 查找
            cell = this.nr5g_cells_parsed.find(c => c.pci === pci && c.provider === provider);
          } else if (type === "LTE") {
            // 如果 type 为 LTE，则从 lte_cells_parsed 查找
            cell = this.lte_cells_parsed.find(c => c.pci === pci && c.provider === provider);
          }

          // 如果找不到对应的小区数据，输出错误并返回默认值
          if (!cell) {
            console.error(`找不到对应的 cell 数据: PCI=${pci}, Provider=${provider}, Type=${type}`);
            return { earfcn1: undefined, pci1: undefined, scs: undefined, band: undefined };
          }

          // 返回找到的小区的详细信息
          return {
            earfcn1: cell.freq,
            pci1: cell.pci,
            scs: 30, // 默认子载波间隔
            band: cell.band
          };
        },

        // ---------- 工具函数 ----------
        sleep(ms) { return new Promise(r => setTimeout(r, ms)); },
        async startModalCountdown(seconds = 1) {
          this.showModal = true;
          this.countdown = seconds;
          while (this.countdown > 0) {
            await this.sleep(1000);
            this.countdown--;
          }
          this.showModal = false;
        },

        // ---------- 业务逻辑 ----------

        applyModelBands() {
          const bm = band_map.getBandsForModel(this.model, band_map.DEFAULTS);
          this.lte_bands = bm.lte;
          this.nsa_bands = bm.nsa;
          this.sa_bands = bm.sa;
        },

        renderBandCheckboxes() {
          this.applyModelBands();
          populateCheckboxes(
            this.lte_bands,
            this.nsa_bands,
            this.sa_bands,
            this.locked_lte_bands || '',
            this.locked_nsa_bands || '',
            this.locked_sa_bands || '',
            this
          );
          this.trackCheckboxChanges();
          updateButtonText();
        },

        bandModes() {
          return ['LTE', 'NSA', 'SA'];
        },

        lockedBandsLoaded(mode) {
          if (mode === 'LTE') return this.locked_lte_bands !== null;
          if (mode === 'NSA') return this.locked_nsa_bands !== null;
          if (mode === 'SA') return this.locked_sa_bands !== null;
          return this.bandModes().every(m => this.lockedBandsLoaded(m));
        },

        setLockedBandsForMode(mode, data) {
          if (mode === 'LTE') {
            this.locked_lte_bands = data.locked_lte_bands || '';
          } else if (mode === 'NSA') {
            this.locked_nsa_bands = data.locked_nsa_bands || '';
          } else if (mode === 'SA') {
            this.locked_sa_bands = data.locked_sa_bands || '';
          }
        },

        setAllLockedBands(data) {
          this.locked_lte_bands = data.locked_lte_bands || '';
          this.locked_nsa_bands = data.locked_nsa_bands || '';
          this.locked_sa_bands = data.locked_sa_bands || '';
        },

        scheduleBandsRetry(mode) {
          const retryMode = mode || 'ALL';
          const count = this._bandsRetryCount[retryMode] || 0;
          if (count >= 3) {
            return;
          }
          this._bandsRetryCount[retryMode] = count + 1;
          if (this._bandsRetryTimers[retryMode]) {
            clearTimeout(this._bandsRetryTimers[retryMode]);
          }
          this._bandsRetryTimers[retryMode] = setTimeout(() => {
            delete this._bandsRetryTimers[retryMode];
            if (!this.lockedBandsLoaded(retryMode)) {
              this.getSupportedBands(true, retryMode);
            }
          }, 1200);
        },

        async getSupportedBands(force = false, mode = 'ALL') {
          const selectedMode = String(mode || 'ALL').toUpperCase();

          // 已经拿过锁定频段时只重绘，不重复请求 AT。
          if (!force && this.lockedBandsLoaded(selectedMode)) {
            this.renderBandCheckboxes();
            return;
          }

          // 同一模式已有请求时，直接复用；单模式和全部模式互不影响。
          if (this._bandsFetchInFlight[selectedMode]) return this._bandsFetchInFlight[selectedMode];

          this.isGettingBands = true;

          const waitForResult = force ? '1' : '0';
          const payload = { action: 'bands', wait: waitForResult, force: force ? '1' : '0' };
          if (selectedMode !== 'ALL') payload.mode = selectedMode;

          this._bandsFetchInFlight[selectedMode] = SimpleAdmin.Api.networkData(payload)
            .then(data => {
              if (data.pending || data.error) {
                this.renderBandCheckboxes();
                this.scheduleBandsRetry(selectedMode);
                return;
              }
              if (selectedMode === 'ALL') {
                this.setAllLockedBands(data);
              } else {
                this.setLockedBandsForMode(selectedMode, data);
              }
              this._bandsRetryCount[selectedMode] = 0;
              this.renderBandCheckboxes();
            })
            .finally(() => {
              delete this._bandsFetchInFlight[selectedMode]; // 清空当前模式请求标记
              this.isGettingBands = Object.keys(this._bandsFetchInFlight).length > 0;
            });

          return this._bandsFetchInFlight[selectedMode];
        },
        async init() {
          if(!this.lockEventsBound){
            this.lockEventsBound=true;
            const sync=()=>{const active=document.querySelector('[data-page="network"]').classList.contains('active');if(active){this.refreshCellLocks().catch(()=>{});if(!this.lockTimer)this.lockTimer=setInterval(()=>this.refreshCellLocks().catch(()=>{}),5000);}else{clearInterval(this.lockTimer);this.lockTimer=null;}};
            window.addEventListener('simpleadmin:page-changed',sync);sync();
          }
          // 初始化时，禁止用户操作（还没有准备好）
          this._initialized = false;

          await this.getModel();          // 先确定型号
          this.applyModelBands();        // 型号可用后立刻得到当前可用频段
          this.renderBandCheckboxes();   // 先显示 LTE / NR5G-NSA / NR5G-SA 三列
          const lockedBandsPromise = this.getSupportedBands(true, 'ALL'); // 随即强制发送 AT 查询当前已锁定频段
          await Promise.all([
            this.getCurrentSettings(),          // 获取当前网络状态，避免被首次频段查询影响
            lockedBandsPromise
          ]);
          //this.clearTableRowsBodyCellScan();

          // 初始化完成后，允许处理用户操作
          this._initialized = true;
        },

        getBandCheckboxes(mode) {
          return document.querySelectorAll(`input.checkbox-band[data-band-mode="${mode}"]`);
        },

        trackCheckboxChanges(mode = null) {
          const modes = mode ? [mode] : this.bandModes();
          modes.forEach(m => {
            const newCheckedValues = [];
            this.getBandCheckboxes(m).forEach(cb => { if (cb.checked) newCheckedValues.push(cb.value); });
            this.updatedLockedBandsByMode[m] = newCheckedValues;
          });
          if (mode) {
            this.currentNetworkMode = mode;
            this.updatedLockedBands = this.updatedLockedBandsByMode[mode];
          }
        },

        toggleBandCheckboxes(mode) {
          const selectedMode = String(mode || this.currentNetworkMode || 'LTE').toUpperCase();
          const checkboxes = this.getBandCheckboxes(selectedMode);
          if (checkboxes.length === 0) return;

          let allChecked = true;
          checkboxes.forEach(cb => { if (!cb.checked) allChecked = false; });
          const targetChecked = !allChecked;

          checkboxes.forEach((checkbox) => {
            checkbox.checked = targetChecked;
          });
          this.trackCheckboxChanges(selectedMode);
          updateButtonText(selectedMode);
        },

        async getCurrentSettings() {
          // 如果请求正在进行中，直接返回
          if (this._currentSettingsFetchInFlight) return this._currentSettingsFetchInFlight;

          this._currentSettingsFetchInFlight = SimpleAdmin.Api.networkData({ action: 'settings' })
            .then(settings => {
              this.apn = settings.apn || '-';
              this.cellLockStatus = settings.cellLockStatus || '未知';
              this.lockRadios = settings.cell_lock?.radios || [];
              this.prefNetwork = settings.prefNetwork || '-';
              this.bands = settings.bands || '-';
              this.nrModeControlCurrent = settings.nrModeControlNum || null;
              this.nrModeControlNew = null;
              this.pdpType = settings.pdpType || '-';
            })
            .finally(() => {
              this._currentSettingsFetchInFlight = null; // 清空请求标记
            });
          return this._currentSettingsFetchInFlight;
        },

        invalidateLockedBands(mode) {
          if (mode === 'LTE') {
            this.locked_lte_bands = null;
          } else if (mode === 'NSA') {
            this.locked_nsa_bands = null;
          } else if (mode === 'SA') {
            this.locked_sa_bands = null;
          }
        },

        async lockSelectedBandsForMode(mode) {
          const selectedMode = String(mode || '').toUpperCase();
          if (!this.bandModes().includes(selectedMode)) {
            alert("频段模式无效");
            return;
          }
          this.trackCheckboxChanges(selectedMode);
          const newCheckedValues = this.updatedLockedBandsByMode[selectedMode] || [];
          if (newCheckedValues.length === 0) {
            alert("没有选中任何频段，请选择至少一个频段！");
            return;
          }
          await SimpleAdmin.Api.networkData({ action: 'lock_bands', mode: selectedMode, values: newCheckedValues.join(':') });
          this.invalidateLockedBands(selectedMode);
          await this.startModalCountdown(3);
          await Promise.all([
            this.getSupportedBands(true, selectedMode),
            this.getCurrentSettings()
          ]);
        },

        async lockSelectedBands() {
          return this.lockSelectedBandsForMode(this.currentNetworkMode || 'LTE');
        },

        async resetBandLocking() {
          await SimpleAdmin.Api.networkData({ action: 'reset_bands', lte: this.lte_bands, nsa: this.nsa_bands, sa: this.sa_bands });
          this.locked_lte_bands = null;
          this.locked_nsa_bands = null;
          this.locked_sa_bands = null;
          await this.startModalCountdown(3);
          await Promise.all([
            this.getSupportedBands(true, 'ALL'),
            this.getCurrentSettings()
          ]);
        },

        async saveChanges() {
          const newApn = this.newApn;
          const prefNetworkMode = this.prefNetworkMode;

          const payload = { action: 'save_settings' };
          if (this.newPdpType !== null || newApn !== null) {
            payload.pdpType = this.newPdpType || this.pdpType || 'IPV4V6';
            payload.apn = (newApn !== null && newApn !== '') ? newApn : (this.apn !== '-' ? this.apn : '');
          }
          if (this.nrModeControlNew !== null && this.nrModeControlNew !== this.nrModeControlCurrent) {
            payload.nrDisableMode = this.nrModeControlNew;
          }
          if (prefNetworkMode !== null) {
            payload.modePref = prefNetworkMode;
          }
          if (!payload.apn && !payload.modePref && payload.nrDisableMode === undefined) {
            alert("没有做出更改");
            return;
          }

          await SimpleAdmin.Api.networkData(payload);
          await this.startModalCountdown(3);
          //await this.getCurrentSettings();
          await this.init();
        },

        async getModel() {
          if (this.model && this.model !== "-") return this.model;
          if (this._modelFetchInFlight) return this._modelFetchInFlight;

          this._modelFetchInFlight = (async () => {
            let lastData = null;
            for (let attempt = 0; attempt < 4; attempt += 1) {
              const data = await SimpleAdmin.Api.networkData({ action: 'model', force: '1' });
              lastData = data;
              if (data.model && data.model !== '-') {
                this.model = data.model;
                break;
              }
              if (!data.pending && attempt >= 1) break;
              await this.sleep(700);
            }
            if (!this.model || this.model === '-') {
              console.warn("[CGMM] 未解析到有效型号，保留现有 model：", this.model, lastData || {});
            }
            this._modelFetchInFlight = null;
            return this.model;
          })();

          return this._modelFetchInFlight;
        },

        getLteCellCount() {
          if (this.cellNum === null || this.cellNum === undefined || this.cellNum === '') return 0;
          const count = Math.floor(Number(this.cellNum));
          if (!Number.isFinite(count) || count < 1) return 0;
          return Math.min(count, 10);
        },

        hasLteManualCellCount() {
          return this.networkModeCell === "LTE" && this.getLteCellCount() > 0;
        },

        visibleLteCellIndexes() {
          const count = this.getLteCellCount();
          const indexes = [];
          for (let i = 1; i <= count; i += 1) indexes.push(i);
          return indexes;
        },

        getLteManualValue(index, field) {
          const suffix = field === 'pci' ? 'pci' : 'earfcn';
          return this[`${suffix}${index}`] || '';
        },

        setLteManualValue(index, field, value) {
          const suffix = field === 'pci' ? 'pci' : 'earfcn';
          this[`${suffix}${index}`] = value;
        },

        clearLteManualInputsAfter(count) {
          for (let i = count + 1; i <= 10; i += 1) {
            this[`earfcn${i}`] = null;
            this[`pci${i}`] = null;
          }
        },

        resetLteManualInputs() {
          this.cellNum = null;
          this.clearLteManualInputsAfter(0);
        },

        normalizeLteCellCount() {
          if (this.cellNum === null || this.cellNum === undefined || this.cellNum === '') {
            this.cellNum = null;
            this.clearLteManualInputsAfter(0);
            return;
          }

          const count = this.getLteCellCount();
          if (count < 1) {
            this.cellNum = null;
            this.clearLteManualInputsAfter(0);
            return;
          }

          this.cellNum = count;
          this.clearLteManualInputsAfter(count);
        },

        onNetworkModeCellChange(event) {
          const selectedMode = event && event.target ? event.target.value : this.networkModeCell;
          this.networkModeCell = selectedMode;
          if (selectedMode === "LTE") {
            this.resetLteManualInputs();
          }
        },

        getVisibleLteManualPairs(count) {
          const pairs = [];
          for (let i = 1; i <= count; i += 1) {
            const earfcn = String(this[`earfcn${i}`] || '').trim();
            const pci = String(this[`pci${i}`] || '').trim();
            if (!earfcn || !pci) return null;
            pairs.push({ earfcn, pci });
          }
          return pairs;
        },

        async cellLockEnableLTE() {
          const cellNum = this.getLteCellCount();
          if (cellNum < 1) {
            alert("请输入要锁定的小区数量");
            return;
          }

          const pairs = this.getVisibleLteManualPairs(cellNum);
          if (!pairs) {
            alert(`请完整填写 ${cellNum} 组 EARFCN 和 PCI`);
            return;
          }

          await this.requestCellLock({
            action: 'lock_lte_manual',
            cellNum,
            pairs: pairs.map(p => `${p.earfcn},${p.pci}`).join(';')
          });
          await this.startModalCountdown(3);
          //await this.getCurrentSettings();
          await this.init();
        },

        async cellLockEnableNR() {
          const { earfcn1: earfcn, pci1: pci, scs, band } = this;
          if (earfcn === null || pci === null || scs === null || band === null) {
            alert("请输入所有必填字段");
            return;
          }
          await this.requestCellLock({ action: 'lock_nr_manual', pci, earfcn, scs, band });
          await this.startModalCountdown(3);
          //await this.getCurrentSettings();
          await this.init();
        },

        async cellLockDisableLTE() {
          await this.requestCellLock({ action: 'unlock_lte' });
          await this.startModalCountdown(3);
          //await this.getCurrentSettings();
          await this.init();
        },

        async cellLockDisableNR() {
          await this.requestCellLock({ action: 'unlock_nr' });
          await this.startModalCountdown(3);
          //await this.getCurrentSettings();
          await this.init();
        },
      };
    }

    function addCheckboxListeners(cellLock) {
      document.querySelectorAll('input.checkbox-band').forEach(function (checkbox) {
        checkbox.onchange = function () {
          const mode = checkbox.dataset.bandMode;
          cellLock.trackCheckboxChanges(mode);
          updateButtonText(mode);
        };
      });
    }

    // 更新每个模式的全选 / 取消选中按钮文本
    function updateButtonText(mode = null) {
      const modes = mode ? [mode] : ['LTE', 'NSA', 'SA'];
      modes.forEach((currentMode) => {
        const checkboxes = document.querySelectorAll(`input.checkbox-band[data-band-mode="${currentMode}"]`);
        const button = document.getElementById(`toggleBands${currentMode}`);
        if (!button) return;
        if (checkboxes.length === 0) {
          button.textContent = "全部选中";
          return;
        }
        let allChecked = true;
        checkboxes.forEach(cb => { if (!cb.checked) allChecked = false; });
        button.textContent = allChecked ? "取消选中" : "全部选中";
      });
    }

if (window.SimpleAdminSpaMode) {
  (window.SimpleAdmin.Pages = window.SimpleAdmin.Pages || {}).network = cellLocking;
} else {
  mountSimpleAdminVueApp(cellLocking);
}
