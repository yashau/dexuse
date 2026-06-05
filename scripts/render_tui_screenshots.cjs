#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node-pty');
const { Terminal } = require('@xterm/headless');
const { SerializeAddon } = require('@xterm/addon-serialize');
const { chromium } = require('playwright');

const ROOT = path.resolve(__dirname, '..');
const OUT = path.join(ROOT, 'screenshots');
const COLS = 120;
const ROWS = 36;
const FONT_SIZE = 15;
const LINE_HEIGHT = 1.24;
// The PNG should be the terminal, not a decorative browser/window that is
// slightly larger than the real TUI. The browser viewport only needs to be
// large enough to render the terminal element; Playwright screenshots that
// element directly below.
const VIEWPORT = { width: 1400, height: 900 };
const FIXTURE_ROOT = path.join(ROOT, 'fixtures', 'usage');
const FIXTURE_ARGS = [
  '--codex-home', path.join(FIXTURE_ROOT, 'codex'),
  '--hermes-home', path.join(FIXTURE_ROOT, 'hermes'),
  '--openclaw-home', path.join(FIXTURE_ROOT, 'openclaw'),
  '--from', '2026-06-01',
  '--to', '2026-06-06',
  '--granularity', 'day'
];
fs.mkdirSync(OUT, { recursive: true });

const scriptTimeout = setTimeout(() => {
  console.error('screenshot capture timed out after 120s');
  process.exit(124);
}, 120_000);

function platformName() {
  if (process.platform === 'win32') return 'win32';
  if (process.platform === 'darwin') return 'darwin';
  if (process.platform === 'linux') return 'linux';
  return process.platform;
}

function binaryName() {
  const ext = process.platform === 'win32' ? '.exe' : '';
  return `dexuse-${platformName()}-${process.arch}${ext}`;
}

function resolveCommand() {
  const binary = path.join(ROOT, 'bin', binaryName());
  if (fs.existsSync(binary)) return { command: binary, args: [] };
  if (fs.existsSync(path.join(ROOT, 'Cargo.toml'))) {
    return { command: 'cargo', args: ['run', '--quiet', '--'] };
  }
  throw new Error(`No packaged binary found at ${binary} and no Cargo.toml fallback exists`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function onceWriteParsed(term) {
  return new Promise((resolve) => {
    const disposable = term.onWriteParsed(() => {
      disposable.dispose();
      resolve();
    });
  });
}

async function writeAndFlush(term, data) {
  term.write(data);
  await Promise.race([onceWriteParsed(term), sleep(40)]);
}

function waitForExit(pty, timeoutMs = 1500) {
  return new Promise((resolve) => {
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      try {
        pty.kill();
      } catch {
        // ignore cleanup errors from already-exited PTYs
      }
      resolve();
    }, timeoutMs);
    pty.onExit(() => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve();
    });
  });
}

function extractPreBody(serializedHtml) {
  const match = serializedHtml.match(/<pre[^>]*>([\s\S]*?)<\/pre>/i);
  if (!match) return serializedHtml;
  return match[1];
}

function wrapHtml(title, serializedHtml) {
  const preInner = extractPreBody(serializedHtml);
  return `<!doctype html>
<html>
<head>
<meta charset="utf-8" />
<title>${title}</title>
<style>
:root { color-scheme: dark; }
* { box-sizing: border-box; }
body {
  margin: 0;
  min-width: ${VIEWPORT.width}px;
  min-height: ${VIEWPORT.height}px;
  overflow: hidden;
  background: #05070d;
  color: #d7e2ff;
  font-family: ui-monospace, "Cascadia Mono", "SFMono-Regular", Menlo, Consolas, "Liberation Mono", monospace;
}
.terminal {
  display: inline-block;
  font-size: ${FONT_SIZE}px;
  line-height: ${LINE_HEIGHT};
  width: ${COLS}ch;
  height: ${Math.ceil(ROWS * FONT_SIZE * LINE_HEIGHT)}px;
  background: #05070d;
  overflow: hidden;
}
.screen {
  background: #05070d;
  width: 100%;
  height: 100%;
  overflow: hidden;
}
.screen pre,
.screen pre div {
  margin: 0 !important;
  background: #05070d !important;
  color: #d7e2ff;
  font-family: ui-monospace, "Cascadia Mono", "SFMono-Regular", Menlo, Consolas, "Liberation Mono", monospace !important;
  font-size: ${FONT_SIZE}px !important;
  line-height: ${LINE_HEIGHT} !important;
}
</style>
</head>
<body>
<div class="terminal" aria-label="${title}"><div class="screen"><pre>${preInner}</pre></div></div>
</body>
</html>`;
}

async function capture(name, title, keys = []) {
  const term = new Terminal({
    cols: COLS,
    rows: ROWS,
    allowProposedApi: true,
    convertEol: false,
    scrollback: 0,
    theme: {
      background: '#05070d',
      foreground: '#d7e2ff'
    }
  });
  const serialize = new SerializeAddon();
  term.loadAddon(serialize);

  const { command, args } = resolveCommand();
  const pty = spawn(command, [...args, ...FIXTURE_ARGS], {
    name: 'xterm-256color',
    cols: COLS,
    rows: ROWS,
    cwd: ROOT,
    env: {
      ...process.env,
      TERM: 'xterm-256color',
      COLORTERM: 'truecolor',
      RUST_BACKTRACE: '1'
    }
  });

  const onData = pty.onData((data) => term.write(data));
  await sleep(1600);
  for (const key of keys) {
    pty.write(key);
    await sleep(500);
  }
  await sleep(700);

  const html = wrapHtml(title, serialize.serializeAsHTML());
  const htmlPath = path.join(OUT, `${name}.html`);
  const pngPath = path.join(OUT, `${name}.png`);
  fs.writeFileSync(htmlPath, html, 'utf8');

  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: VIEWPORT, deviceScaleFactor: 1 });
    await page.goto(`file://${htmlPath.replace(/\\/g, '/')}`);
    await page.locator('.terminal').screenshot({ path: pngPath });
  } finally {
    await browser.close();
  }

  try {
    pty.write('q');
  } catch {
    // ignore if the PTY has already exited
  }
  await waitForExit(pty);
  onData.dispose();
  term.dispose();
  console.log(pngPath);
}

(async () => {
  await capture('dexuse-tui-timeline', 'dexuse — explore: day timeline + table', []);
  await capture('dexuse-tui-drilldown', 'dexuse — explore: year → month → week → day drilldown', ['y', '\r', '\r', '\r']);
  await capture('dexuse-tui-models', 'dexuse — explore: models chart + table', ['2']);
  await capture('dexuse-tui-sources', 'dexuse — explore: sources chart + table', ['3']);
  clearTimeout(scriptTimeout);
  process.exit(0);
})().catch((error) => {
  clearTimeout(scriptTimeout);
  console.error(error);
  process.exit(1);
});
