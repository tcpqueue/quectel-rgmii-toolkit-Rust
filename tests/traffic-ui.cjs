const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const root = path.resolve(__dirname,'..');
const SimpleAdmin = {Pages:{},Mask:{getSensitiveVisible:()=>false}};
const window = {SimpleAdmin,SimpleAdminSpaMode:true,addEventListener:()=>{}};
const context = vm.createContext({window,SimpleAdmin,document:{readyState:'loading',addEventListener:()=>{}}});
vm.runInContext(fs.readFileSync(path.join(root,'development/simpleadmin/www/js/pages/index.js'),'utf8'),context);
const app = SimpleAdmin.Pages.dashboard();
const sample = {traffic_rates:true,nr_rx_bytes:20000,nr_tx_bytes:10000,nr_dl_speed:'2.0 KB/s',nr_ul_speed:'1.0 KB/s'};
for (let i=0;i<5;i++) {
  app.updateTraffic(sample);
  assert.equal(app.nr_dl_speed,'2.0 KB/s');
  assert.equal(app.nr_ul_speed,'1.0 KB/s');
}
app.updateTraffic({...sample,nr_dl_speed:'0 B/s',nr_ul_speed:'0 B/s'});
assert.equal(app.nr_dl_speed,'0 B/s');
app.updateTraffic({...sample,nr_dl_speed:'-',nr_ul_speed:'-'});
assert.equal(app.nr_dl_speed,'-');
vm.runInContext(fs.readFileSync(path.join(root,'development/simpleadmin/www/js/monitor.js'),'utf8'),context);
const options = window.SimpleAdminMonitor.chartOptions({serverTime:300000,signal:[],ping:[],traffic:[{time:5000,download:null,upload:null},{time:10000,download:2097152,upload:524288}]},'NR',{},true).traffic;
assert.equal(options.yAxis.name,'MB/s');
assert.equal(options.series[0].data[0][1],null);
assert.equal(options.series[0].data[1][1],2);
assert.equal(options.series[1].data[1][1],0.5);
console.log('Repeated cache samples, real zero, missing data and traffic chart units passed.');
