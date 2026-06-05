#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');

const root = path.resolve(__dirname, '..');
const platform = process.platform;
const arch = process.arch;
const exe = platform === 'win32' ? '.exe' : '';
const candidates = [
  path.join(root, 'bin', `dexuse-${platform}-${arch}${exe}`),
  path.join(root, 'target', 'release', `dexuse${exe}`),
  path.join(root, 'target', 'debug', `dexuse${exe}`),
];

let bin = candidates.find((p) => fs.existsSync(p));
let result;
if (bin) {
  result = spawnSync(bin, process.argv.slice(2), { stdio: 'inherit' });
} else if (fs.existsSync(path.join(root, 'Cargo.toml'))) {
  // Development fallback: lets `npm exec -- dexuse` and local `npx .` work before
  // release artifacts are published. Published packages should ship bin/* assets.
  result = spawnSync('cargo', ['run', '--quiet', '--', ...process.argv.slice(2)], {
    cwd: root,
    stdio: 'inherit',
    shell: platform === 'win32',
  });
} else {
  console.error(`dexuse: no binary for ${platform}-${arch}. Please install a package that ships this target.`);
  process.exit(1);
}

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 0);
