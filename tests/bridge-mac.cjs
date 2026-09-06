const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {spawnSync} = require('node:child_process');
const source = fs.readFileSync(path.join(__dirname, '../development/simpleadmin/mobileap_bridge0_mac.sh'), 'utf8');
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'bridge-mac-'));
let count = 0;
try {
  for (const mode of ['apmac', 'earlyeth', 'same', 'invalid', 'multicast', 'missing', 'unsupported', 'disabled', 'backup-failed']) {
    const dir = path.join(root, mode); fs.mkdirSync(dir);
    const mac = '8C:12:34:56:78:9A';
    const original = mode === 'earlyeth' ? '<EarlyEthMode>0</EarlyEthMode><EarlyEthMACAddr>00:00:00:00:00:00</EarlyEthMACAddr>' : mode === 'unsupported' ? '<Unknown>unchanged</Unknown>' : `<APMACAddress>${mode === 'same' ? mac : '02:00:00:00:00:01'}</APMACAddress>`;
    fs.writeFileSync(path.join(dir, 'cfg'), original, {mode: 0o640});
    fs.writeFileSync(path.join(dir, 'address'), mode === 'invalid' ? '00:00:00:00:00:00' : mode === 'multicast' ? 'FF:12:34:56:78:9A' : mac.toLowerCase());
    if (mode === 'missing') fs.unlinkSync(path.join(dir, 'address'));
    fs.writeFileSync(path.join(dir, 'mounts'), 'rootfs / ubifs ro,relatime 0 0\n');
    fs.mkdirSync(path.join(dir, 'bin'));
    for (const cmd of ['mount', 'sync']) {
      fs.writeFileSync(path.join(dir, 'bin', cmd), `#!/bin/sh\necho '${cmd}' "$@" >> '${dir}/trace'\n`, {mode: 0o755});
    }
    if (mode === 'backup-failed') fs.writeFileSync(path.join(dir, 'bin/cp'), '#!/bin/sh\nexit 1\n', {mode: 0o755});
    const script = source.replace('MOBILEAP_CFG_PRIMARY="/usrdata/etc/data/mobileap_cfg.xml"', `MOBILEAP_CFG_PRIMARY="${dir}/cfg"`).replace('BRIDGE0_ADDRESS_FILE="/sys/class/net/bridge0/address"', `BRIDGE0_ADDRESS_FILE="${dir}/address"`).replace('/proc/mounts', `${dir}/mounts`);
    fs.writeFileSync(path.join(dir, 'helper'), script);
    const env = {...process.env, PATH: `${dir}/bin:${process.env.PATH}`, SIMPLEADMIN_DIR: dir, MOBILEAP_RESULT_FILE: `${dir}/result`, SIMPLEADMIN_FIX_BRIDGE0_MAC: mode === 'disabled' ? '0' : '1'};
    const run = () => spawnSync('bash', [path.join(dir, 'helper')], {env, encoding: 'utf8'});
    const result = run(); assert.equal(result.status, mode === 'backup-failed' ? 1 : 0, result.stderr);
    const trace = fs.existsSync(`${dir}/trace`) ? fs.readFileSync(`${dir}/trace`, 'utf8') : '';
    if (['apmac', 'earlyeth'].includes(mode)) {
      assert(fs.readFileSync(`${dir}/cfg`, 'utf8').includes(mac));
      assert.equal(fs.readFileSync(`${dir}/cfg.simpleadmin.bak`, 'utf8'), original);
      assert.equal(fs.statSync(`${dir}/cfg`).mode & 0o777, 0o640);
      assert.match(trace, /^mount -o remount,rw \/\n/); assert.match(trace, /mount -o remount,ro \/\n$/);
      const before = fs.statSync(`${dir}/cfg`).mtimeMs; fs.writeFileSync(`${dir}/trace`, '');
      assert.equal(run().status, 0); assert.equal(fs.statSync(`${dir}/cfg`).mtimeMs, before); assert.equal(fs.readFileSync(`${dir}/trace`, 'utf8'), '');
    } else {
      assert.equal(fs.readFileSync(`${dir}/cfg`, 'utf8'), original);
      if (mode === 'backup-failed') assert.match(trace, /mount -o remount,ro \/\n$/);
      else assert.equal(trace, '');
    }
    assert(!fs.existsSync(`${dir}/bridge0_mac`));
    console.log(`PASS bridge MAC: ${mode}`); count++;
  }
  assert(!/ip link|ifconfig|systemctl|\/dev\/urandom/.test(source));
  console.log(`${count} bridge MAC checks passed`);
} finally { fs.rmSync(root, {recursive: true, force: true}); }
