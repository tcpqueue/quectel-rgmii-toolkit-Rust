// Exercise real ICMP sockets in an isolated Linux network namespace.
// Run after cargo build: sudo unshare -n sh -c 'ip link set lo up; node tests/offline-ping.cjs'
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const {spawn, execFileSync} = require('node:child_process');
const root = path.resolve(__dirname, '..');
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-icmp-'));
const library = path.join(scratch, 'icmp-errors.so');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const hz = Number(execFileSync('getconf', ['CLK_TCK'], {encoding:'utf8'}).trim());
execFileSync('gcc', ['-Wall','-shared','-fPIC',path.join(__dirname,'fixtures/icmp-errors.c'),'-ldl','-o',library]);
function ticks(pid) {
  const raw = fs.readFileSync(`/proc/${pid}/stat`,'utf8');
  const fields = raw.slice(raw.lastIndexOf(')')+2).split(/\s+/);
  return Number(fields[11])+Number(fields[12]);
}
function sockets(pid) {
  const inodes = new Set();
  for (const name of ['icmp','icmp6','raw','raw6']) {
    for (const line of fs.readFileSync(`/proc/${pid}/net/${name}`,'utf8').trim().split('\n').slice(1)) {
      const fields = line.trim().split(/\s+/);
      inodes.add(fields[9]);
    }
  }
  let count = 0;
  for (const fd of fs.readdirSync(`/proc/${pid}/fd`)) {
    try {
      const inode = fs.readlinkSync(`/proc/${pid}/fd/${fd}`).match(/^socket:\[(\d+)\]$/)?.[1];
      if (inodes.has(inode)) count++;
    } catch (e) {if (e.code !== 'ENOENT') throw e;}
  }
  return count;
}
async function until(read, check, label, timeout = 5000) {
  const end = performance.now()+timeout;
  let value;
  do {
    value = await read();
    if (check(value)) return value;
    await delay(50);
  } while (performance.now()<end);
  throw new Error(`${label}: ${JSON.stringify(value)}`);
}
async function freePort() {
  const server = net.createServer();
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const port = server.address().port;
  await new Promise(resolve=>server.close(resolve));
  return port;
}
async function run(raw) {
  const dir = path.join(scratch, raw?'raw':'dgram');
  fs.mkdirSync(dir);
  fs.writeFileSync(path.join(dir,'auth'),'admin:test-only\n');
  const schedule = {ping_enabled:true,ping_interval:1,sample_enabled:false,sample_interval:5};
  fs.writeFileSync(path.join(dir,'monitor.json'),JSON.stringify({target:'127.0.0.1',...schedule}));
  const fault = path.join(dir,'fault');
  const drop = path.join(dir,'drop');
  const port = await freePort();
  const base = `http://127.0.0.1:${port}`;
  const child = spawn(path.join(root,'target/debug/simpleadmin-httpd'),[
    '--http',`127.0.0.1:${port}`,'--static',path.join(root,'development/simpleadmin/www'),
    '--auth-file',path.join(dir,'auth'),'--ttl-file',path.join(dir,'ttl')
  ],{env:{...process.env,LD_PRELOAD:library,TEST_ICMP_FAULT_FILE:fault,TEST_ICMP_DROP_FILE:drop,
    ...(raw?{TEST_ICMP_RAW:'1'}:{}),SIMPLEADMIN_MANAGE_ROOTFS:'0'},stdio:['ignore','pipe','pipe']});
  let logs = '', cookie = '';
  child.stdout.on('data',b=>logs+=b);
  child.stderr.on('data',b=>logs+=b);
  async function request(url, body) {
    const response = await fetch(base+url,{method:body?'POST':'GET',
      headers:{cookie,'content-type':'application/json'},
      ...(body?{body:JSON.stringify(body)}:{}),signal:AbortSignal.timeout(2000)});
    assert.equal(response.status,200,`${url}: ${await response.clone().text()}`);
    return response.json();
  }
  const telemetry = ()=>request('/api/telemetry');
  const last = data=>data.ping.at(-1);
  try {
    await until(async()=>{try{return (await fetch(base+'/login.html')).status;}catch{return 0;}},x=>x===200,'startup');
    const login = await fetch(base+'/api/login',{method:'POST',body:'username=admin&password=test-only'});
    assert.equal(login.status,200);
    cookie = login.headers.get('set-cookie').split(';')[0];
    await until(telemetry,x=>last(x)?.status==='ok','IPv4 echo reply (ICMP permissions required)');
    assert.equal(sockets(child.pid),1);
    fs.writeFileSync(fault,'1');
    await until(telemetry,x=>last(x)?.status==='unavailable','receive error recorded');
    const c0 = ticks(child.pid), t0 = performance.now();
    await delay(3200);
    const cpu = 100*(ticks(child.pid)-c0)/hz/((performance.now()-t0)/1000);
    assert(cpu<25,`persistent receive error used ${cpu.toFixed(1)}% of a core`);
    const errors = (logs.match(/ICMP_RECV_ERROR/g)||[]).length;
    assert(errors>=2 && errors<=8,`receiver must stop, with at most one retry per probe: ${errors}`);
    // Disabling also closes the socket and receiver while the fault is still present.
    await request('/api/telemetry/schedule',{...schedule,ping_enabled:false});
    await until(()=>sockets(child.pid),x=>x===0,'disabled probe releases all ICMP sockets');
    const before = (logs.match(/ICMP_RECV_ERROR/g)||[]).length;
    await delay(1200);
    assert.equal((logs.match(/ICMP_RECV_ERROR/g)||[]).length,before);
    fs.unlinkSync(fault);
    await request('/api/telemetry/schedule',schedule);
    await until(telemetry,x=>last(x)?.status==='ok','IPv4 probe recovers after re-enable');
    await request('/api/telemetry/target',{target:'::1'});
    await until(telemetry,x=>last(x)?.ip==='::1' && last(x)?.status==='ok','IPv6 echo reply');
    assert.equal(sockets(child.pid),2);
    // Consume the echo reply and leave the current probe waiting until disabled.
    fs.writeFileSync(drop,'1');
    await until(()=>logs,x=>x.includes('ICMP_DROP'),'probe waiting for a dropped reply');
    const changed = performance.now();
    await request('/api/telemetry/schedule',{...schedule,ping_enabled:false});
    await until(()=>sockets(child.pid),x=>x===0,'disable closes IPv4 and IPv6 sockets',750);
    assert(performance.now()-changed<1000);
    fs.unlinkSync(drop);
    await request('/api/telemetry/target',{target:'192.0.2.1'});
    await request('/api/telemetry/schedule',schedule);
    await until(telemetry,x=>last(x)?.status==='unavailable','missing route reported without retry loop');
    const n0 = ticks(child.pid), s0 = performance.now();
    await delay(2200);
    const offlineCpu = 100*(ticks(child.pid)-n0)/hz/((performance.now()-s0)/1000);
    assert(offlineCpu<25,`missing route used ${offlineCpu.toFixed(1)}% of a core`);
    await request('/api/telemetry/target',{target:'127.0.0.1'});
    await until(telemetry,x=>last(x)?.status==='ok','recovery after missing route');
    console.log(`PASS ${raw?'RAW':'DGRAM'}: receive-error CPU ${cpu.toFixed(1)}%, ${errors} errors; missing-route CPU ${offlineCpu.toFixed(1)}%; IPv4/IPv6, disable and recovery`);
  } catch(e) {
    e.message+='\n'+logs;
    throw e;
  } finally {
    const exited = new Promise(resolve=>child.once('exit',resolve));
    child.kill('SIGKILL');
    await exited;
  }
}
(async()=>{
  const routes = execFileSync('ip',['route','show'],{encoding:'utf8'}).trim();
  assert.equal(routes,'','run in an isolated namespace with only loopback');
  await run(false);
  await run(true);
})().catch(e=>{console.error(e);process.exitCode=1;}).finally(()=>fs.rmSync(scratch,{recursive:true,force:true}));
