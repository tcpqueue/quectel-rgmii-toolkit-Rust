const assert=require('node:assert/strict');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const net=require('node:net');
const {spawn}=require('node:child_process');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
(async()=>{
  assert(!process.env.DEVICE_URL,'Layout tests require an isolated mock server');
  const root=path.resolve(__dirname,'..'),temp=fs.mkdtempSync(path.join(os.tmpdir(),'sms-layout-'));
  const reservation=net.createServer();await new Promise(r=>reservation.listen(0,'127.0.0.1',r));const port=reservation.address().port;await new Promise(r=>reservation.close(r));
  const base='http://127.0.0.1:'+port;
  const child=spawn(path.join(root,'target/debug/simpleadmin-httpd'),['--mock','--http','127.0.0.1:'+port,'--static',path.join(root,'development/simpleadmin/www'),'--auth-file',path.join(temp,'auth'),'--ttl-file',path.join(temp,'ttl')],{stdio:'ignore'});
  const images=path.join(root,'work/sms-layout');fs.mkdirSync(images,{recursive:true});let browser;
  try{
    for(let i=0;i<100;i++){try{await fetch(base+'/login.html');break;}catch{await new Promise(r=>setTimeout(r,100));}}
    browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1440,height:1000}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
    await page.goto(base+'/login.html');await page.locator('#loginLanguage').selectOption('zh-CN');await page.locator('#username').fill('admin');await page.locator('#password').fill('admin');await page.locator('#loginButton').click();await page.waitForURL(base+'/');
    await page.locator('[data-page-link="sms"]').click();await page.waitForFunction(()=>SimpleAdmin.Vue.apps['#smsApp']?.smsSettingsLoaded);
    for(const language of ['zh-CN','en','ru','ar']){
      await page.evaluate(lang=>SimpleAdmin.Lang.setLanguage(lang),language);
      for(const width of [1440,390,320]){
        await page.setViewportSize({width,height:1000});
        for(const [index,view] of ['inbox','compose','forwarding'].entries()){
          await page.locator('.sa-sms-tabs button').nth(index).click();await page.waitForTimeout(150);
          assert.equal(await page.locator('.sa-sms-inbox').isVisible(),view==='inbox');
          assert.equal(await page.locator('.sa-sms-compose').isVisible(),view==='compose');
          assert.equal(await page.locator('#forwardingApp').isVisible(),view==='forwarding');
          assert(!(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1)));
          if(view==='inbox'&&width===320){
            const sender=await page.locator('.sa-sms-summary-sender').first().boundingBox(),date=await page.locator('.sa-sms-summary-date').first().boundingBox();
            if(sender&&date)assert(date.y>=sender.y+sender.height-1,'Sender and date overlap');
          }
          if(width===320)await page.screenshot({path:path.join(images,`${language}-${view}.png`),fullPage:true});
        }
      }
    }
    await page.setViewportSize({width:1440,height:1000});await page.locator('[data-page-link="network"]').click();await page.waitForFunction(()=>SimpleAdmin.Vue.apps['#networkApp']?._initialized);
    await page.evaluate(()=>SimpleAdmin.Vue.apps['#networkApp'].requestCellLock({action:'lock_nr_manual',pci:'0',earfcn:'633984',scs:'30',band:'78',persistence:'persistent',auto_unlock:'1'}));
    const labels={'en':'Persistent cell lock','ru':'Постоянная фиксация соты','ar':'تثبيت خلية دائم','zh-CN':'持久化锁频'};
    for(const [language,label] of Object.entries(labels)){
      await page.evaluate(lang=>SimpleAdmin.Lang.setLanguage(lang),language);
      await page.waitForFunction(label=>document.querySelectorAll('.sa-lock-status')[1].textContent.includes(label),label);
    }
    await page.locator('.sa-lock-policy').scrollIntoViewIfNeeded();await page.screenshot({path:path.join(images,'cell-lock.png'),fullPage:true});
    assert.deepEqual(errors,[]);console.log('PASS SMS tabs, mobile sender/date layout, and immediate lock status language changes: '+images);
  }finally{if(browser)await browser.close();child.kill();await new Promise(r=>child.once('exit',r));}
})().catch(error=>{console.error(error);process.exitCode=1;});
