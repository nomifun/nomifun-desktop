import { existsSync, readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';

const retiredEndpoint = /\/api\/(?:plugin-drafts(?:[/?#]|$)|plugins\/(?:authoring(?:[/?#]|$)|[^/?#\s]+\/(?:config|surface)(?:[/?#]|$)))/;
function retiredContractEndpoints(value, found = []) {
  if (typeof value === 'string' && retiredEndpoint.test(value)) found.push(value);
  else if (Array.isArray(value)) value.forEach(item => retiredContractEndpoints(item, found));
  else if (value && typeof value === 'object') {
    for (const [key, item] of Object.entries(value)) {
      // Explicit prohibitions document retirement; they are not active APIs.
      if (key !== 'forbidden_paths') retiredContractEndpoints(item, found);
    }
  }
  return found;
}

if (process.argv.includes('--self-test')) {
  assert.deepEqual(retiredContractEndpoints({ operations: [{ path: '/api/plugins/authoring/sessions' }] }), ['/api/plugins/authoring/sessions']);
  assert.deepEqual(retiredContractEndpoints({ payload: { operations: [{ path: '/api/plugins/{plugin_id}/config' }] } }), ['/api/plugins/{plugin_id}/config']);
  assert.deepEqual(retiredContractEndpoints({ path: '/api/plugin-drafts/{id}/preview' }), ['/api/plugin-drafts/{id}/preview']);
  assert.deepEqual(retiredContractEndpoints({ operations: [{ path: '/api/plugins' }, { path: '/api/plugins/{plugin_id}' }, { path: '/api/apps/{id}' }], forbidden_paths: ['/api/plugins/authoring', '/api/plugin-drafts'], purpose: 'plugin_authoring' }), []);
  console.log('App hosting boundary self-test passed: retired active contracts rejected; kernel metadata and typed historical purposes preserved.');
}

const root = resolve(import.meta.dirname, '..');
const retired = [
  'crates/backend/nomifun-plugin-platform/Cargo.toml',
  'crates/backend/nomifun-plugin-development/Cargo.toml',
  'crates/backend/nomifun-js-runtime/Cargo.toml',
  'crates/backend/nomifun-app/src/router/plugin.rs',
  'crates/backend/nomifun-app/src/router/engine_plugin_bindings.rs',
  'crates/backend/nomifun-agent-contracts/src/plugin.rs',
  'ui/src/common/adapter/pluginPlatformBridge.ts',
  'ui/src/common/types/pluginPlatform.ts',
  'docs/architecture/plugin-platform.zh.md',
];
const failures = retired.filter(path => existsSync(resolve(root, path))).map(path => `${path}: retired hosting owner remains`);
const paths = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard'], { cwd: root, encoding: 'utf8' }).split(/\r?\n/);
for (const path of paths) {
  if (!/^(?:crates|apps|ui)\/.+\.(?:rs|toml|ts|tsx|json)$/.test(path) || !existsSync(resolve(root, path))) continue;
  const source = readFileSync(resolve(root, path), 'utf8');
  if (path.endsWith('.json')) {
    try {
      for (const endpoint of new Set(retiredContractEndpoints(JSON.parse(source)))) {
        failures.push(`${path}: retired plugin hosting endpoint remains active: ${endpoint}`);
      }
    } catch (error) {
      failures.push(`${path}: active JSON contract cannot be inspected: ${error.message}`);
    }
    continue;
  }
  if (/\bnomifun_plugin_(?:platform|development)\b|nomifun-plugin-(?:platform|development)|pluginPlatformBridge|pluginDevelopment|\/api\/plugin-drafts|window\.nomi\b/.test(source)) {
    failures.push(`${path}: retired plugin hosting dependency or execution path`);
  }
}
const core = readFileSync(resolve(root, 'crates/backend/nomifun-app-hosting/src/owner.rs'), 'utf8');
if (!core.includes('nomi_process_runtime') || !core.includes('NOMIFUN_LOCAL_TRUST')) failures.push('hosting must use managed processes and current LocalTrust');
const migration = readFileSync(resolve(root, 'crates/backend/nomifun-db/migrations/004_retire_plugin_hosting.sql'), 'utf8');
if (/\b(?:agent_\w+|schema_metadata|_sqlx_migrations)\b/i.test(migration)) failures.push('hosting retirement must preserve canonical Agent facts and published receipts');
if (failures.length) {
  console.error(`App hosting boundary failed:\n${failures.map(value => `  - ${value}`).join('\n')}`);
  process.exitCode = 1;
} else console.log('App hosting boundary passed: one process owner, retired plugin domain absent, canonical history preserved.');
