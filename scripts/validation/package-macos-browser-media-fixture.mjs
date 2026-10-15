#!/usr/bin/env node
// Package the independently compiled native example with real App privacy/ATS
// metadata, an isolated identity, and a fresh directory. Never grants media.
import { cp, mkdir, stat, writeFile } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { ensureMacosDevelopmentBundle, macosDevelopmentApplicationArguments, runCommand } from '../run-macos-dev-runner.mjs';
import { signMacosAppBundle } from '../lib/macos-app-bundle.mjs';
import { launchMacosApplication } from '../lib/macos-application-launch.mjs';

async function digestFile(path) {
  const digest = createHash('sha256');
  for await (const bytes of createReadStream(path)) digest.update(bytes);
  return digest.digest('hex');
}

const AUDITS = {
  media: { productName: 'NomiFun WK Media Audit', identifier: 'com.nomifun.desktop.audit.media', scope: 'browser_media_audit' },
  compatibility: { productName: 'NomiFun WK Compatibility Audit', identifier: 'com.nomifun.desktop.audit.compatibility', scope: 'browser_compatibility_audit' },
  appkit: { productName: 'NomiFun AppKit Main Loop Audit', identifier: 'com.nomifun.desktop.audit.mainloop', scope: 'appkit_main_loop_audit' },
};

