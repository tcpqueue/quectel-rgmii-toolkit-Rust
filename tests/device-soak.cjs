const assert = require('node:assert/strict');

(async () => {
  const base = process.env.DEVICE_URL || 'http://127.0.0.1:18081';
  const login = await fetch(base+'/api/login', {method:'POST',body:new URLSearchParams({username:process.env.DEVICE_USER||'admin',password:process.env.DEVICE_PASSWORD||'admin'})});
  assert.equal(login.status,200);
  const cookie = login.headers.get('set-cookie').split(';')[0];
  async function request(endpoint, options={}) {
    const r = await fetch(base+endpoint, {...options, headers:{cookie,...options.headers}});
    assert.equal(r.status,200,endpoint);
    return r.json();
  }
  for (const command of ['ATI','AT+CGMM','AT+CSQ','AT+QENG="servingcell"','AT+QTEMP','AT+CPIN?']) {
    const data = await request('/api/settings_data', {method:'POST',body:new URLSearchParams({action:'manual_at',command})});
    assert.equal(data.ok,true,command);
  }
  const initial = await request('/api/telemetry');
  await request('/api/telemetry/target',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({target:initial.target})});
  console.log('Read-only AT checks passed. Waiting 310 seconds with no history requests.');
  await new Promise(resolve=>setTimeout(resolve,310000));
  const data = await request('/api/telemetry');
  assert(data.ping.length>=295 && data.ping.length<=300);
  assert(data.signal.length>=58 && data.signal.length<=60);
  assert(data.ping.every(p=>p.time>data.serverTime-300000));
  assert(data.signal.every(p=>p.time>data.serverTime-300000));
  let differences = [];
  for (let i=0;i<data.ping.length;i++) {
    const p=data.ping[i],prev=data.ping[i-1];
    if (prev && prev.rtt!==null && p.rtt!==null && p.time>prev.time && p.time-prev.time<=1500) {
      const delta=Math.round(Math.abs(p.rtt-prev.rtt)*10)/10;
      assert.equal(p.jitter,delta);differences.push(delta);
    } else {assert.equal(p.jitter,null);}
  }
  assert.equal(data.summary.jitter,differences.length ? Math.round(differences.reduce((a,b)=>a+b,0)/differences.length*10)/10 : null);
  const interval = points => {const spans=points.slice(1).map((p,i)=>p.time-points[i].time).sort((a,b)=>a-b);return spans[Math.floor(spans.length/2)];};
  assert(Math.abs(interval(data.ping)-1000)<100);
  assert(Math.abs(interval(data.signal)-5000)<150);
  assert(data.signal.some(p=>p.rsrpNR!==null && p.sinrNR!==null && p.temperature!==null));
  console.log(JSON.stringify({pingPoints:data.ping.length,signalPoints:data.signal.length,pingIntervalMs:interval(data.ping),signalIntervalMs:interval(data.signal),jitterRecalculation:'passed',successfulPings:data.summary.received,backgroundCollection:'passed'}));
})().catch(error=>{console.error(error);process.exitCode=1;});
