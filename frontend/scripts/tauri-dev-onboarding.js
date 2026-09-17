const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');

const frontendDir = path.resolve(__dirname, '..');
const dataDir = path.join(frontendDir, '.dev-onboarding');
fs.mkdirSync(dataDir, { recursive: true });
fs.rmSync(path.join(dataDir, 'onboarding-status.json'), { force: true });

console.log(`Onboarding test profile: ${dataDir}`);
const child = spawn(
  process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm',
  ['run', 'tauri:dev:cpu'],
  {
    cwd: frontendDir,
    env: { ...process.env, MEETILY_DEV_DATA_DIR: dataDir },
    stdio: 'inherit',
    shell: process.platform === 'win32',
  },
);

child.on('error', (error) => {
  console.error(error);
  process.exitCode = 1;
});
child.on('exit', (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exitCode = code ?? 1;
});
