const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const http = require('node:http');
const {spawn} = require('node:child_process');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

(async()=>{
  assert(!process.env.DEVICE_URL, 'Forwarding mutation tests must only run against an isolated mock server');
  const root=path.resolve(__dirname,'..');const temp=fs.mkdtempSync(path.join(os.tmpdir(),'simpleadmin-forwarding-'));
  const received=[];
  const sink=http.createServer(async(req,res)=>{let body='';for await(const b of req)body+=b;received.push({headers:req.headers,body:JSON.parse(body)});res.writeHead(received.length===2?503:200,{'Content-Type':'application/json'});res.end('{}');});
  await new Promise(r=>sink.listen(18087,'127.0.0.1',r));
  const child=spawn(path.join(root,'target/debug/simpleadmin-httpd'),['--mock','--http','127.0.0.1:18086','--static',path.join(root,'development/simpleadmin/www'),'--auth-file',path.join(temp,'auth'),'--ttl-file',path.join(temp,'ttl')],{stdio:['ignore','pipe','pipe']});
  let logs='';child.stdout.on('data',b=>logs+=b);child.stderr.on('data',b=>logs+=b);
  let browser;
  try {
    for(let i=0;i<100;i++){try{await fetch('http://127.0.0.1:18086/login.html');break;}catch{await new Promise(r=>setTimeout(r,100));}}
    browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1440,height:1000}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
    await page.goto('http://127.0.0.1:18086/login.html');await page.locator('#loginLanguage').selectOption('zh-CN');await page.locator('#username').fill('admin');await page.locator('#password').fill('admin');await page.locator('#loginButton').click();await page.waitForURL('http://127.0.0.1:18086/');
    const api=(path,data)=>page.evaluate(async({path,data})=>{const response=await SimpleAdmin.Api.request(path,data?{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(data)}:{});return {status:response.status,data:await response.json()};},{path,data});
    assert.equal((await api('/api/telemetry')).data.mock,true);
    const showForwarding=async()=>{await page.locator('[data-page-link="sms"]').click();await page.locator('.sa-sms-tabs button').nth(2).click();await page.waitForFunction(()=>SimpleAdmin.Vue.apps['#forwardingApp']?.loaded);};
    await showForwarding();
    assert(!(await page.locator('.sa-sms-inbox').isVisible()));
    assert(!(await page.locator('.sa-sms-compose').isVisible()));
    await page.locator('.sa-sms-tabs button').nth(1).click();assert(await page.locator('.sa-sms-compose').isVisible());assert(!(await page.locator('#forwardingApp').isVisible()));
    await page.locator('.sa-sms-tabs button').nth(0).click();assert(await page.locator('.sa-sms-inbox').isVisible());assert(!(await page.locator('.sa-sms-compose').isVisible()));
    await page.locator('.sa-sms-tabs button').nth(2).click();
    for(const language of ['zh-CN','en','ru','ar']) {
      await page.evaluate(lang=>SimpleAdmin.Lang.setLanguage(lang),language);
      for(const width of [1440,1024,390,320]) {
        await page.setViewportSize({width,height:1000});
        await page.waitForTimeout(350);
        for(const platform of ['serverchan','wecom','dingtalk','feishu','webhook','sim']) {
          await page.locator('#forward-tab-'+platform).click();
          assert(!(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1)),`${language}/${width}/${platform}: overflow`);
          const panel=page.locator('#forward-panel-'+platform);assert(await panel.isVisible());
          assert.equal(await panel.locator('.sa-forward-guide li').count(),3);
        }
        await page.screenshot({path:path.join(temp,`${language}-${width}.png`),fullPage:true});
      }
    }
    await page.evaluate(()=>SimpleAdmin.Lang.setLanguage('zh-CN'));await page.setViewportSize({width:1440,height:1000});
    await page.locator('#forward-tab-webhook').click();
    await page.locator('#forwardDevice').fill('Mock SMS module');
    await page.locator('#forward-url-webhook').fill('http://127.0.0.1:18087/hook');await page.locator('#forward-token-webhook').fill('Bearer local-test');await page.locator('#forward-channel-webhook').check();
    await page.locator('.sa-forward-toolbar button[type="submit"]').click();await page.waitForFunction(()=>!SimpleAdmin.Vue.apps['#forwardingApp'].dirty && !SimpleAdmin.Vue.apps['#forwardingApp'].busy);
    assert.equal((await api('/api/forwarding')).data.channels[4].has_token,true);assert(!(JSON.stringify((await api('/api/forwarding')).data)).includes('local-test'));
    await page.locator('#forward-panel-webhook').getByRole('button',{name:'测试推送',exact:true}).click();await page.waitForFunction(()=>SimpleAdmin.Vue.apps['#forwardingApp'].message==='测试成功');
    assert.equal(received.length,1);assert.equal(received[0].body.sender,'SimpleAdmin');assert.equal(received[0].body.text,'SMS forwarding test');assert.equal(received[0].headers.authorization,'Bearer local-test');
    const raw=(text,index=1)=>`+CMGL: ${index},"REC READ","10086",,"26/09/06,12:00:00+32"\n${text}\nOK`;
    const mock=payload=>page.evaluate(async payload=>{const r=await SimpleAdmin.Api.postForm('/api/mock_at',{action:'set',kind:'sms',payload});return r.json();},payload);
    assert.equal((await mock(raw('Old message'))).ok,true);
    await page.locator('#forwardEnabled').check();await page.locator('.sa-forward-toolbar button[type="submit"]').click();
    for(let i=0;i<80;i++) {if((await api('/api/forwarding')).data.ready)break;await page.waitForTimeout(250);}
    assert.equal((await api('/api/forwarding')).data.ready,true);assert.equal(received.length,1,'must not forward existing inbox');
    assert.equal((await mock(raw('New SMS: 123456',2))).ok,true);
    for(let i=0;i<90 && received.length<3;i++)await page.waitForTimeout(500);
    if(received.length!==3) console.log(JSON.stringify({logs,state:(await api('/api/forwarding')).data,sms:await page.evaluate(async()=>{const r=await SimpleAdmin.Api.postForm('/api/get_sms',{force:'1'});return r.text();})}));
    assert.equal(received.length,3,'first attempt and retry must arrive');assert.equal(received[1].body.text,'New SMS: 123456');assert.deepEqual(received[1].body,received[2].body);assert.equal(received[1].headers['idempotency-key'],received[2].headers['idempotency-key']);
    await page.waitForTimeout(12000);assert.equal(received.length,3,'duplicate polling must not resend');
    const status=(await api('/api/forwarding')).data;assert.equal(status.retry_seconds,180);assert(status.records.some(r=>r.status==='sent'&&r.sender==='10086'));assert(status.records.some(r=>r.status==='retrying'));
    await page.locator('.sa-menu-link[data-page-link="settings"]').click();await page.reload();await showForwarding();assert(await page.locator('#forwardEnabled').isChecked());
    await page.locator('#forwardEnabled').uncheck();await page.locator('.sa-forward-toolbar button[type="submit"]').click();await page.waitForFunction(()=>!SimpleAdmin.Vue.apps['#forwardingApp'].busy&&!SimpleAdmin.Vue.apps['#forwardingApp'].dirty);
    await page.locator('[data-page-link="sms"]').click();await page.waitForFunction(()=>SimpleAdmin.Vue.apps['#smsApp']?.smsSettingsLoaded);
    await page.locator('#smsServiceEnabled').uncheck();await page.waitForFunction(()=>!SimpleAdmin.Vue.apps['#smsApp'].smsSettingsSaving);
    assert.equal((await api('/api/forwarding')).data.sms_enabled,false);
    assert.equal(await page.locator('#phoneNumber').count(),0,'disabled SMS must remove send controls');
    await page.locator('#smsDeleteAfterDay').check();await page.waitForFunction(()=>!SimpleAdmin.Vue.apps['#smsApp'].smsSettingsSaving);
    assert.equal((await api('/api/forwarding')).data.delete_after_day,true);
    await page.reload();await page.waitForFunction(()=>SimpleAdmin.Vue.apps['#smsApp']?.smsSettingsLoaded);assert(!(await page.locator('#smsServiceEnabled').isChecked()));assert(await page.locator('#smsDeleteAfterDay').isChecked());
    for(const language of ['zh-CN','en','ru','ar']) {
      await page.evaluate(lang=>SimpleAdmin.Lang.setLanguage(lang),language);
      await page.setViewportSize({width:320,height:1000});
      await page.waitForTimeout(350);
      assert(!(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1)),`${language}: SMS controls overflow`);
      await page.screenshot({path:path.join(temp,`sms-${language}.png`),fullPage:true});
    }
    await page.evaluate(()=>SimpleAdmin.Lang.setLanguage('zh-CN'));await page.setViewportSize({width:1440,height:1000});
    await page.locator('#smsDeleteAfterDay').uncheck();await page.waitForFunction(()=>!SimpleAdmin.Vue.apps['#smsApp'].smsSettingsSaving);
    await page.locator('#smsServiceEnabled').check();await page.waitForFunction(()=>!SimpleAdmin.Vue.apps['#smsApp'].smsSettingsSaving);
    await showForwarding();
    await page.locator('#forward-tab-sim').click();
    await page.locator('#forward-number-sim').fill('+8613800138000');await page.locator('#forward-channel-sim').check();
    await page.locator('.sa-forward-toolbar button[type="submit"]').click();await page.waitForFunction(()=>!SimpleAdmin.Vue.apps['#forwardingApp'].dirty&&!SimpleAdmin.Vue.apps['#forwardingApp'].busy);
    assert.equal((await api('/api/forwarding')).data.channels[5].number,'+8613800138000');
    assert.equal(await page.locator('#forward-panel-sim').getByRole('button',{name:'测试推送'}).count(),0);
    await page.reload();await showForwarding();await page.locator('#forward-tab-sim').click();assert.equal(await page.locator('#forward-number-sim').inputValue(),'+8613800138000');
    await page.waitForTimeout(350);await page.screenshot({path:path.join(temp,'configured.png'),fullPage:true});assert.deepEqual(errors,[]);
    await page.locator('[data-page-link="network"]').click();await page.waitForFunction(()=>SimpleAdmin.Vue.apps['#networkApp']?._initialized);
    await page.locator('input[name="cell-lock-mode"][value="persistent"]').check();
    await page.locator('#networkModeCell').selectOption({label:'NR5G-SA'});
    await page.locator('#saElementsCell input[aria-label="EARFCN"]').fill('633984');await page.locator('#saElementsCell input[aria-label="PCI"]').fill('0');
    await page.locator('#saElementsCell select').selectOption('30');await page.locator('#saElementsCell input[aria-label="band"]').fill('78');
    await page.getByRole('button',{name:'锁定NR5G-SA小区',exact:true}).click();await page.waitForFunction(()=>SimpleAdmin.Vue.apps['#networkApp'].lockRadios[1]?.persistent===true);
    assert(fs.existsSync(path.join(temp,'cell-lock.json')));
    await page.waitForFunction(()=>!SimpleAdmin.Vue.apps['#networkApp'].showModal);
    for(const language of ['zh-CN','en','ru','ar']){await page.evaluate(lang=>SimpleAdmin.Lang.setLanguage(lang),language);for(const width of [1440,390,320]){await page.setViewportSize({width,height:1000});await page.waitForTimeout(250);assert(!(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1)),`${language}/${width}: locks overflow`);}}
    await page.evaluate(()=>SimpleAdmin.Lang.setLanguage('zh-CN'));await page.setViewportSize({width:1440,height:1000});
    await page.locator('.sa-lock-policy').scrollIntoViewIfNeeded();await page.screenshot({path:path.join(temp,'cell-lock.png'),fullPage:true});
    await page.locator('.sa-lock-status').filter({hasText:'NR5G-SA'}).getByRole('button',{name:'取消锁频',exact:true}).click();await page.waitForFunction(()=>SimpleAdmin.Vue.apps['#networkApp'].lockRadios[1]?.persistent===false);
    assert.equal(JSON.parse(fs.readFileSync(path.join(temp,'cell-lock.json'),'utf8')).rules[1],null);
    assert.deepEqual(errors,[]);
    console.log(JSON.stringify({result:'passed',languages:4,viewports:4,platforms:6,cellLocks:'passed',baseline:'passed',retry:'passed',dedup:'passed',screenshots:temp}));
  } finally {if(browser)await browser.close();child.kill();await new Promise(r=>child.once('exit',r));await new Promise(r=>sink.close(r));}
})().catch(error=>{console.error(error);process.exitCode=1;});
