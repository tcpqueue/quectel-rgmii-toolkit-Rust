const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const https = require('node:https');
const {spawn} = require('node:child_process');

(async () => {
  const root = path.resolve(__dirname, '..');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-tls-'));
  const names = ['cert', 'key', 'ca-cert', 'ca-key'];
  const args = ['--mock', '--no-tls=false', '--http', '127.0.0.1:18084', '--https', '127.0.0.1:18443', '--static', path.join(root, 'development/simpleadmin/www'), '--auth-file', path.join(dir,'auth'), '--ttl-file', path.join(dir,'ttl')];
  for (const name of names) args.push('--'+name, path.join(dir,name+'.pem'));
  let previous;
  for (let run=0; run<2; run++) {
    const child = spawn(path.join(root,'target/debug/simpleadmin-httpd'), args, {stdio:['ignore','pipe','pipe']});
    let logs = ''; child.stderr.on('data', b => logs+=b);
    try {
      let ready = false;
      for (let i=0;i<100;i++) {
        try {
          const response = await fetch('http://127.0.0.1:18084/login.html', {redirect:'manual'});
          assert.equal(response.status,308);
          assert.equal(response.headers.get('location'),'https://127.0.0.1:18443/login.html');
          ready = true; break;
        } catch { await new Promise(r=>setTimeout(r,100)); }
      }
      assert(ready, logs);
      const page = await new Promise((resolve,reject) => {
        https.get('https://127.0.0.1:18443/login.html', {ca:fs.readFileSync(path.join(dir,'ca-cert.pem'))}, response => {
          let body='';response.on('data', b=>body+=b);response.on('end',()=>resolve({status:response.statusCode,body}));
        }).on('error',reject);
      });
      assert.equal(page.status,200);assert(page.body.includes('loginLanguage'));
      const state = names.map(name => ({bytes:fs.readFileSync(path.join(dir,name+'.pem'),'utf8'), mtime:fs.statSync(path.join(dir,name+'.pem')).mtimeMs}));
      if (previous) assert.deepEqual(state,previous,'restart must reuse certificates without writes');
      previous = state;
    } finally {child.kill();await new Promise(resolve=>child.once('exit',resolve));}
  }
  console.log('TLS CA validation, HTTPS, redirect and certificate reuse passed.');
})().catch(error=>{console.error(error);process.exitCode=1;});
