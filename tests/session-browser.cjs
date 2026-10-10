// Login, session expiry and settings feedback in the module Web UI (mock backend).
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
 const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'simpleadmin-session-'));
 const port=await freePort(); const base='http://127.0.0.1:'+port;
 const start=()=>spawn(path.join(root,'target/debug/simpleadmin-httpd'),['--mock','--http','127.0.0.1:'+port,'--static',path.join(root,'development/simpleadmin/www'),'--auth-file',path.join(temporary,'auth'),'--ttl-file',path.join(temporary,'ttl')],{stdio:'ignore'});
 const ready=async()=>{for(let i=0;i<100;i++){try{await fetch(base+'/login.html');return;}catch{await new Promise(r=>setTimeout(r,100));}}throw new Error('server did not start');};
 const stop=async(server)=>{server.kill();await new Promise(resolve=>server.once('exit',resolve));};
 let child=start(); let browser;
 try {
  await ready();
  browser=await chromium.launch({headless:true});
  const login=async(page,language,next)=>{
   await page.goto(base+'/login.html'+(next?'?next='+encodeURIComponent(next):''));
   await page.locator('#loginLanguage').selectOption(language);
   await page.locator('#username').fill('admin');await page.locator('#password').fill('admin');
   await page.locator('#loginButton').click();await page.waitForURL(base+'/');
  };
  for(const language of ['zh-CN','en']){
   const context=await browser.newContext({viewport:{width:1280,height:1000},locale:language});
   const page=await context.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
   const say=(zh,en)=>language==='zh-CN'?zh:en;
   // Only same-site paths are followed after login; "/\host" counts as another host.
   await login(page,language,'/\\example.com');
   assert.equal(new URL(page.url()).host,new URL(base).host);
   await page.waitForFunction(title=>document.title.startsWith(title),say('首页','Home'));
   await page.locator('.sa-menu-link[data-page-link="settings"]').click();
   await page.waitForFunction(title=>document.title.startsWith(title),say('设置','Settings'));
   // Back and forward switch the page once, not once per history event.
   await page.locator('.sa-menu-link[data-page-link="network"]').click();
   const switches=await page.evaluate(async()=>{let count=0;addEventListener('simpleadmin:page-changed',()=>count++);history.back();await new Promise(r=>setTimeout(r,400));return count;});
   assert.equal(switches,1);
   assert.equal(new URL(page.url()).hash,'#settings');
   // Network feature changes report invalid input and the module's answer under the list.
   const message=page.locator('#networkFeatureMessage');
   await page.locator('#dmzInput').fill('192.168.1.300');
   await page.locator('#dmzInput').locator('xpath=..').getByRole('button').first().click();
   await assert.doesNotReject(message.waitFor({state:'visible'}));
   assert.match(await message.getAttribute('class'),/is-error/);
   assert.equal((await message.textContent()).trim(),say('请输入有效的 IP 地址','Enter a valid IP address'));
   await page.locator('#dmzInput').fill('192.168.225.20');
   const dmz=page.waitForResponse(()=>true).catch(()=>null);
   await page.locator('#dmzInput').locator('xpath=..').getByRole('button').first().click();
   await dmz;
   await page.waitForFunction(text=>document.querySelector('#networkFeatureMessage')?.textContent.trim()===text,say('已保存','Saved'));
   assert.doesNotMatch(await message.getAttribute('class')||'',/is-error/);
   await page.locator('#ttlInput').fill('300');
   await page.locator('#ttlInput').locator('xpath=..').getByRole('button').click();
   await page.waitForFunction(text=>document.querySelector('#ttlMessage')?.textContent.trim()===text,say('请输入 0–255 的 TTL 值','Enter a TTL value from 0 to 255'));
   await page.locator('#ttlInput').fill('64');
   await page.locator('#ttlInput').locator('xpath=..').getByRole('button').click();
   await page.waitForFunction(text=>document.querySelector('#ttlMessage')?.textContent.trim()===text,say('已保存','Saved'));
   // A restarted service forgets every session; the open page returns to the login page.
   await stop(child); child=start(); await ready();
   await page.locator('.sa-menu-link[data-page-link="dashboard"]').click();
   await page.waitForURL(base+'/login.html',{timeout:15000});
   assert.deepEqual(errors,[]);await context.close();console.log('PASS session and settings feedback '+language);
  }
  // Locked-out clients are told to wait instead of seeing "wrong password".
  const context=await browser.newContext({locale:'en'});const page=await context.newPage();
  await page.goto(base+'/login.html');await page.locator('#loginLanguage').selectOption('en');
  await page.locator('#username').fill('admin');await page.locator('#password').fill('wrong');
  for(let i=0;i<6;i++){
   const answer=page.waitForResponse(r=>r.url().endsWith('/api/login'));
   await page.locator('#loginButton').click();assert.equal((await answer).status(),401);
   await page.waitForFunction(()=>!document.querySelector('#loginButton').disabled);
  }
  assert.equal((await page.locator('#loginMessage').textContent()).trim(),'Incorrect username or password');
  const answer=page.waitForResponse(r=>r.url().endsWith('/api/login'));
  await page.locator('#loginButton').click();assert.equal((await answer).status(),429);
  await page.waitForFunction(()=>/^Too many attempts\. Try again in \d+ s$/.test(document.querySelector('#loginMessage').textContent.trim()));
  await context.close();console.log('PASS login lockout message');
 }finally{if(browser)await browser.close();await stop(child);}
})().catch(error=>{console.error(error);process.exitCode=1;});
