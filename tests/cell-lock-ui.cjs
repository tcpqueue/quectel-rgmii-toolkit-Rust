const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

const root = path.resolve(__dirname, '..');
const assets = path.join(root, 'development/simpleadmin/www');
const html = fs.readFileSync(path.join(assets, 'index.html'), 'utf8');
const scanMode = html.match(/<select[^>]*id="cellSelect"[\s\S]*?<\/select>/)[0];
const scanTable = html.match(/<table class="table">\s*<thead>[\s\S]*?id="cellScanTableBody"[\s\S]*?<\/table>/)[0];
const lockButton = html.match(/<button[^>]*@click="lockSelectedCells\(\)"[\s\S]*?<\/button>/)[0];
const nr = {type: 'NR5G', provider: '中国联通', band: '78', freq: '627264', pci: '317', rsrp: '-72', scs: 30};
const samePci = {...nr, freq: '633984'};
const shared = {...nr, provider: '中国电信'};
const lte = Array.from({length: 11}, (_, i) => ({
  type: 'LTE', provider: '中国联通', band: '3', freq: String(1650 + i), pci: '0', rsrp: '-85'
}));

(async () => {
  const browser = await chromium.launch({
    headless: true,
    executablePath: process.env.CHROMIUM_EXECUTABLE_PATH || undefined,
    args: process.env.CHROMIUM_ARGS ? JSON.parse(process.env.CHROMIUM_ARGS) : []
  });
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.setContent(`<div id="app">${scanTable}${scanMode}${lockButton}</div>`);
    await page.addScriptTag({path: path.join(assets, 'js/vue.global.prod.js')});
    await page.evaluate(() => {
      window.SimpleAdminSpaMode = true;
      window.alerts = [];
      window.alert = message => alerts.push(message);
      window.requests = [];
      window.rejectLock = false;
      window.SimpleAdmin = {
        Pages: {}, Lang: {getCurrentLanguage: () => 'zh-CN', t: text => text},
        Api: {networkData: async params => {
          requests.push(params);
          if (params.action === 'scan') return window.scanResult;
          if (window.holdLock) await new Promise(resolve => { window.releaseLock = resolve; });
          if (rejectLock) return {ok: false, error: 'cell lock rejected: ERROR'};
          return {ok: true, cell_lock: {radios: []}};
        }}
      };
    });
    await page.addScriptTag({path: path.join(assets, 'js/pages/network.js')});
    await page.evaluate(({nr, samePci, shared, lte}) => {
      const state = SimpleAdmin.Pages.network();
      state.init = async () => { window.refreshes++; };
      state.startModalCountdown = async () => { window.countdowns++; };
      window.refreshes = 0;
      window.countdowns = 0;
      window.scanResult = {nr5g_cells_parsed: [nr, samePci, shared], lte_cells_parsed: lte};
      window.cellApp = Vue.createApp({data: () => state}).mount('#app');
    }, {nr, samePci, shared, lte});
    const rows = page.locator('#cellScanTableBody .table-row');
    const checked = () => page.locator('#cellScanTableBody input:checked').count();
    const select = page.locator('#cellSelect');
    const lock = page.locator('button');
    async function scan(mode) {
      await select.selectOption(mode);
      await page.evaluate(() => cellApp.startCellScan());
      await page.waitForFunction(() => cellApp.resultDoneCell && !cellApp.isLoading);
    }
    async function resetRequests() {
      await page.evaluate(() => { requests.length = 0; alerts.length = 0; refreshes = 0; countdowns = 0; });
    }

    await scan('NR5G Only');
    await resetRequests();
    await rows.nth(0).click();
    assert.equal(await page.evaluate(() => cellApp.selectedCells.length), 1, 'first NR selection must succeed');
    assert.equal(await checked(), 1);
    assert.deepEqual(await page.evaluate(() => alerts), []);
    await rows.nth(0).click();
    assert.equal(await checked(), 0, 'clicking the same NR cell must deselect it');
    assert.equal(await lock.isDisabled(), true);
    await rows.nth(0).click();
    await rows.nth(1).click();
    assert.equal(await checked(), 1, 'same PCI on another frequency must replace the selection');
    assert.equal(await rows.nth(1).locator('input').isChecked(), true);
    await lock.click();
    assert.deepEqual(await page.evaluate(() => requests), [{
      persistence: 'temporary', auto_unlock: '1', action: 'lock_scanned_cells',
      mode: 'NR5G Only', pci: '317', earfcn: '633984', scs: 30, band: '78'
    }]);
    await resetRequests();
    await rows.nth(2).click();
    await page.evaluate(() => { cellApp.lockPersistence = 'persistent'; });
    await lock.click();
    assert.equal(await checked(), 1, 'shared cells on another provider must remain selectable');
    assert.equal(await page.evaluate(() => requests[0].earfcn), '627264');
    assert.equal(await page.evaluate(() => requests[0].persistence), 'persistent');

    await resetRequests();
    await page.evaluate(() => { cellApp.selectedCells.push({...cellApp.selectedCells[0]}); });
    await lock.click();
    assert.equal(await page.evaluate(() => requests.length), 0, 'NR multi-selection must be rejected at submission');
    assert.equal(await page.evaluate(() => alerts.length), 1);
    assert.equal(await page.evaluate(() => countdowns), 0);

    await page.evaluate(() => { scanResult.nr5g_cells_parsed[0] = {...scanResult.nr5g_cells_parsed[0], scs: null}; });
    await scan('NR5G Only');
    await resetRequests();
    await rows.nth(0).click();
    await lock.click();
    assert.equal(await page.evaluate(() => requests.length), 0, 'a scanned NR cell without SCS must not be locked');
    assert.match(await page.evaluate(() => cellApp.lockMessage), /子载波间隔/);
    await page.evaluate(() => { scanResult.nr5g_cells_parsed[0] = {...scanResult.nr5g_cells_parsed[0], scs: 30}; });

    await scan('NR5G Only');
    assert.equal(await checked(), 0, 'a new scan must clear previous selection');
    await rows.nth(0).click();
    await resetRequests();
    await page.evaluate(() => { rejectLock = true; });
    await lock.click();
    await page.waitForFunction(() => !cellApp.lockBusy);
    assert.equal(await page.evaluate(() => cellApp.lockMessage), 'cell lock rejected: ERROR');
    assert.equal(await page.evaluate(() => countdowns), 0, 'failed requests must not show the success countdown');
    assert.equal(await page.evaluate(() => refreshes), 0);
    await page.evaluate(() => { rejectLock = false; });
    await resetRequests();
    await page.evaluate(() => { window.holdLock = true; });
    await lock.click();
    assert.equal(await lock.isDisabled(), true);
    assert.equal(await select.isDisabled(), true);
    await rows.nth(1).click();
    assert.equal(await rows.nth(0).locator('input').isChecked(), true, 'pending requests must keep their selected cell');
    await page.evaluate(() => cellApp.lockSelectedCells());
    assert.equal(await page.evaluate(() => requests.length), 1, 'pending lock requests must not be submitted twice');
    await page.evaluate(() => { window.holdLock = false; releaseLock(); });
    await page.waitForFunction(() => !cellApp.lockBusy && refreshes === 1);
    await resetRequests();
    await page.evaluate(() => { cellApp.nr5g_cells_parsed = []; });
    await lock.click();
    assert.equal(await page.evaluate(() => requests.length), 0, 'missing scan records must not generate a lock request');
    assert.equal(await page.evaluate(() => countdowns), 0);
    assert.equal(await page.evaluate(() => cellApp.lockMessage), '找不到对应的小区，请重新扫描后选择');
    await select.selectOption('LTE Only');
    assert.equal(await lock.isDisabled(), true, 'changing scan mode must clear stale selections');
    assert.equal(await checked(), 0);
    await scan('LTE Only');
    await resetRequests();
    await rows.nth(0).click();
    await rows.nth(1).click();
    assert.equal(await checked(), 2, 'LTE cells with PCI=0 on different frequencies must be independent');
    await rows.nth(0).click();
    assert.equal(await checked(), 1);
    await lock.click();
    assert.equal(await page.evaluate(() => requests[0].earfcn), '1651');
    assert.equal(await page.evaluate(() => requests[0].pci), '0');
    await scan('LTE Only');
    await resetRequests();
    for (let i = 0; i < 10; i++) await rows.nth(i).click();
    await lock.click();
    assert.equal(await page.evaluate(() => requests.length), 1);
    assert.equal(await page.evaluate(() => requests[0].earfcn.split(',').length), 10);
    await resetRequests();
    await rows.nth(10).click();
    await lock.click();
    assert.equal(await page.evaluate(() => requests.length), 0, 'more than 10 LTE cells must not be submitted');
    assert.equal(await page.evaluate(() => alerts.length), 1);
    await page.evaluate(() => cellApp.clearTableRowsBodyCellScan());
    assert.equal(await page.evaluate(() => cellApp.selectedCells.length), 0);
    assert.equal(await lock.isDisabled(), true);
    await scan('Full Scan');
    await rows.nth(0).click();
    assert.equal(await checked(), 0, 'full scan must not show unsupported selection');
    assert.equal(await lock.isDisabled(), true);
    assert.equal(await page.locator('thead').count(), 1, 'repeated scans must not duplicate the table header');
    assert.deepEqual(errors, []);
    console.log('Cell lock UI passed: first selection, replacement, deselection, exact frequency, PCI=0, limits, scan reset, request errors and table state.');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
