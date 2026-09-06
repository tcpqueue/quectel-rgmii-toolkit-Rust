const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const vm = require('node:vm');
const {spawn} = require('node:child_process');
const root = path.resolve(__dirname,'..');
const dir = fs.mkdtempSync(path.join(os.tmpdir(),'simpleadmin-resource-'));
const base = 'http://127.0.0.1:24321';
const wait = ms => new Promise(resolve=>setTimeout(resolve,ms));
const child = spawn(path.join(root,'target/debug/simpleadmin-httpd'),['--mock','--http','127.0.0.1:24321','--static',path.join(root,'development/simpleadmin/www'),'--auth-file',path.join(dir,'auth')],{stdio:'ignore'});
const sockets=[];
function raw(request) {
  return new Promise((resolve,reject)=>{
    const socket=net.connect(24321,'127.0.0.1',()=>socket.write(request)); sockets.push(socket);
    let data=''; socket.on('data',chunk=>{data+=chunk;if(data.includes('\r\n')) {socket.destroy();resolve(data.split('\r\n')[0]);}});
    socket.on('error',reject); socket.setTimeout(8000,()=>{socket.destroy();reject(new Error('response timeout'));});
  });
}
(async()=>{try {
  for(let i=0;i<50;i++){try{await fetch(base+'/login.html');break;}catch{await wait(100);}}
  const login=await fetch(base+'/api/login',{method:'POST',body:'username=admin&password=admin'});
  assert.equal(login.status,200); const cookie=login.headers.get('set-cookie').split(';')[0];
  const get=async url=>(await fetch(base+url,{headers:{cookie}})).json();
  await wait(1200);
  const first=await get('/api/telemetry');
  const delta=await get('/api/telemetry?cursor='+encodeURIComponent(JSON.stringify(first.cursor)));
  assert.equal(delta.delta,true); assert.equal(delta.ping.length,0);
  await wait(1200);
  const next=await get('/api/telemetry?cursor='+encodeURIComponent(JSON.stringify(first.cursor)));
  assert(next.ping.length>0);
  const context={window:{addEventListener(){}},document:{readyState:'loading',addEventListener(){}}};
  vm.createContext(context);vm.runInContext(fs.readFileSync(path.join(root,'development/simpleadmin/www/js/monitor.js'),'utf8'),context);
  const merge=context.window.SimpleAdminMonitor.mergeSnapshot;
  const merged=merge(first,next); assert(merged.ping.length>first.ping.length);
  assert.equal(merge(merged,next).ping.length,merged.ping.length);
  const expired=merge(merged,{...next,serverTime:next.serverTime+301000,ping:[],signal:[],traffic:[]});
  assert.equal(expired.ping.length,0);
  console.log('PASS telemetry delta merge, duplicate response and expiry');
  assert.match(await raw('POST /api/login HTTP/1.1\r\nHost: localhost\r\nContent-Length: 65537\r\n\r\n'),/413/);
  assert.match(await raw('POST /api/login HTTP/1.1\r\nHost: localhost\r\nContent-Length: 10\r\n\r\nx'),/408/);
  console.log('PASS oversized body and stalled body deadlines');
  let closed=0;
  for(let i=0;i<35;i++) {
    const socket=net.connect(24321,'127.0.0.1',()=>socket.write('GET /login.html HTTP/1.1\r\nHost:'));
    sockets.push(socket);socket.on('data',()=>{});socket.on('error',()=>{});socket.on('close',()=>closed++);
  }
  await wait(1000);assert(closed>=3,'excess connections should close');
  await wait(10500);assert.equal(closed,35,'stalled headers should time out');
  assert.equal((await fetch(base+'/login.html')).status,200);
  console.log('PASS connection cap, stalled header expiry and subsequent recovery');
}finally{for(const socket of sockets)socket.destroy();child.kill('SIGKILL');await new Promise(r=>child.once('exit',r));fs.rmSync(dir,{recursive:true,force:true});}})().catch(e=>{console.error(e);process.exitCode=1;});
