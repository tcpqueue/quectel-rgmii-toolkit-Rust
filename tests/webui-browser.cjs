const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const {spawn} = require('node:child_process');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
async function freePort() {
 const server=net.createServer(); await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 const port=server.address().port; await new Promise(resolve=>server.close(resolve)); return port;
}
(async()=>{
 const root=path.resolve(__dirname,'..');
 const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'simpleadmin-webui-'));
 const port=await freePort(); let base='http://127.0.0.1:'+port;
 const child=spawn(path.join(root,'target/debug/simpleadmin-httpd'),['--mock','--http','127.0.0.1:'+port,'--static',path.join(root,'development/simpleadmin/www'),'--auth-file',path.join(temporary,'auth'),'--ttl-file',path.join(temporary,'ttl')],{stdio:'ignore'});
 let browser;
 try {
  for(let i=0;i<100;i++){try{await fetch(base+'/login.html');break;}catch{await new Promise(r=>setTimeout(r,100));}}
  browser=await chromium.launch({headless:true});
  for(const language of ['zh-CN','en','ru','ar']){
   const context=await browser.newContext({viewport:{width:1280,height:1000},locale:language});
   const page=await context.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
   await page.goto(base+'/login.html');await page.locator('#loginLanguage').selectOption(language);
   await page.locator('#username').fill('admin');await page.locator('#password').fill('admin');
   await page.locator('#loginButton').click();await page.waitForURL(base+'/');
   await page.locator('.sa-menu-link[data-page-link="settings"]').click();
   await page.waitForFunction(()=>document.querySelector('#webUsernameInput')?.value==='admin');
   assert.equal(await page.locator('#webuiPortInput').inputValue(),String(new URL(base).port));
   await page.locator('#webuiPortHeading').scrollIntoViewIfNeeded();
   await page.waitForTimeout(1500);
   if(language==='zh-CN') await page.screenshot({path:path.join(root,'docs/images/webui-settings.png')});
   await page.setViewportSize({width:390,height:1000});
   assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1),false);
   if(language==='zh-CN'){
    await page.locator('#webuiCurrentPassword').fill('admin');
    const next=await freePort(); await page.locator('#webuiPortInput').fill(String(next));
    const response=page.waitForResponse(r=>r.url().endsWith('/api/set_webui_port'));
    await page.locator('form').filter({has:page.locator('#webuiPortInput')}).locator('button[type="submit"]').click();
    assert.equal((await response).status(),200);
    await page.waitForFunction(()=>document.querySelector('#settingsApp')?.textContent.includes('通过 ADB 转发访问时'));
    base='http://127.0.0.1:'+next;
    await page.setViewportSize({width:1280,height:1000});
    await page.goto(base+'/'); await page.locator('.sa-menu-link[data-page-link="settings"]').click();
    await page.waitForFunction(()=>document.querySelector('#webUsernameInput')?.value==='admin');
    await page.locator('#webUsernameInput').fill('owner');
    await page.locator('#currentPasswordInput').fill('admin');
    await page.locator('form').filter({has:page.locator('#webUsernameInput')}).locator('button[type="submit"]').click();
    await page.waitForURL(base+'/login.html');
    await page.locator('#username').fill('owner');await page.locator('#password').fill('admin');await page.locator('#loginButton').click();await page.waitForURL(base+'/');
    await page.locator('.sa-menu-link[data-page-link="settings"]').click();
    await page.waitForFunction(()=>document.querySelector('#webUsernameInput')?.value==='owner');
    await page.locator('#webUsernameInput').fill('admin');await page.locator('#currentPasswordInput').fill('admin');
    await page.locator('form').filter({has:page.locator('#webUsernameInput')}).locator('button[type="submit"]').click();
    await page.waitForURL(base+'/login.html');
   }
   assert.deepEqual(errors,[]);await context.close();console.log('PASS WebUI settings browser '+language);
  }
 }finally{if(browser)await browser.close();child.kill();await new Promise(resolve=>child.once('exit',resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});