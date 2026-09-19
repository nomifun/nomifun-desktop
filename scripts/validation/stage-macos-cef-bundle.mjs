#!/usr/bin/env node
// Stage the pinned CEF runtime into an already-built NomiFun.app. This tool
// never discovers an arbitrary app/helper through PATH and never signs with a
// credential value; callers provide an identity name/hash or '-' for ad-hoc.

import { spawn } from 'node:child_process';
import {
  cp,
  mkdir,
  mkdtemp,
  open,
  readFile,
  readdir,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';

const args = process.argv.slice(2);
const option = key => {
  const index = args.indexOf(key);
  if (index < 0 || !args[index + 1] || args[index + 1].startsWith('--')) {
    throw new Error(`${key} requires a value`);
  }
  return args[index + 1];
};
const app = resolve(option('--app'));
const helper = resolve(option('--helper'));
const runtime = resolve(option('--runtime'));
const identity = args.includes('--identity') ? option('--identity') : '-';
const contents = join(app, 'Contents');
const frameworks = join(contents, 'Frameworks');
const frameworkSource = join(runtime, 'Chromium Embedded Framework.framework');
const framework = join(frameworks, 'Chromium Embedded Framework.framework');
const expectedArchive = 'cef_binary_152.0.6+g708dc14+chromium-152.0.7977.83_macosarm64_minimal.tar.bz2';
const helperNames = ['', ' (GPU)', ' (Renderer)', ' (Plugin)', ' (Alerts)']
  .map(suffix => `NomiFun Helper${suffix}`);

const run = (command, argv, capture = false) => new Promise((accept, reject) => {
  const child = spawn(command, argv, {
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
    env: Object.fromEntries(['PATH', 'HOME', 'TMPDIR', 'LANG', 'DEVELOPER_DIR']
      .filter(key => process.env[key])
      .map(key => [key, process.env[key]])),
  });
  let output = '';
  if (capture) {
    child.stdout.on('data', chunk => { output += chunk; });
    child.stderr.on('data', chunk => { output += chunk; });
  }
  child.once('error', reject);
  child.once('exit', code => code === 0
    ? accept(output.trim())
    : reject(new Error(`${command} exited ${code}${capture ? `: ${output}` : ''}`)));
});

const plist = object => `<?xml version="1.0" encoding="UTF-8"?>\n` +
  `<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n` +
  `<plist version="1.0"><dict>${Object.entries(object).map(([key, value]) =>
    `<key>${key}</key>${typeof value === 'boolean' ? `<${value}/>` : `<string>${value}</string>`}`
  ).join('')}</dict></plist>\n`;

async function requireFile(path, label) {
  const descriptor = await open(path, 'r').catch(() => null);
  if (!descriptor) throw new Error(`${label} is missing: ${path}`);
  await descriptor.close();
}

async function signMachO(directory, sign) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) await signMachO(path, sign);
    else if (entry.isFile()) {
      const descriptor = await open(path, 'r');
      const bytes = Buffer.alloc(4);
      try { await descriptor.read(bytes, 0, 4, 0); } finally { await descriptor.close(); }
      if (['cffaedfe', 'cefaedfe', 'cafebabe'].includes(bytes.toString('hex'))) await sign(path);
    }
  }
}

let temporary;
try {
  if (process.platform !== 'darwin' || process.arch !== 'arm64') {
    throw new Error('CEF product staging requires an Apple Silicon macOS host');
  }
  await requireFile(join(contents, 'MacOS', 'nomifun-desktop'), 'NomiFun host');
  await requireFile(helper, 'CEF helper');
  await requireFile(join(frameworkSource, 'Chromium Embedded Framework'), 'CEF framework');
  const archive = JSON.parse(await readFile(join(runtime, 'archive.json'), 'utf8'));
  if (archive.name !== expectedArchive) {
    throw new Error(`unexpected CEF archive: ${archive.name || 'unknown'}`);
  }
  const helperArch = await run('lipo', ['-archs', helper], true);
  const frameworkArch = await run(
    'lipo', ['-archs', join(frameworkSource, 'Chromium Embedded Framework')], true,
  );
  if (helperArch !== 'arm64' || frameworkArch !== 'arm64') {
    throw new Error(`CEF arm64 staging received helper=${helperArch}, framework=${frameworkArch}`);
  }

  await mkdir(frameworks, { recursive: true });
  await rm(framework, { recursive: true, force: true });
  await cp(frameworkSource, framework, {
    recursive: true,
    dereference: false,
    verbatimSymlinks: true,
  });

  const common = {
    CFBundlePackageType: 'APPL',
    CFBundleVersion: '1',
    CFBundleShortVersionString: '1.0',
    LSMinimumSystemVersion: '14.0',
    NSHighResolutionCapable: true,
    LSUIElement: true,
  };
  for (const name of helperNames) {
    const helperApp = join(frameworks, `${name}.app`);
    await rm(helperApp, { recursive: true, force: true });
    await mkdir(join(helperApp, 'Contents/MacOS'), { recursive: true });
    await cp(helper, join(helperApp, 'Contents/MacOS', name));
    const suffix = name.replace('NomiFun Helper', '').replace(/[^A-Za-z]/g, '').toLowerCase();
    await writeFile(join(helperApp, 'Contents/Info.plist'), plist({
      ...common,
      CFBundleIdentifier: `com.nomifun.desktop.helper${suffix ? `.${suffix}` : ''}`,
      CFBundleName: name,
      CFBundleExecutable: name,
    }));
  }

  const legal = join(contents, 'Resources', 'browser-cef');
  await mkdir(legal, { recursive: true });
  await cp(join(runtime, 'CREDITS.html'), join(legal, 'CREDITS.html'));
  await writeFile(join(legal, 'runtime.json'), `${JSON.stringify({
    cef: '152.0.6',
    chromium: '152.0.7977.83',
    crate: '152.3.0',
    archive: archive.name,
    archive_sha1: archive.sha1,
    architecture: 'arm64',
  }, null, 2)}\n`);

  temporary = await mkdtemp(join(tmpdir(), 'nomifun-cef-sign-'));
  const entitlements = join(temporary, 'helper-entitlements.plist');
  await writeFile(entitlements, plist({ 'com.apple.security.cs.allow-jit': true }));
  const sign = (path, jit = false) => run('codesign', [
    '--force',
    ...(identity === '-' ? [] : ['--options', 'runtime', '--timestamp']),
    '--sign', identity,
    ...(jit ? ['--entitlements', entitlements] : []),
    path,
  ]);
  await signMachO(framework, sign);
  await sign(framework);
  for (const name of helperNames) await sign(join(frameworks, `${name}.app`), true);
  await sign(app);
  await run('codesign', ['--verify', '--deep', '--strict', app]);

  process.stdout.write(`${JSON.stringify({
    status: 'pass',
    app,
    identity: identity === '-' ? 'adhoc' : 'provided',
    cef: '152.0.6',
    chromium: '152.0.7977.83',
    architecture: 'arm64',
    framework: `Contents/Frameworks/${basename(framework)}`,
    helpers: helperNames.map(name => `Contents/Frameworks/${name}.app`),
    credits: 'Contents/Resources/browser-cef/CREDITS.html',
  }, null, 2)}\n`);
} catch (error) {
  process.stderr.write(`MACOS_CEF_STAGE_FAIL ${error.message}\n`);
  process.exitCode = 1;
} finally {
  if (temporary) await rm(temporary, { recursive: true, force: true });
}
