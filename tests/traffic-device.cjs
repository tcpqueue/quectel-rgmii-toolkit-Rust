const assert = require('node:assert/strict');
(async()=>{
  const base=process.env.DEVICE_URL||'http://127.0.0.1:18081';
  const login=await fetch(base+'/api/login',{method:'POST',body:new URLSearchParams({username:process.env.DEVICE_USER||'admin',password:process.env.DEVICE_PASSWORD||'admin'})});
  assert.equal(login.status,200);
  const cookie=login.headers.get('set-cookie').split(';')[0];
  async function read(endpoint){const r=await fetch(base+endpoint,{headers:{cookie}});assert.equal(r.status,200);return r.json();}
  let previous,duplicates=0;const samples=[];
  for(let i=0;i<22;i++){
    const data=await read('/api/dashboard_data');
    assert.equal(data.traffic_rates,true);
    const current={sample:data.trafficSampleTime,download:data.nr_dl_speed,upload:data.nr_ul_speed};
    if(previous&&current.sample===previous.sample){assert.deepEqual(current,previous);duplicates++;}
    else samples.push(current);
    previous=current;
    await new Promise(r=>setTimeout(r,1000));
  }
  const history=await read('/api/telemetry');
  assert(history.traffic.length>2&&history.traffic.length<=60);
  assert(history.traffic.some(p=>p.download!==null&&p.upload!==null));
  assert(duplicates>5&&samples.length>2);
  const summary=history.trafficSummary,total=summary.downloadBytes+summary.uploadBytes;
  if(total){assert.equal(summary.downloadShare,Math.round(summary.downloadBytes/total*1000)/10);assert.equal(Math.round((summary.downloadShare+summary.uploadShare)*10),1000);}
  console.log(JSON.stringify({duplicateSamplesKept:duplicates,newSamples:samples.length,points:history.traffic.length,shareVerified:true,rates:samples}));
})().catch(e=>{console.error(e);process.exitCode=1;});
