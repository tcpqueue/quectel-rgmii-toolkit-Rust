// Verify a real signed release using an older server's OTA implementation in preview mode.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const {spawn,execFileSync} = require('node:child_process');
const root = path.resolve(__dirname,'..');
const assets = path.resolve(process.env.RELEASE_DIR || path.join(root,'packages'));
const binary = process.env.OTA_TEST_BINARY;
assert(binary,'set OTA_TEST_BINARY to an older simpleadmin-httpd binary');
fs.accessSync(binary,fs.constants.X_OK);
const manifestBytes = fs.readFileSync(path.join(assets,'simpleadmin-ota.json'));
const manifest = JSON.parse(manifestBytes);
const signature = fs.readFileSync(path.join(assets,'simpleadmin-ota.json.sig'),'utf8');
const publicBytes = Buffer.from(fs.readFileSync(path.join(root,'src/ota-public-key.txt'),'utf8').trim(),'base64');
const publicKey = crypto.createPublicKey({format:'der',type:'spki',key:Buffer.concat([
  Buffer.from('302a300506032b6570032100','hex'),publicBytes
])});
assert(crypto.verify(null,manifestBytes,publicKey,Buffer.from(signature,'base64')),'official signature');
const packageBytes = fs.readFileSync(path.join(assets,manifest.package));
assert.equal(packageBytes.length,manifest.size);
assert.equal(crypto.createHash('sha256').update(packageBytes).digest('hex'),manifest.sha256);
const scratch = fs.mkdtempSync(path.join(os.tmpdir(),'simpleadmin-release-ota-'));
const files = {'/simpleadmin-ota.json':manifestBytes,'/simpleadmin-ota.json.sig':Buffer.from(signature),['/'+manifest.package]:packageBytes};
let tamper = false;
const releases = http.createServer((request,response)=>{
  let data = files[request.url];
  if (tamper && request.url==='/'+manifest.package) {
    data = Buffer.from(data);data[data.length-10]^=1;
  }
  response.writeHead(data?200:404);response.end(data||'');
});
const delay = ms=>new Promise(resolve=>setTimeout(resolve,ms));
let child, logs = '';
(async()=>{
  await new Promise(resolve=>releases.listen(0,'127.0.0.1',resolve));
  const source = `http://127.0.0.1:${releases.address().port}`;
  child = spawn(binary,['--mock','--http','127.0.0.1:0','--static',path.join(root,'development/simpleadmin/www'),
    '--auth-file',path.join(scratch,'auth'),'--ttl-file',path.join(scratch,'ttl')],{stdio:['ignore','pipe','pipe']});
  child.stdout.on('data',b=>logs+=b);child.stderr.on('data',b=>logs+=b);
  await new Promise((resolve,reject)=>child.once('spawn',resolve).once('error',reject));
  for(let i=0;i<100 && !logs.includes('HTTP listening on ');i++) await delay(50);
  const base = 'http://'+logs.match(/HTTP listening on ([^\s]+)/)?.[1];
  const login = await fetch(base+'/api/login',{method:'POST',body:'username=admin&password=admin'});
  assert.equal(login.status,200,logs);
  const cookie = login.headers.get('set-cookie').split(';')[0];
  async function request(url,body) {
    const response = await fetch(base+url,{method:body?'POST':'GET',headers:{cookie,'content-type':'application/json'},
      ...(body?{body:JSON.stringify(body)}:{}),signal:AbortSignal.timeout(10000)});
    assert.equal(response.status,200,await response.clone().text());
    return response.json();
  }
  await request('/api/ota/settings',{mode:'off',source,proxy:'',public_key:''});
  const check = await request('/api/ota/check',{});
  assert.equal(check.error,'');
  assert.equal(check.latest.version,manifest.version);
  assert.equal(check.latest.newer,true,'an older installed version must discover the release');
  async function install() {
    await request('/api/ota/install',{});
    for(let i=0;i<200;i++) {
      await delay(50);
      const status = await request('/api/ota');
      if(status.phase==='idle') return status;
    }
    throw new Error('install preview timed out');
  }
  tamper = true;
  const bad = await install();
  assert(bad.error,'a modified archive must be rejected');
  assert(!fs.existsSync(path.join(scratch,'ota-work/development')),'tampered package must not be extracted');
  tamper = false;
  const good = await install();
  assert.equal(good.error,'');
  const extracted = path.join(scratch,'ota-work/development');
  assert(fs.existsSync(path.join(extracted,'install_simpleadmin_rust.sh')));
  assert(fs.existsSync(path.join(extracted,'simpleadmin/systemd/simpleadmin-ota.service')));
  execFileSync('sha256sum',['-c','SHA256SUMS'],{cwd:extracted,stdio:'pipe'});
  const elf = fs.readFileSync(path.join(extracted,'simpleadmin/simpleadmin-httpd.armv7'));
  assert.equal(elf.subarray(0,4).toString('hex'),'7f454c46');
  assert.equal(elf[4],1,'ELF32');assert.equal(elf.readUInt16LE(18),40,'ARM');
  console.log(`PASS ${execFileSync(binary,['--version'],{encoding:'utf8'}).trim()} -> v${manifest.version}: official signature, newer version, download, tamper rejection, unpack, all payload checksums and ARM ELF`);
})().catch(e=>{console.error(e,logs);process.exitCode=1;}).finally(async()=>{
  if(child && child.exitCode===null) {
    const exited = new Promise(resolve=>child.once('exit',resolve));child.kill('SIGKILL');await exited;
  }
  await new Promise(resolve=>releases.close(resolve));
  fs.rmSync(scratch,{recursive:true,force:true});
});