export async function packageMacosNativeFixture({ binary, outputRoot, kind = 'media', launch = false, controllerProgram, userReady = false, applicationArgs = [], environment: suppliedEnvironment = process.env },
  { ensureBundle = ensureMacosDevelopmentBundle, signBundle = signMacosAppBundle, run = runCommand, launchApplication = launchMacosApplication } = {}) {
  if (process.platform !== 'darwin' || !isAbsolute(binary ?? '') || !isAbsolute(outputRoot ?? '')) throw new Error('macOS and absolute --binary /CURRENT/EXAMPLE --output-dir /NEW/ROOT required');
  const audit = AUDITS[kind];
  if (!audit || !Array.isArray(applicationArgs) || applicationArgs.some(arg => typeof arg !== 'string' || arg.includes('\0'))) throw new Error('A fixed media, compatibility or appkit audit is required');
  if (kind !== 'media' && (userReady || applicationArgs.some(arg => ['--media-permissions', '--media-user-ready'].includes(arg)))) throw new Error('Media flags require the fixed media audit');
  if (kind === 'media' && applicationArgs.length) throw new Error('The media audit retains its fixed permission entry');
  binary = resolve(binary);
  outputRoot = resolve(outputRoot);
  if (!(await stat(binary)).isFile()) throw new Error('Compiled fixture binary is not a file');
  if (launch && (!isAbsolute(controllerProgram ?? '') || !(await stat(controllerProgram)).isFile())) {
    throw new Error('--launch requires an absolute compiled --controller macos_application_controller');
  }
  if (userReady && !launch) throw new Error('--media-user-ready requires --launch through the compiled controller');
  await mkdir(outputRoot); // An existing directory is rejected; never replace an App.
  const dataDir = join(outputRoot, 'data');
  await mkdir(dataDir);
  const evidenceDir = join(outputRoot, 'evidence');
  await mkdir(evidenceDir, { mode: 0o700 });
  const { productName, identifier } = audit;
  const identity = { productName, identifier };
  const args = kind === 'media' ? ['--media-permissions', ...(userReady ? ['--media-user-ready'] : [])]
    : kind === 'appkit' ? ['--report', join(evidenceDir, 'main-loop-report.json'), ...applicationArgs]
    : ['--evidence-dir', evidenceDir, ...applicationArgs];
  const environment = { ...suppliedEnvironment, NOMIFUN_DATA_DIR: dataDir, TAURI_CONFIG: JSON.stringify(identity) };
  const sourceSha256 = await digestFile(binary);
  let preSigningCopySha256;
  const cachedProgram = await ensureBundle({ hostPath: binary, environment }, {
    sign: async request => {
      // Prove the exact supplied bytes BEFORE codesign legitimately changes
      // the Mach-O signature. Never compare signed bytes to unsigned supply.
      const copied = join(request.appPath, 'Contents', 'MacOS', 'nomifun-desktop');
      const copiedSha256 = await digestFile(copied);
      if (copiedSha256 !== sourceSha256 || await digestFile(binary) !== sourceSha256) throw new Error('Pre-signing fixture copy differs from the exact supplied binary');
      preSigningCopySha256 = copiedSha256;
      await signBundle(request);
    },
  });
  // The new output root supplies an independent cache identity. If an old
  // cache is nevertheless reused, refuse a claim without its supply witness.
  if (!preSigningCopySha256) throw new Error('Fresh fixture signing did not provide a pre-signing supply witness');
  if (await digestFile(binary) !== sourceSha256) throw new Error('Supplied fixture changed during packaging');
  const codesignedSha256 = await digestFile(cachedProgram);
  const cachedApp = dirname(dirname(dirname(cachedProgram)));
  const app = join(outputRoot, `${identity.productName}.app`);
  await cp(cachedApp, app, { recursive: true, dereference: false, errorOnExist: true, force: false });
  const program = join(app, 'Contents', 'MacOS', 'nomifun-desktop');
  const packagedSha256 = await digestFile(program);
  if (codesignedSha256 !== packagedSha256) throw new Error('Final App copy differs from the verified codesigned fixture');
  await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', app]);
  const report = { app, program, data_dir: dataDir, ...identity, application_scope: audit.scope, application_args: args, fixture_binary_sha256: sourceSha256,
    pre_signing_copy_sha256: preSigningCopySha256, codesigned_binary_sha256: codesignedSha256, packaged_binary_sha256: packagedSha256,
    supply_verified_before_signing: true, package_matches_codesigned_cache: true, signature_verification: 'codesign_verify_deep_strict',
    system_webkit: true, grants_media: false, actual_sensor_capture: false, packaging_only: !launch };
  await writeFile(join(outputRoot, 'package-report.json'), `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
  if (launch) {
    const application = await launchApplication({ program, controllerProgram: resolve(controllerProgram),
      args: macosDevelopmentApplicationArguments(args),
      environment, cwd: dirname(binary), applicationScope: audit.scope });
    if (application.evidenceDirectory) console.error(`[audit] native application logs: ${application.evidenceDirectory}`);
    const exited = await application.exit;
    if (exited.code !== 0 || exited.signal || !exited.cleanupVerified) throw new Error('Media audit application cleanup and actual normal exit were not verified');
    report.launch_transport = 'public_nsworkspace';
    report.application_exit = { pid: exited.pid, code: exited.code, signal: exited.signal, cleanup_verified: true };
    await writeFile(join(outputRoot, 'package-report.json'), `${JSON.stringify(report, null, 2)}\n`);
  }
  return report;
}

export async function packageMacosBrowserMediaFixture(options, hooks) {
  return packageMacosNativeFixture({ ...options, kind: 'media' }, hooks);
}

export async function main(args = process.argv.slice(2)) {
  const option = name => { const index = args.indexOf(name); return index < 0 ? undefined : args[index + 1]; };
  if (args.includes('--help')) {
    console.log('Use --binary /ABS/current/macos_browser_compatibility_smoke --output-dir /ABS/NEW/ROOT [--launch --controller /ABS/current/macos_application_controller] [--media-user-ready]. Packages NomiFun WK Media Audit.app with com.nomifun.desktop.audit.media, a fresh data marker and current repository usage/ATS metadata. Supply bytes are checked before signing; signed copy and codesign are checked afterwards. --launch uses public LaunchServices for the fixed --media-permissions entry, all Deny and no Grant; success requires authenticated cleanup and the actual application kernel exit. --media-user-ready adds the trusted Start gate: each visible focused native page must receive a trusted Start click within 60 seconds. No Cargo or credentials are invoked.');
    return;
  }
  console.log(JSON.stringify(await packageMacosBrowserMediaFixture({ binary: option('--binary'), outputRoot: option('--output-dir'), launch: args.includes('--launch'), controllerProgram: option('--controller'), userReady: args.includes('--media-user-ready') })));
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
