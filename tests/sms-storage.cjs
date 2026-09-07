const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

(async () => {
  const calls = [];
  const context = {console, SimpleAdminSpaMode: true, SimpleAdmin: {Api: {smsData: async params => {
    calls.push(JSON.parse(JSON.stringify(params)));
    return {ok: true};
  }}}};
  context.window = context;
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(path.join(__dirname, '../development/simpleadmin/www/js/pages/sms.js'), 'utf8'), context);
  const app = context.SimpleAdmin.Pages.sms();
  const me = {storage: 'ME', indices: [1], sender: '10086', text: 'sample', date: 'same'};
  const sm = {...me, storage: 'SM'};
  assert.notEqual(app.messageKey(me), app.messageKey(sm));
  for (const method of ['makeSMSListSignature', 'makeSMSIndexSignature', 'makeSMSMetaSignature']) {
    assert.notEqual(app[method]({messages: [me]}), app[method]({messages: [sm]}));
  }
  app.messages = [me, sm, {...me, indices: [2]}];
  app.messageIndices = [[1], [1], [2]];
  app.selectedMessages = [0, 1];
  app.requestSMS = async () => {};
  await app.deleteSelectedSMS();
  assert.deepEqual(calls, [
    {action: 'delete_indices', storage: 'ME', indices: '1'},
    {action: 'delete_indices', storage: 'SM', indices: '1'}
  ]);
  console.log('PASS storage-aware message identity, change detection and selected deletion');
})().catch(error => { console.error(error); process.exitCode = 1; });
