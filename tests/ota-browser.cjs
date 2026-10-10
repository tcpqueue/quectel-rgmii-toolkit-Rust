// Online update section: custom source, signature trust, check and (preview) install.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const {spawn, execFileSync} = require('node:child_process');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
async function freePort() {
 const server=net.createServer(); await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 const port=server.address().port; await new Promise(resolve=>server.close(resolve)); return port;
}
(async()=>{
 const root=path.resolve(__dirname,'..');
 const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'simpleadmin-ota-'));
 // A signed release folder: package, manifest and Ed25519 signature.
 const tree=path.join(temporary,'src/development');
 fs.mkdirSync(path.join(tree,'simpleadmin'),{recursive:true});
 fs.writeFileSync(path.join(tree,'SHA256SUMS'),'sums\n');
 fs.writeFileSync(path.join(tree,'install_simpleadmin_rust.sh'),'#!/bin/bash\n',{mode:0o755});
 fs.writeFileSync(path.join(tree,'simpleadmin/simpleadmin-httpd.armv7'),'ELF',{mode:0o755});
 const packagePath=path.join(temporary,'simpleadmin-ota-9.9.9.tar.gz');
 execFileSync('tar',['--format=ustar','-czf',packagePath,'-C',path.join(temporary,'src'),'development']);
 const packageBytes=fs.readFileSync(packagePath);
 const manifest=Buffer.from(JSON.stringify({version:'9.9.9',tag:'v9.9.9',package:'simpleadmin-ota-9.9.9.tar.gz',size:packageBytes.length,sha256:crypto.createHash('sha256').update(packageBytes).digest('hex')}));
 const {publicKey,privateKey}=crypto.generateKeyPairSync('ed25519');
 const signature=crypto.sign(null,manifest,privateKey).toString('base64');
 const publicRaw=publicKey.export({format:'der',type:'spki'}).subarray(-32).toString('base64');
 const files={'/ota/simpleadmin-ota.json':manifest,'/ota/simpleadmin-ota.json.sig':Buffer.from(signature),'/ota/simpleadmin-ota-9.9.9.tar.gz':packageBytes};
 const releases=http.createServer((request,response)=>{const body=files[request.url];response.writeHead(body?200:404);response.end(body||'');});
 const releasePort=await freePort(); await new Promise(resolve=>releases.listen(releasePort,'127.0.0.1',resolve));
 const source='http://127.0.0.1:'+releasePort+'/ota';

 const port=await freePort(); const base='http://127.0.0.1:'+port;
 const child=spawn(path.join(root,'target/debug/simpleadmin-httpd'),['--mock','--http','127.0.0.1:'+port,'--static',path.join(root,'development/simpleadmin/www'),'--auth-file',path.join(temporary,'auth'),'--ttl-file',path.join(temporary,'ttl')],{stdio:'ignore'});
 let browser;
 try {
  for(let i=0;i<100;i++){try{await fetch(base+'/login.html');break;}catch{await new Promise(r=>setTimeout(r,100));}}
  browser=await chromium.launch({headless:true});
  for(const language of ['zh-CN','en']){
   const say=(zh,en)=>language==='zh-CN'?zh:en;
   const context=await browser.newContext({viewport:{width:1280,height:1000},locale:language});
   const page=await context.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
   page.on('dialog',dialog=>dialog.accept());
   await page.goto(base+'/login.html');await page.locator('#loginLanguage').selectOption(language);
   await page.locator('#username').fill('admin');await page.locator('#password').fill('admin');
   await page.locator('#loginButton').click();await page.waitForURL(base+'/');
   await page.locator('.sa-menu-link[data-page-link="settings"]').click();
   await page.waitForFunction(()=>/^v\d+\.\d+\.\d+$/.test(document.querySelector('#otaCurrent')?.textContent||''));
   assert.equal((await page.locator('#otaHeading').textContent()).trim(),say('在线更新','Online update'));
   const message=page.locator('#otaMessage');
   // Invalid proxy is refused with a readable message.
   await page.locator('#otaProxy').fill('ghfast.top');
   await page.locator('.sa-ota-settings button[type="submit"]').click();
   await page.waitForFunction(text=>document.querySelector('.sa-ota-settings .ui-status')?.textContent.includes(text),say('GitHub 代理需为 http(s) 地址','The GitHub proxy must be an http(s) address'));
   await page.locator('#otaProxy').fill('');
   // A custom source is not trusted until its key is entered.
   await page.locator('#otaSource').fill(source);
   await page.locator('#otaKey').fill('');
   await page.locator('.sa-ota-settings button[type="submit"]').click();
   await page.waitForFunction(text=>document.querySelector('.sa-ota-settings .ui-status')?.textContent.trim()===text,say('已保存','Saved'));
   await page.locator('#otaCheckButton').click();
   await page.waitForFunction(text=>document.querySelector('#otaMessage')?.textContent.includes(text),say('更新签名无效或不受信任','Update signature is invalid or not trusted'));
   assert.match(await message.getAttribute('class'),/is-error/);
   assert.equal(await page.locator('#otaInstallButton').isVisible(),false);
   await page.locator('#otaKey').fill(publicRaw);
   await page.locator('.sa-ota-settings button[type="submit"]').click();
   await page.waitForFunction(text=>document.querySelector('.sa-ota-settings .ui-status')?.textContent.trim()===text,say('已保存','Saved'));
   await page.locator('#otaCheckButton').click();
   await page.waitForFunction(()=>document.querySelector('#otaLatest')?.textContent.trim()==='v9.9.9');
   assert.equal((await message.textContent()).trim(),say('发现新版本','New version available'));
   await page.locator('#otaInstallButton').click();
   await page.waitForFunction(text=>document.querySelector('#otaMessage')?.textContent.trim()===text,say('预览模式：安装包已校验并解包，未执行安装','Preview mode: the package was verified and unpacked but not installed'),{timeout:20000});
   assert.equal(fs.existsSync(path.join(temporary,'ota-work/development/install_simpleadmin_rust.sh')),true);
   // Saved settings come back after a reload.
   await page.reload();await page.locator('.sa-menu-link[data-page-link="settings"]').click();
   await page.waitForFunction(value=>document.querySelector('#otaSource')?.value===value,source);
   assert.equal(await page.locator('#otaKey').inputValue(),publicRaw);
   await page.setViewportSize({width:390,height:900});
   await page.locator('#otaHeading').scrollIntoViewIfNeeded();
   assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1),false);
   if(language==='zh-CN'&&process.env.CAPTURE_DOCS){await page.setViewportSize({width:1280,height:1000});await page.locator('#otaHeading').scrollIntoViewIfNeeded();await page.waitForTimeout(500);await page.screenshot({path:path.join(root,'docs/images/ota.png')});}
   assert.deepEqual(errors,[]);await context.close();console.log('PASS online update '+language);
  }
 }finally{if(browser)await browser.close();child.kill();releases.close();await new Promise(resolve=>child.once('exit',resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
