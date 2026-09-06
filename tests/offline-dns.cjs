const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const {spawn, spawnSync} = require('node:child_process');
const root = path.resolve(__dirname, '..');
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-dns-'));
const library = path.join(scratch, 'slow-dns.so');
const build = spawnSync('gcc', ['-shared', '-fPIC', path.join(__dirname, 'fixtures/slow-dns.c'), '-o', library], {encoding:'utf8'});
assert.equal(build.status, 0, build.stderr);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function get(port, url) {
  const start = Date.now();
  return new Promise(resolve => {
    const req = http.get({hostname:'127.0.0.1',port,path:url,agent:false}, res => {
      res.resume(); res.on('end',()=>resolve(`${url}: HTTP ${res.statusCode}, ${Date.now()-start}ms`));
    });
    const timer = setTimeout(()=>req.destroy(new Error('timeout')),2000);
    req.on('close',()=>clearTimeout(timer));
    req.on('error',err=>resolve(`${url}: ${err.message}, ${Date.now()-start}ms`));
  });
}
(async()=>{
  try { for (const [name, slow, target] of [['fast-dns-failure',0,'www.baidu.com'],['slow-dns-failure',25,'www.baidu.com'],['ip-target-with-slow-dns',25,'192.0.2.1']]) {
    const dir = path.join(scratch,name); fs.mkdirSync(dir,{recursive:true});
    fs.writeFileSync(path.join(dir,'auth'),'admin:test-only\n');
    fs.writeFileSync(path.join(dir,'monitor.json'),JSON.stringify({target}));
    const port = 24319;
    const proc = spawn(path.join(root,'target/debug/simpleadmin-httpd'),['--http',`127.0.0.1:${port}`,'--static',path.join(root,'development/simpleadmin/www'),'--auth-file',path.join(dir,'auth'),'--at-devices','/nonexistent-test-at','--ttl-file',path.join(dir,'ttl')],{env:{...process.env,LD_PRELOAD:library,TEST_DNS_DELAY:String(slow),SIMPLEADMIN_MANAGE_ROOTFS:'0'},stdio:['ignore','pipe','pipe']});
    let logs=''; proc.stdout.on('data',data=>logs+=data);proc.stderr.on('data',data=>logs+=data);
    try {
      await delay(500);
      assert.match(await get(port,'/login.html'), /HTTP 200/);
      await delay(6500);
      assert.match(await get(port,'/'), /HTTP 303/);
      assert.match(await get(port,'/login.html'), /HTTP 200/);
      assert.match(await get(port,'/js/locales.js'), /HTTP 200/);
      const jobs = (logs.match(/DNS start:/g)||[]).length;
      assert(jobs <= (slow === 0 ? 2 : target === 'www.baidu.com' ? 1 : 0), logs);
      console.log(`PASS ${name}: web responsive; ${jobs} DNS jobs`);
    } finally { proc.kill('SIGKILL'); await new Promise(resolve=>proc.once('exit',resolve)); }
  } } finally { fs.rmSync(scratch,{recursive:true,force:true}); }
})().catch(e=>{console.error(e);process.exitCode=1;});
