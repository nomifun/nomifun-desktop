#!/usr/bin/env node
import { readFileSync, readdirSync } from 'node:fs';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], { cwd: root, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 }));
const packages = new Map(metadata.packages.map(pkg => [pkg.name, pkg]));
const allowedCore = new Set(['nomifun-voice-contracts', 'async-trait', 'tokio', 'tokio-util']);
const failures = [];
for (const dependency of packages.get('nomifun-voice-core').dependencies.filter(dep => dep.kind !== 'dev')) {
  if (!allowedCore.has(dependency.name)) failures.push(`voice-core depends on ${dependency.name}`);
}
for (const dependency of packages.get('nomifun-voice').dependencies.filter(dep => dep.kind !== 'dev')) {
  if (['nomifun-app', 'nomifun-model-invoke', 'nomifun-agent-runtime', 'nomifun-agent-session'].includes(dependency.name)) {
    failures.push(`application voice must inject ${dependency.name} through a port`);
  }
}
function files(directory) { return readdirSync(directory, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? files(join(directory, entry.name)) : entry.name.endsWith('.rs') ? [join(directory, entry.name)] : []); }
for (const file of files(join(root, 'crates/backend/nomifun-voice-core/src'))) {
  const source = readFileSync(file, 'utf8').replace(/\/\/[^\n]*|\/\*[\s\S]*?\*\//g, '');
  for (const forbidden of ['nomifun_model_invoke', 'nomifun_agent_runtime', 'nomifun_agent_session', 'nomifun_app', 'reqwest', 'tungstenite', 'rusqlite', 'sqlx']) {
    if (new RegExp(`\\b${forbidden}\\s*::`).test(source)) failures.push(`${file}: forbidden import ${forbidden}`);
  }
}
if (packages.get('nomifun-agent-contracts').dependencies.some(dep => dep.name.startsWith('nomifun-voice'))) failures.push('main Agent contracts depend on optional voice');
const voiceTypes = readFileSync(join(root, 'crates/backend/nomifun-voice-contracts/src/voice.rs'), 'utf8');
for (const nativeField of ['energy_awakeness_threshold', 'prefix_padding_ms', 'silence_duration_ms', 'session.update', 'input_audio_buffer.append']) {
  if (voiceTypes.includes(nativeField)) failures.push(`neutral voice types contain provider wire field ${nativeField}`);
}
if (failures.length) { console.error(failures.join('\n')); process.exit(1); }
console.log('Voice dependency, port and native-wire boundaries passed.');
