const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const {createHash} = require('node:crypto');
const {spawn, spawnSync} = require('node:child_process');

const root = path.resolve(__dirname, '..');
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'simpleadmin-installer-'));
let passed = 0;
function manifest(dir) {
  const lines = [];
  function visit(relative) {
    for (const entry of fs.readdirSync(path.join(dir, relative), {withFileTypes:true})) {
      const name = path.posix.join(relative, entry.name);
      if (entry.isDirectory()) visit(name);
      else lines.push(`${createHash('sha256').update(fs.readFileSync(path.join(dir,name))).digest('hex')}  ${name}`);
    }
  }
  visit('simpleadmin');
  fs.writeFileSync(path.join(dir, 'SHA256SUMS'), lines.join('\n')+'\n');
}
function fixture(name) {
  const dir = path.join(scratch, name);
  fs.mkdirSync(dir);
  fs.cpSync(path.join(root,'development'),path.join(dir,'package'),{recursive:true});
  fs.writeFileSync(path.join(dir,'package/simpleadmin/simpleadmin-httpd.armv7'),'#!/bin/sh\necho fixture-version\n');
  manifest(path.join(dir,'package'));
  fs.mkdirSync(path.join(dir,'installed'));
  fs.writeFileSync(path.join(dir,'installed/simpleadmin.auth'),'admin:existing-secret\n');
  fs.writeFileSync(path.join(dir,'installed/forwarding.json'),'{"fixture":true}\n');
  fs.writeFileSync(path.join(dir,'post_boot'),'#!/bin/sh\n');
  return dir;
}
const setup = `
source "$1/development/install_simpleadmin_rust.sh"
ORIGINAL_RESTART="$(declare -f restart_services)"
PKG_DIR="$2/package"
SIMPLEADMIN_SRC="$PKG_DIR/simpleadmin"
SIMPLEADMIN_DIR="$2/installed"
INSTALL_RESULT_FILE="$2/result"
REBOOT_MARKER_FILE="$2/reboot"
TTL_VALUE_FILE="$SIMPLEADMIN_DIR/ttlvalue"
AT_DEVICES_FILE="$SIMPLEADMIN_DIR/at_devices.conf"
ROOT_BIN="$2/bin"
MOBILEAP_HELPER_SRC="$SIMPLEADMIN_SRC/mobileap_bridge0_mac.sh"
MOBILEAP_HELPER_SCRIPT="$SIMPLEADMIN_DIR/mobileap_bridge0_mac.sh"
MOBILEAP_RESULT_FILE="$2/mobileap-result"
POST_BOOT_FILE="$2/post_boot"
FALLBACK_START_SCRIPT="$SIMPLEADMIN_DIR/start_simpleadmin.sh"
FALLBACK_STOP_SCRIPT="$SIMPLEADMIN_DIR/stop_simpleadmin.sh"
PORT_PREPARE_SCRIPT="$SIMPLEADMIN_DIR/prepare_simpleadmin_ports.sh"
TRACE="$2/trace"
id() { echo 0; }
df() { printf 'Filesystem 1024-blocks Used Available Capacity Mounted\\nfixture 999999 0 999999 0%% /usrdata\\n'; }
mount() { echo "$*" >> "$TRACE"; }
sync() { :; }
stop_existing_simpleadmin_runtime() { echo STOP >> "$TRACE"; }
install_systemd_unit() { SERVICE_UNIT_INSTALLED=1; }
restart_services() { echo START >> "$TRACE"; }
`;
function run(dir, script) {
  return spawnSync('bash',['-c',setup+'\n'+script,'installer-test',root,dir],{encoding:'utf8',timeout:20000});
}
function check(name, fn) { fn(); passed++; console.log(`PASS ${name}`); }
function status(result, expected) { assert.equal(result.status,expected,result.stdout+'\n'+result.stderr); }
function portDefinitions(dir) {
  const script=fs.readFileSync(path.join(dir,'installed/prepare_simpleadmin_ports.sh'),'utf8');
  return script.slice(0,script.lastIndexOf('\nopen_web_port || exit 1'))
    .replace('. /usrdata/simpleadmin/runtime_processes.sh','')
    .replace('. /usrdata/simpleadmin/web_port.sh',`. "${root}/development/simpleadmin/web_port.sh"`)
    .replace('$(simpleadmin_read_http_port)',`$(simpleadmin_read_http_port "${dir}/installed/http_port")`);
}

