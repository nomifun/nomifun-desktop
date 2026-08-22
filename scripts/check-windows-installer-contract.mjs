#!/usr/bin/env bun
/**
 * Static safety contract for the vendored Tauri NSIS installer template.
 *
 * Default mode checks the source template, locked CLI, bundle resources, and
 * data/install-root separation. Pass "--rendered <path>" after a Windows
 * package build to additionally verify Tauri's rendered installer.nsi without
 * executing the installer.
 */
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { dirname, isAbsolute, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const configPath = resolve(root, 'apps/desktop/tauri.conf.json');
const packagePath = resolve(root, 'package.json');
const lockPath = resolve(root, 'bun.lock');
const cliPackagePath = resolve(root, 'node_modules/@tauri-apps/cli/package.json');
const cliSchemaPath = resolve(root, 'node_modules/@tauri-apps/cli/config.schema.json');
const dataRootSourcePath = resolve(root, 'crates/backend/nomifun-app/src/cli.rs');
const noticePath = resolve(root, 'NOTICE');
const expectedCliVersion = '2.11.2';
const expectedUpstreamCommit = '499df79be65ef8c0670abc0207cd9e37b55d8491';
const expectedUpstreamSha256 = 'ee84148e405adc4d736a46456dd8345a644751bd1f28a335dd7fd833a32d7c3e';
const expectedCustomSha256 = '646077ad9b18482820aab29237248a2585fbc1944525ff96393ad8c316415311';
const errors = [];

function check(condition, message) {
  if (!condition) errors.push(message);
}

function checkIncludes(source, needle, message) {
  check(source.includes(needle), message || ('missing required text: ' + needle));
}

function count(source, needle) {
  return source.split(needle).length - 1;
}

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function parseJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

function renderedArgument(argv) {
  const index = argv.indexOf('--rendered');
  if (index < 0) {
    check(argv.length === 0, 'unknown arguments: ' + argv.join(' '));
    return null;
  }
  check(index === 0 && argv.length === 2, 'usage: check-windows-installer-contract.mjs [--rendered <installer.nsi>]');
  return argv[index + 1] ?? null;
}

const packageJson = parseJson(packagePath);
const lockText = readFileSync(lockPath, 'utf8');
const config = parseJson(configPath);
const nsis = config?.bundle?.windows?.nsis;

check(
  packageJson?.devDependencies?.['@tauri-apps/cli'] === expectedCliVersion,
  'package.json must exact-pin @tauri-apps/cli to ' + expectedCliVersion,
);
check(
  lockText.includes('"@tauri-apps/cli": "2.11.2"'),
  'bun.lock workspace spec must exact-pin @tauri-apps/cli to 2.11.2',
);
check(
  lockText.includes('"@tauri-apps/cli": ["@tauri-apps/cli@2.11.2"'),
  'bun.lock must resolve @tauri-apps/cli 2.11.2',
);
check(existsSync(cliPackagePath), 'installed @tauri-apps/cli package is missing; run bun install');
check(existsSync(cliSchemaPath), 'installed Tauri config schema is missing; run bun install');
if (existsSync(cliPackagePath)) {
  check(parseJson(cliPackagePath).version === expectedCliVersion, 'installed Tauri CLI version must be 2.11.2');
}
if (existsSync(cliSchemaPath)) {
  const nsisProperties = parseJson(cliSchemaPath)?.definitions?.NsisConfig?.properties ?? {};
  check('template' in nsisProperties, 'locked Tauri schema must support bundle.windows.nsis.template');
  check('installMode' in nsisProperties, 'locked Tauri schema must support bundle.windows.nsis.installMode');
  check('installerHooks' in nsisProperties, 'locked Tauri schema must retain the documented hook surface');
}
check(nsis?.installMode === 'currentUser', 'bundle.windows.nsis.installMode must be currentUser');
check(nsis?.template === 'nsis/installer.nsi', 'bundle.windows.nsis.template must be nsis/installer.nsi');

const resources = config?.bundle?.resources ?? {};
const requiredResources = {
  '../../ui/dist/': 'webui-dist/',
  '../../LICENSE': 'LICENSE',
  '../../NOTICE': 'NOTICE',
  '../../third_party/infinite-canvas/LICENSE': 'third_party/infinite-canvas/LICENSE',
  '../../third_party/infinite-canvas/SOURCE.md': 'third_party/infinite-canvas/SOURCE.md',
};
for (const [source, target] of Object.entries(requiredResources)) {
  check(resources[source] === target, 'bundle resource must map ' + source + ' to ' + target);
  if (source !== '../../ui/dist/') {
    check(existsSync(resolve(dirname(configPath), source)), 'bundle resource source is missing: ' + source);
  }
}

const notice = readFileSync(noticePath, 'utf8');
for (const token of [
  'infinite-canvas',
  'third_party/infinite-canvas/LICENSE',
  'Template tag: tauri-cli-v2.11.2',
  'Template revision: ' + expectedUpstreamCommit,
  'Licensed under the Apache License, Version 2.0.',
]) {
  checkIncludes(notice, token, 'NOTICE lost a required third-party attribution anchor: ' + token);
}

const dataRootSource = readFileSync(dataRootSourcePath, 'utf8');
checkIncludes(dataRootSource, 'dirs::data_local_dir()', 'data-root contract must remain per-user local data');
checkIncludes(dataRootSource, 'format!("NomiFun{suffix}")', 'stable data-root leaf must remain NomiFun');

const templatePath = resolve(dirname(configPath), nsis?.template ?? '');
check(
  templatePath === resolve(root, 'apps/desktop/nsis/installer.nsi'),
  'NSIS template must resolve relative to apps/desktop',
);
check(existsSync(templatePath), 'vendored NSIS template is missing');

let source = '';
let sourceBytes = Buffer.alloc(0);
if (existsSync(templatePath)) {
  sourceBytes = readFileSync(templatePath);
  source = sourceBytes.toString('utf8');
  check(sourceBytes.length === 31370, 'vendored template byte length changed unexpectedly');
  check(sha256(sourceBytes) === expectedCustomSha256, 'vendored template SHA-256 changed unexpectedly');
  check(source.split(/\r?\n/).length >= 940, 'vendored template looks incomplete');
}

checkIncludes(source, 'tag tauri-cli-v2.11.2', 'template must record the upstream Tauri tag');
checkIncludes(source, expectedUpstreamCommit, 'template must record the exact upstream commit');
checkIncludes(source, expectedUpstreamSha256, 'template must record the verified upstream SHA-256');
checkIncludes(source, 'SPDX-License-Identifier: Apache-2.0', 'template must carry the selected Apache-2.0 license');

const programsPath = '"$LOCALAPPDATA\\Programs\\${PRODUCTNAME}"';
const legacyPath = '"$LOCALAPPDATA\\${PRODUCTNAME}"';
check(count(source, programsPath) === 1, 'template must define exactly one current-user Programs install path');
check(count(source, legacyPath) === 1, 'legacy colliding install path is allowed only in the migration guard');

const restoreStart = source.indexOf('Function RestorePreviousInstallLocation');
const restoreEnd = source.indexOf('FunctionEnd', restoreStart);
const restoreFunction =
  restoreStart >= 0 && restoreEnd >= 0 ? source.slice(restoreStart, restoreEnd + 'FunctionEnd'.length) : '';
checkIncludes(restoreFunction, legacyPath, 'RestorePreviousInstallLocation must recognize the legacy collision');
checkIncludes(restoreFunction, 'StrCpy $4 ""', 'legacy collision must be cleared instead of restored');
checkIncludes(
  restoreFunction,
  '!if "${INSTALLMODE}" == "currentUser"',
  'legacy location guard must be scoped to currentUser installs',
);

for (const forbidden of [
  'DeleteAppDataCheckbox',
  'DeleteAppDataCheckboxState',
  '$(deleteAppData)',
  'RmDir /r "$APPDATA\\${BUNDLEID}"',
  'RmDir /r "$LOCALAPPDATA\\${BUNDLEID}"',
]) {
  check(!source.includes(forbidden), 'unsafe or misleading delete-data surface remains: ' + forbidden);
}
check(!/RmDir\s+\/r\s+["']?\$LOCALAPPDATA\\NomiFun/i.test(source), 'template must never recursively delete the durable NomiFun data root');

const metadataStart = source.indexOf('; Remove installer-owned metadata on a real uninstall.');
const metadataEnd = source.indexOf('!ifmacrodef NSIS_HOOK_POSTUNINSTALL', metadataStart);
const metadataBlock =
  metadataStart >= 0 && metadataEnd >= 0 ? source.slice(metadataStart, metadataEnd) : '';
for (const token of [
  '${If} $UpdateMode <> 1',
  'DeleteRegKey SHCTX "${MANUPRODUCTKEY}"',
  'DeleteRegKey /ifempty SHCTX "${MANUKEY}"',
  'DeleteRegValue HKCU "${MANUPRODUCTKEY}" "Installer Language"',
  'DeleteRegKey /ifempty HKCU "${MANUKEY}"',
]) {
  checkIncludes(metadataBlock, token, 'non-update uninstall must retain installer metadata cleanup: ' + token);
}

for (const token of [
  '{{#each languages}}',
  '{{#each language_files}}',
  '{{#each resources_dirs}}',
  '{{#each resources}}',
  '{{#each resources_ancestors}}',
  '{{#each binaries}}',
  '{{#each file_associations as |association| ~}}',
  '{{#each deep_link_protocols as |protocol| ~}}',
  '{{no-escape @key}}',
  '{{or association.name ext}}',
  '{{association-description association.description ext}}',
  '!include "utils.nsh"',
  '!include "FileAssociation.nsh"',
  'WriteUninstaller "$INSTDIR\\uninstall.exe"',
]) {
  checkIncludes(source, token, 'vendored template lost a Tauri packaging anchor: ' + token);
}
for (const mode of ['downloadBootstrapper', 'embedBootstrapper', 'offlineInstaller']) {
  checkIncludes(source, '"${INSTALLWEBVIEW2MODE}" == "' + mode + '"', 'vendored template lost WebView2 mode: ' + mode);
}
for (const hook of [
  'NSIS_HOOK_PREINSTALL',
  'NSIS_HOOK_POSTINSTALL',
  'NSIS_HOOK_PREUNINSTALL',
  'NSIS_HOOK_POSTUNINSTALL',
]) {
  checkIncludes(source, '!ifmacrodef ' + hook, 'vendored template lost hook guard: ' + hook);
  checkIncludes(source, '!insertmacro ' + hook, 'vendored template lost hook insertion: ' + hook);
}

const renderedArg = renderedArgument(process.argv.slice(2));
if (renderedArg) {
  const renderedPath = isAbsolute(renderedArg) ? renderedArg : resolve(root, renderedArg);
  check(existsSync(renderedPath), 'rendered installer template is missing: ' + renderedPath);
  if (existsSync(renderedPath)) {
    const rendered = readFileSync(renderedPath, 'utf8');
    check(!rendered.includes('{{'), 'rendered installer still contains Handlebars placeholders');
    checkIncludes(rendered, '!define PRODUCTNAME "NomiFun"', 'rendered installer product name is wrong');
    checkIncludes(rendered, '!define BUNDLEID "com.nomifun.desktop"', 'rendered installer bundle id is wrong');
    checkIncludes(rendered, 'RequestExecutionLevel user', 'rendered installer must remain currentUser');
    checkIncludes(rendered, programsPath, 'rendered installer lost the Programs install path');
    checkIncludes(
      rendered,
      '!define INSTALLWEBVIEW2MODE "downloadBootstrapper"',
      'rendered installer lost the configured WebView2 mode',
    );
    for (const target of [
      'webui-dist',
      'third_party\\infinite-canvas\\LICENSE',
      'third_party\\infinite-canvas\\SOURCE.md',
    ]) {
      checkIncludes(rendered, target, 'rendered installer lost bundled resource: ' + target);
    }
    for (const forbidden of ['DeleteAppDataCheckbox', '$(deleteAppData)', 'RmDir /r "$LOCALAPPDATA']) {
      check(!rendered.includes(forbidden), 'rendered installer contains forbidden delete-data text: ' + forbidden);
    }
  }
}

if (errors.length > 0) {
  console.error('Windows installer contract failed:');
  for (const error of errors) console.error('  - ' + error);
  process.exit(1);
}

console.log(
  'Windows installer contract passed: currentUser Programs path, preserved data, Tauri ' +
    expectedCliVersion +
    ', custom SHA-256 ' +
    expectedCustomSha256,
);
