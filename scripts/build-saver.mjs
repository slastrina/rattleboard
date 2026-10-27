#!/usr/bin/env node
// Builds the screen saver that ships inside the app, into target/saver/ (Tauri runs this as
// beforeBundleCommand; see tauri.macos.conf.json and tauri.windows.conf.json):
//
//   macOS    target/saver/Rattleboard.saver   (Swift, arm64 + x86_64, macOS 11+)
//   Windows  target/saver/Rattleboard.scr     (crates/rattleboard-saver)
//   Linux    nothing
//
// Both carry the screen saver page (src/saver.html and saver.js, the split-flap renderer and
// the font). On macOS, RB_SAVER_IDENTITY (a Developer ID Application identity, its name or
// SHA-1) signs the bundle for notarization, with the hardened runtime and a timestamp;
// without it the bundle is signed ad hoc, which is enough to try it on this Mac.
//
//   node scripts/build-saver.mjs            build for this platform
//   node scripts/build-saver.mjs --web-only only assemble target/saver/web (for trying the page)
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync, copyFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(root, 'target', 'saver');
const version = JSON.parse(readFileSync(join(root, 'src-tauri', 'tauri.conf.json'), 'utf8')).version;
const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: 'inherit', cwd: root, ...opts });

/** The page and what it needs, laid out as src/ has them. */
function web(dir) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(join(dir, 'shared'), { recursive: true });
  mkdirSync(join(dir, 'fonts'), { recursive: true });
  for (const f of ['saver.html', 'saver.js']) copyFileSync(join(root, 'src', f), join(dir, f));
  for (const f of ['split-flap.js', 'base.css']) copyFileSync(join(root, 'src', 'shared', f), join(dir, 'shared', f));
  for (const w of [500, 600]) copyFileSync(join(root, 'src', 'fonts', `barlow-semi-condensed-${w}.woff2`), join(dir, 'fonts', `barlow-semi-condensed-${w}.woff2`));
}

function mac() {
  const bundle = join(out, 'Rattleboard.saver'), contents = join(bundle, 'Contents');
  rmSync(bundle, { recursive: true, force: true });
  mkdirSync(join(contents, 'MacOS'), { recursive: true });
  mkdirSync(join(contents, 'Resources'), { recursive: true });
  const src = join(root, 'saver', 'macos', 'RattleboardSaver.swift');
  const slices = [];
  for (const arch of ['arm64', 'x86_64']) {
    const o = join(out, `saver-${arch}`);
    run('xcrun', ['swiftc', '-O', '-target', `${arch}-apple-macos11.0`, '-module-name', 'RattleboardSaver',
      '-emit-library', '-Xlinker', '-bundle', '-framework', 'ScreenSaver', '-framework', 'WebKit', '-framework', 'AppKit',
      '-o', o, src]);
    slices.push(o);
  }
  run('lipo', ['-create', ...slices, '-output', join(contents, 'MacOS', 'Rattleboard')]);
  slices.forEach(s => rmSync(s, { force: true }));
  writeFileSync(join(contents, 'Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>Rattleboard</string>
  <key>CFBundleExecutable</key><string>Rattleboard</string>
  <key>CFBundleIdentifier</key><string>io.github.slastrina.rattleboard.saver</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Rattleboard</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSPrincipalClass</key><string>RattleboardSaverView</string>
  <key>NSHumanReadableCopyright</key><string>MIT License</string>
</dict>
</plist>
`);
  web(join(contents, 'Resources', 'web'));
  for (const t of ['thumbnail.png', 'thumbnail@2x.png']) {
    const p = join(root, 'saver', 'macos', t);
    if (existsSync(p)) copyFileSync(p, join(contents, 'Resources', t));
  }
  const id = process.env.RB_SAVER_IDENTITY;
  if (id) run('codesign', ['--force', '--options', 'runtime', '--timestamp', '--sign', id, bundle]);
  else run('codesign', ['--force', '--sign', '-', bundle]);
  console.log(`Screen saver: ${bundle}${id ? ' (signed)' : ' (ad hoc)'}`);
}

function windows() {
  run('cargo', ['build', '-p', 'rattleboard-saver', '--release']);
  const exe = join(root, 'target', 'release', 'rattleboard-saver.exe');
  copyFileSync(exe, join(out, 'Rattleboard.scr'));
  console.log(`Screen saver: ${join(out, 'Rattleboard.scr')}`);
}

mkdirSync(out, { recursive: true });
// CI builds and signs it in a step of its own (with the Developer ID keychain), then bundles.
const prebuilt = process.env.RB_SAVER_PREBUILT === '1'
  && existsSync(join(out, process.platform === 'darwin' ? 'Rattleboard.saver' : 'Rattleboard.scr'));
if (prebuilt) console.log('Screen saver: prebuilt');
else if (process.argv.includes('--web-only')) web(join(out, 'web'));
else if (process.platform === 'darwin') mac();
else if (process.platform === 'win32') windows();
else console.log('No screen saver on this platform.');