(async()=>{
  let server;
  try {
    check('complete install preserves credentials/settings and restores read-only root',()=>{
      const dir=fixture('success'); const result=run(dir,'main'); status(result,0);
      assert.match(fs.readFileSync(path.join(dir,'result'),'utf8'),/INSTALL_STATUS=OK/);
      assert.equal(fs.readFileSync(path.join(dir,'installed/simpleadmin.auth'),'utf8'),'admin:existing-secret\n');
      assert.equal(fs.readFileSync(path.join(dir,'installed/forwarding.json'),'utf8'),'{"fixture":true}\n');
      assert.equal(fs.readFileSync(path.join(dir,'trace'),'utf8'),'-o remount,rw /\nSTOP\nSTART\n-o remount,ro /\n');
      assert(!fs.existsSync(path.join(dir,'installed/bridge0_mac')),'default install must not change MAC');
      assert.equal(fs.readFileSync(path.join(dir,'installed/http_port'),'utf8'),'80\n');
    });
    check('invalid installation credentials stop before remount or service stop',()=>{
      const dir=fixture('invalid-credentials');
      fs.writeFileSync(path.join(dir,'package/install-credentials.json'),'{"web_password":"do-not-log-secret"}');
      fs.writeFileSync(path.join(dir,'package/simpleadmin/simpleadmin-httpd.armv7'),
        '#!/bin/sh\nif [ "$1" = install-credentials ]; then cat >/dev/null; exit 1; fi\necho fixture-version\n');
      manifest(path.join(dir,'package'));
      const result=run(dir,'main'); status(result,1);
      assert(!fs.existsSync(path.join(dir,'trace')));
      assert(!result.stdout.includes('do-not-log-secret') && !result.stderr.includes('do-not-log-secret'));
    });
    check('credentials are validated before writes and applied before root default initialization',()=>{
      const dir=fixture('credentials-order');
      const input='{"web_username":"owner","web_password":"do-not-log-secret"}';
      fs.writeFileSync(path.join(dir,'package/install-credentials.json'),input);
      fs.writeFileSync(path.join(dir,'package/simpleadmin/simpleadmin-httpd.armv7'),
        '#!/bin/sh\ncase "$1" in\ninstall-credentials)\n if [ "$2" = --check ]; then cat >/dev/null; echo CHECK >> "'+dir+'/trace"; else cat > "'+dir+'/received"; echo CREDENTIALS >> "'+dir+'/trace"; fi;;\nroot-password-init) echo ROOT_DEFAULT >> "'+dir+'/trace";;\n*) echo fixture-version;;\nesac\n');
      manifest(path.join(dir,'package'));
      const result=run(dir,'main'); status(result,0);
      assert.equal(fs.readFileSync(path.join(dir,'received'),'utf8'),input);
      assert.equal(fs.readFileSync(path.join(dir,'trace'),'utf8'),'CHECK\n-o remount,rw /\nSTOP\nCREDENTIALS\nROOT_DEFAULT\nSTART\n-o remount,ro /\n');
      assert(!fs.existsSync(path.join(dir,'package/install-credentials.json')));
      assert(!result.stdout.includes('do-not-log-secret') && !result.stderr.includes('do-not-log-secret'));
    });
    check('credential save failure restores read-only root and does not initialize defaults',()=>{
      const dir=fixture('credentials-save-failure');
      fs.writeFileSync(path.join(dir,'package/install-credentials.json'),'{}');
      fs.writeFileSync(path.join(dir,'package/simpleadmin/simpleadmin-httpd.armv7'),
        '#!/bin/sh\nif [ "$1" = install-credentials ]; then cat >/dev/null; [ "$2" = --check ]; exit $?; fi\nif [ "$1" = root-password-init ]; then echo BAD_ROOT_INIT >> "'+dir+'/trace"; fi\necho fixture-version\n');
      manifest(path.join(dir,'package'));
      const result=run(dir,'main'); status(result,1);
      const trace=fs.readFileSync(path.join(dir,'trace'),'utf8');
      assert(trace.endsWith('-o remount,ro /\n'));
      assert(!trace.includes('BAD_ROOT_INIT') && !trace.includes('START'));
    });
    check('custom port is persisted, preserved by upgrade, and can be changed',()=>{
      const dir=fixture('custom-port');
      status(run(dir,'SIMPLEADMIN_HTTP_PORT=18089; main'),0);
      const file=path.join(dir,'installed/http_port');
      assert.equal(fs.readFileSync(file,'utf8'),'18089\n');
      const before=fs.statSync(file).mtimeMs;
      status(run(dir,'main'),0);
      assert.equal(fs.readFileSync(file,'utf8'),'18089\n');
      assert.equal(fs.statSync(file).mtimeMs,before,'unchanged port must not be rewritten');
      status(run(dir,'SIMPLEADMIN_HTTP_PORT=18090; main'),0);
      assert.equal(fs.readFileSync(file,'utf8'),'18090\n');
    });
    check('invalid ports fail before remount and service stop',()=>{
      for(const [i,port] of ['', '0','65536','-1','abc','080','80;touch /tmp/injected','999999999999999999'].entries()) {
        const dir=fixture('invalid-port-'+i);
        status(run(dir,`SIMPLEADMIN_HTTP_PORT='${port}'; main`),1);
        assert(!fs.existsSync(path.join(dir,'trace')));
      }
    });
    check('invalid saved port fails safely and can be repaired explicitly',()=>{
      const dir=fixture('invalid-saved-port');
      fs.writeFileSync(path.join(dir,'installed/http_port'),'invalid\n');
      status(run(dir,'main'),1);
      assert(!fs.existsSync(path.join(dir,'trace')));
      status(run(dir,'SIMPLEADMIN_HTTP_PORT=18091; main'),0);
    });
    check('port reader supports boundaries and never evaluates shell content',()=>{
      const file=path.join(scratch,'port-data');
      for(const port of ['1','65535','$(echo 8080)','80\n8080']) {
        fs.writeFileSync(file,port);
        const result=spawnSync('sh',['-c',`. "${root}/development/simpleadmin/web_port.sh"; simpleadmin_read_http_port "${file}"`],{encoding:'utf8'});
        status(result,['1','65535'].includes(port)?0:1);
      }
    });
    check('service launcher reads the saved port on each start',()=>{
      const dir=fixture('launcher-port');
      const installed=path.join(dir,'installed');
      const helper=fs.readFileSync(path.join(root,'development/simpleadmin/web_port.sh'),'utf8').replaceAll('/usrdata/simpleadmin',installed);
      fs.writeFileSync(path.join(installed,'web_port.sh'),helper);
      fs.writeFileSync(path.join(installed,'simpleadmin-httpd'),'#!/bin/sh\nprintf "%s\\n" "$@" > "'+dir+'/args"\n',{mode:0o755});
      const launcher=fs.readFileSync(path.join(root,'development/simpleadmin/run_simpleadmin.sh'),'utf8')
        .replaceAll('/usrdata/simpleadmin',installed).replace('/tmp/simpleadmin-startup.log',path.join(dir,'startup.log'));
      for(const port of ['8080','65535']) {
        fs.writeFileSync(path.join(installed,'http_port'),port+'\n');
        status(spawnSync('sh',['-c',launcher],{encoding:'utf8'}),0);
        assert(fs.readFileSync(path.join(dir,'args'),'utf8').includes('-http\n:'+port+'\n-no-tls\n'));
      }
    });
    check('missing or corrupt web files fail before stopping the old service',()=>{
      for(const mode of ['missing','corrupt']) {
        const dir=fixture(mode); const file=path.join(dir,'package/simpleadmin/www/js/simpleadmin-spa.js');
        if(mode==='missing') fs.unlinkSync(file); else fs.appendFileSync(file,'corruption');
        status(run(dir,'main'),1);
        assert(!fs.existsSync(path.join(dir,'trace')));
        assert.match(fs.readFileSync(path.join(dir,'result'),'utf8'),/INSTALL_STATUS=FAIL/);
      }
    });
    check('unsupported executable fails before writes or service stop',()=>{
      const dir=fixture('bad-arch');
      fs.writeFileSync(path.join(dir,'package/simpleadmin/simpleadmin-httpd.armv7'),'#!/bin/sh\nexit 126\n');
      manifest(path.join(dir,'package')); status(run(dir,'main'),1);
      assert(!fs.existsSync(path.join(dir,'trace')));
    });
    check('insufficient storage fails before service stop',()=>{
      const dir=fixture('no-space'); status(run(dir,`df() { echo 'fixture 100 99 1 99% /usrdata'; }; main`),1);
      assert(!fs.existsSync(path.join(dir,'trace')));
    });
    check('copy failure preserves old web and restores root read-only',()=>{
      const dir=fixture('copy-failed'); fs.mkdirSync(path.join(dir,'installed/www'));
      fs.writeFileSync(path.join(dir,'installed/www/index.html'),'old-web');
      const result=run(dir,'cp() { case "${*: -1}" in */www.new) return 1;; esac; command cp "$@"; }; main');
      status(result,1);
      assert.equal(fs.readFileSync(path.join(dir,'installed/www/index.html'),'utf8'),'old-web');
      assert.equal(fs.readFileSync(path.join(dir,'trace'),'utf8'),'-o remount,rw /\n-o remount,ro /\n');
      assert.match(fs.readFileSync(path.join(dir,'result'),'utf8'),/INSTALL_STATUS=FAIL/);
    });
    check('non-root ADB fails preflight',()=>{
      const dir=fixture('non-root'); status(run(dir,'id() { echo 2000; }; main'),1);
      assert(!fs.existsSync(path.join(dir,'trace')));
    });
    check('systemd and fallback share firewall setup; insertion failure is not swallowed',()=>{
      const dir=fixture('firewall'); status(run(dir,'install_fallback_scripts'),0);
      const definitions=portDefinitions(dir);
      for(const ok of [true,false]) {
        const result=spawnSync('sh',['-c',definitions+`\niptables() { echo "$*"; case "$1" in -C) return 1;; -I) return ${ok?0:1};; esac; }; open_web_port`],{encoding:'utf8'});
        status(result,ok?0:1);
        assert.match(result.stdout,/-I INPUT 1 -p tcp --dport 80 -j ACCEPT/);
      }
    });
    check('occupied socket cannot be mistaken for free when PID lookup fails',()=>{
      const dir=fixture('occupied'); status(run(dir,'install_fallback_scripts'),0);
      const definitions=portDefinitions(dir);
      status(spawnSync('sh',['-c',definitions+'\nweb_listener_inodes() { echo 123; }; web_owner_pids() { :; }; stop_known_web_conflicts() { :; }; sleep() { :; }; wait_for_web_port_free'],{encoding:'utf8'}),1);
    });
    check('custom port preserves existing cellular HTTP blocking ahead of ACCEPT',()=>{
      const dir=fixture('cellular-firewall'); status(run(dir,'install_fallback_scripts'),0);
      fs.writeFileSync(path.join(dir,'installed/http_port'),'8080\n');
      fs.mkdirSync(path.join(dir,'rmnet_data0'));
      const definitions=portDefinitions(dir).replace('/sys/class/net/rmnet*',dir+'/rmnet*');
      const trace=path.join(dir,'firewall-trace');
      for(const blocked of [true,false]) {
        fs.writeFileSync(trace,'');
        const result=spawnSync('sh',['-c',definitions+`\niptables() {
          echo "$*" >> "${trace}"
          case "$*" in
            '-C INPUT -i rmnet_data0 -p tcp --dport 80 -j DROP') return ${blocked?0:1} ;;
            -C*) return 1 ;;
          esac
        }; open_web_port`],{encoding:'utf8'});
        status(result,0);
        const calls=fs.readFileSync(trace,'utf8');
        assert.equal(calls.includes('-I INPUT 1 -i rmnet_data0 -p tcp --dport 8080 -j DROP'),blocked);
        if(blocked) assert(calls.indexOf('-j ACCEPT')<calls.lastIndexOf('-I INPUT 1 -i rmnet_data0'),'cellular DROP must end up before general ACCEPT');
      }
    });
    check('process cleanup rejects stale PIDs, adbd and command substring matches',()=>{
      const dir=fixture('process-identity');
      const proc=path.join(dir,'proc'); fs.mkdirSync(proc);
      const entries=[['11','/sbin/adbd','adbd'],['12','/bin/sh','sh -c cat /dev/ttyIN'],['13','/usr/bin/awk','awk cat /dev/ttyIN'],['14','/usrdata/simpleadmin/simpleadmin-httpd','simpleadmin-httpd']];
      for(const [pid,exe,args] of entries){fs.mkdirSync(path.join(proc,pid));fs.symlinkSync(exe,path.join(proc,pid,'exe'));fs.writeFileSync(path.join(proc,pid,'cmdline'),args.replaceAll(' ','\0'));}
      const helper=fs.readFileSync(path.join(root,'development/simpleadmin/runtime_processes.sh'),'utf8').replaceAll('/proc/',proc+'/');
      for(const pid of ['0','1','-1','11','12','13','9999']) {
        status(spawnSync('sh',['-c',helper+'\nruntime_kind '+pid],{encoding:'utf8'}),1);
      }
      const result=spawnSync('sh',['-c',helper+'\nruntime_kind 14'],{encoding:'utf8'});status(result,0);assert.equal(result.stdout.trim(),'server');
    });
    for(const mode of ['healthy','unhealthy','no-autostart']) {
      check(`service readiness ${mode}`,()=>{
        const dir=fixture('service-'+mode);
        const result=run(dir,`
eval "$ORIGINAL_RESTART"
SERVICE_UNIT_INSTALLED=1
systemctl() { echo "systemctl $*" >> "$TRACE"; }
remove_systemd_unit_files() { echo REMOVE >> "$TRACE"; }
start_fallback_service() { echo FALLBACK >> "$TRACE"; }
wait_for_web() { return ${mode==='healthy'?0:1}; }
${mode==='no-autostart'?'POST_BOOT_FILE="$2/missing-post-boot"':''}
restart_services
`);
        status(result,mode==='healthy'?0:1);
        const trace=fs.readFileSync(path.join(dir,'trace'),'utf8');
        assert.equal(trace.includes('FALLBACK'),mode==='unhealthy');
      });
    }
    // Serve only synthetic public files. Never start a modem reader or use ADB.
    let mode='ok';
    server=http.createServer((req,res)=>{
      if(mode==='timeout') return;
      const pages={'/':'SimpleAdminSpaMode','/login.html':'loginLanguage','/js/locales.js':'root.Lang'};
      res.statusCode=mode==='404'||(mode==='missing-js'&&req.url.endsWith('.js'))?404:200;
      if (req.url==='/'&&['ok','missing-js','wrong-redirect'].includes(mode)) {
        res.statusCode=303; res.setHeader('Location',mode==='wrong-redirect'?'/factory-login':'/login.html');
      }
      res.end(mode==='wrong-app'?'factory web':pages[req.url]||'missing');
    });
    await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve);});
    check('occupied custom port preserves the running installation',()=>{
      const dir=fixture('occupied-custom');
      const result=run(dir,`SIMPLEADMIN_HTTP_PORT=${server.address().port}; main`);
      status(result,1);
      assert.match(result.stderr,/已被占用/);
      assert(!fs.existsSync(path.join(dir,'trace')));
    });
    check('startup preparation reads custom port for firewall and live sockets',()=>{
      const dir=fixture('custom-firewall'); status(run(dir,'install_fallback_scripts'),0);
      fs.writeFileSync(path.join(dir,'installed/http_port'),String(server.address().port)+'\n');
      const result=spawnSync('sh',['-c',portDefinitions(dir)+'\niptables() { echo "$*"; [ "$1" != -C ]; }; open_web_port; web_listener_inodes'],{encoding:'utf8'});
      status(result,0);
      assert(result.stdout.includes(`--dport ${server.address().port}`));
      assert.match(result.stdout,/\n[0-9]+\n/,'custom port socket must be detected');
    });
    for(const item of ['ok','404','wrong-app','missing-js','wrong-redirect','timeout']) {
      mode=item;
      const result=await new Promise(resolve=>{
        const child=spawn('bash',[path.join(root,'development/simpleadmin/check_web.sh')],{timeout:12000,env:{...process.env,SIMPLEADMIN_CHECK_PORT:String(server.address().port)}});
        let output=''; child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>output+=b);
        child.on('exit',(code)=>resolve({status:code,stdout:output,stderr:''}));
      });
      status(result,item==='ok'?0:1); passed++;console.log(`PASS HTTP ${item}`);
    }
    console.log(`${passed} installer checks passed`);
  } finally {
    if(server) {server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
    fs.rmSync(scratch,{recursive:true,force:true});
  }
})().catch(error=>{console.error(error);process.exitCode=1;});
