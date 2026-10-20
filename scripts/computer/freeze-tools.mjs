#!/usr/bin/env node
// Capture metadata only. This command never sends a desktop action to the worker.
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
export const manifestPath = path.join(root, 'crates/backend/nomifun-agent-contracts/contracts/computer-tools.v1.json');
export const overridesPath = path.join(root, 'crates/backend/nomifun-agent-contracts/contracts/computer-tools.overrides.v1.json');
export const upstream = {
  repository: 'https://github.com/trycua/cua',
  requested_tag: 'cua-driver-rs-v0.37.0',
  commit: '4d959bbfa57f0c89f310330e849500b222084194',
};
const platforms = ['linux', 'macos', 'windows'];
const platform = { darwin: 'macos', win32: 'windows', linux: 'linux' }[process.platform];

export function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])]));
  }
  return value;
}

export function digest(value) {
  return createHash('sha256').update(JSON.stringify(canonical(value))).digest('hex');
}

function stableDriverMetadata(metadata) {
  const fields = ['driver_version', 'contract_version', 'tools_list_schema_version', 'capability_version', 'mcp_protocol_version'];
  if (!metadata || fields.some(field => typeof metadata[field] !== 'string')) throw new Error('Incomplete worker driver metadata');
  return Object.fromEntries(fields.map(field => [field, metadata[field]]));
}

export function validateOverrides(overrides) {
  if (overrides.schema_version !== 1 || !Array.isArray(overrides.tools) || overrides.tools.length !== 22) {
    throw new Error('Computer overrides must declare schema_version 1 and exactly 22 tools');
  }
  const names = new Set();
  for (const tool of overrides.tools) {
    if (!/^[a-z][a-z0-9_]*$/.test(tool.driver_tool) || names.has(tool.driver_tool)) {
      throw new Error(`Invalid or duplicate driver tool: ${tool.driver_tool}`);
    }
    names.add(tool.driver_tool);
    for (const locale of ['en-US', 'zh-CN']) {
      if (!tool.titles?.[locale] || !tool.descriptions?.[locale]) throw new Error(`Missing ${locale} presentation: ${tool.driver_tool}`);
    }
    if (!['Pure', 'ReadSensitive', 'Physical', 'ExecuteLocal'].includes(tool.effect_class)
      || !['observe', 'input', 'launch'].includes(tool.resource_operation)
      || !['eager', 'deferred'].includes(tool.loading)
      || !['all', 'full_only', 'window_without_image', 'element_token', 'steps'].includes(tool.no_image)) {
      throw new Error(`Invalid Computer metadata: ${tool.driver_tool}`);
    }
    if (typeof tool.system_permissions?.accessibility !== 'boolean'
      || typeof tool.system_permissions?.screenshot_default !== 'boolean'
      || !['never', 'always', 'window_screenshot', 'batch_screenshot'].includes(tool.system_permissions?.screen_recording)) {
      throw new Error(`Invalid Computer system permissions: ${tool.driver_tool}`);
    }
  }
}

function replaceToolNames(text, tools) {
  const names = tools.map(tool => tool.driver_tool).sort((a, b) => b.length - a.length);
  return text.replace(new RegExp(`(?<![a-zA-Z0-9_])(${names.join('|')})(?![a-zA-Z0-9_])`, 'g'), 'computer_$1');
}

// Only remove fields or replace prose. Every remaining structural schema value
// comes from this platform's actual worker list, including nested schemas.
export function compactSchema(inputSchema, overrides, tool) {
  const schema = structuredClone(inputSchema);
  const trustedParameters = new Set(overrides.trusted_parameters);
  function visit(node, rootNode = false) {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) {
      for (const child of node) visit(child);
      return;
    }
    if (node.properties) {
      const duplicateTargetFields = rootNode && node.properties.target ? new Set(overrides.remove_when_target_present) : new Set();
      const removed = new Set();
      for (const name of Object.keys(node.properties)) {
        if (overrides.strip_parameter_prefixes.some(prefix => name.startsWith(prefix)) || trustedParameters.has(name) || overrides.remove_parameters.includes(name)
          || (rootNode && tool.remove_parameters?.includes(name)) || duplicateTargetFields.has(name)) {
          delete node.properties[name];
          removed.add(name);
        } else if (overrides.parameter_descriptions[name]
          && (name !== 'steps' || node.properties[name].type === 'array')) {
          node.properties[name].description = overrides.parameter_descriptions[name];
        }
      }
      if (node.required) node.required = node.required.filter(name => !removed.has(name));
    }
    for (const [key, child] of Object.entries(node)) {
      if (key === 'description' && typeof child === 'string') node[key] = replaceToolNames(child, overrides.tools);
      else visit(child);
    }
  }
  visit(schema, true);
  if (tool.driver_tool === 'run_steps') {
    const stepTool = schema.properties?.steps?.items?.properties?.tool;
    if (!Array.isArray(stepTool?.enum)) throw new Error('run_steps no longer declares its batchable tool enum');
    const exposed = new Set(overrides.tools.map(item => item.driver_tool));
    stepTool.enum = stepTool.enum.filter(name => exposed.has(name)).map(name => `computer_${name}`);
    if (stepTool.enum.length === 0) throw new Error('run_steps has no exposed batchable tools');
  }
  return canonical(schema);
}

export function freezeInventory({ list, metadata, currentPlatform, overrides, previous = null }) {
  validateOverrides(overrides);
  if (!platforms.includes(currentPlatform)) throw new Error(`Unsupported capture platform: ${currentPlatform}`);
  if (list.schema_version !== '1' || !Array.isArray(list.tools)) throw new Error('Unexpected worker list schema');
  const definitions = new Map();
  for (const definition of list.tools) {
    if (typeof definition.name !== 'string' || definitions.has(definition.name)) throw new Error('Invalid or duplicate tool in worker list');
    definitions.set(definition.name, definition);
  }
  // Never silently retain definitions produced from a different source/profile.
  if (previous && (digest(previous.upstream) !== digest(upstream) || previous.overrides_sha256 !== digest(overrides))) {
    const otherPlatforms = Object.keys(previous.captures ?? {}).filter(name => name !== currentPlatform);
    if (otherPlatforms.length) throw new Error(`Source/profile changed; recapture all platforms together (still present: ${otherPlatforms.join(', ')})`);
  }
  const captures = { ...previous?.captures };
  captures[currentPlatform] = {
    metadata: stableDriverMetadata(metadata),
    list_schema_version: list.schema_version,
    capability_version: list.capability_version,
    list_sha256: digest(list),
    listed_tools: list.tools.length,
  };
  const tools = overrides.tools.map(tool => {
    const definition = definitions.get(tool.driver_tool);
    if (!definition?.inputSchema || typeof definition.description !== 'string') {
      throw new Error(`Required Computer tool missing from ${currentPlatform} worker list: ${tool.driver_tool}`);
    }
    const platformDefinitions = { ...previous?.tools?.find(item => item.driver_tool === tool.driver_tool)?.platforms };
    platformDefinitions[currentPlatform] = {
      input_schema: compactSchema(definition.inputSchema, overrides, tool),
      annotations: definition.annotations,
      capabilities: definition.capabilities,
      source_definition_sha256: digest(definition),
    };
    return {
      action_id: `computer/${tool.driver_tool}`,
      model_tool: `computer_${tool.driver_tool}`,
      driver_tool: tool.driver_tool,
      titles: tool.titles,
      descriptions: tool.descriptions,
      effect_class: tool.effect_class,
      resource_operation: tool.resource_operation,
      loading: tool.loading,
      no_image: tool.no_image,
      group: tool.group,
      system_permissions: tool.system_permissions,
      platforms: platformDefinitions,
    };
  });
  return canonical({
    schema_version: 1,
    upstream,
    overrides_sha256: digest(overrides),
    identity: overrides.identity,
    instruction_sources: overrides.instruction_sources,
    instructions: overrides.instructions,
    schema_profile: {
      trusted_parameters: overrides.trusted_parameters,
      strip_parameter_prefixes: overrides.strip_parameter_prefixes,
      remove_parameters: overrides.remove_parameters,
      remove_when_target_present: overrides.remove_when_target_present,
    },
    captures,
    tools,
  });
}

export function isolatedEnvironment(directory, hostPlatform = process.platform) {
  const env = {};
  for (const name of ['PATH', 'SystemRoot', 'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'LANG', 'LC_ALL']) {
    if (process.env[name] !== undefined) env[name] = process.env[name];
  }
  return {
    ...env,
    HOME: directory,
    USERPROFILE: directory,
    APPDATA: path.join(directory, 'config'),
    LOCALAPPDATA: path.join(directory, 'local'),
    TEMP: directory,
    TMP: directory,
    XDG_CONFIG_HOME: path.join(directory, 'config'),
    XDG_DATA_HOME: path.join(directory, 'data'),
    XDG_STATE_HOME: path.join(directory, 'state'),
    XDG_CACHE_HOME: path.join(directory, 'cache'),
    ...(hostPlatform === 'linux' ? {
      XDG_RUNTIME_DIR: directory,
      // The SDK discovers the real user's bus when this variable is absent.
      // An explicit unreachable endpoint keeps metadata discovery isolated.
      DBUS_SESSION_BUS_ADDRESS: `unix:path=${path.join(directory, 'no-session-bus')}`,
    } : {}),
    CUA_DRIVER_RS_HOME: path.join(directory, 'driver'),
    CUA_DRIVER_CURSOR_THEME_DIR: path.join(directory, 'driver', 'cursor-themes'),
    NO_AT_BRIDGE: '1',
  };
}

export async function captureWorkerInventory(worker, { timeoutMs = 30000, prefixArguments = [] } = {}) {
  const directory = await mkdtemp(path.join(tmpdir(), 'nomifun-computer-freeze-'));
  const env = isolatedEnvironment(directory);
  await Promise.all(['config', 'local', 'data', 'state', 'cache', 'driver/cursor-themes'].map(name => mkdir(path.join(directory, name), { recursive: true })));
  const generation = 'nomifun-freeze-tools';
  const child = spawn(worker, [...prefixArguments, '__private-worker', '--generation', generation], {
    cwd: directory, env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
  });
  let pending;
  let buffer = '';
  let stderr = '';
  let transportError;
  let timer;
  let exited = false;
  const exit = new Promise((resolve, reject) => {
    child.once('error', error => { transportError = error; pending?.reject(error); reject(error); });
    child.once('close', (code, signal) => {
      exited = true;
      const error = new Error(`Worker exited before completing discovery (code ${code}, signal ${signal ?? 'none'})`);
      pending?.reject(error);
      resolve({ code, signal });
    });
  });
  // Keep failures handled while requests own the more precise diagnostic.
  exit.catch(() => {});
  child.stdin.on('error', error => { transportError = error; pending?.reject(error); });
  child.stderr.on('data', chunk => { stderr = (stderr + chunk.toString('utf8')).slice(-8192); });
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => {
    buffer += chunk;
    if (buffer.length > 16 * 1024 * 1024) {
      transportError = new Error('Worker discovery response exceeded 16 MiB');
      pending?.reject(transportError);
      child.kill();
      return;
    }
    let newline;
    while ((newline = buffer.indexOf('\n')) >= 0) {
      const line = buffer.slice(0, newline);
      buffer = buffer.slice(newline + 1);
      try {
        const response = JSON.parse(line);
        if (!pending || response.protocol_version !== 1 || response.request_id !== pending.id || response.generation !== generation) {
          throw new Error('Worker discovery response identity mismatch');
        }
        if (!response.ok) throw new Error(`Worker discovery rejected: ${response.error_code ?? response.error ?? 'unknown'}`);
        if (response.completion !== 'completed') throw new Error('Worker discovery did not complete');
        pending.resolve(response.result);
        pending = undefined;
      } catch (error) {
        transportError = error;
        pending?.reject(error);
        child.kill();
      }
    }
  });
  let requestId = 0;
  function request(operation, argumentsValue) {
    if (transportError) return Promise.reject(transportError);
    if (exited) return Promise.reject(new Error('Worker already exited'));
    return new Promise((resolve, reject) => {
      pending = { id: requestId, resolve, reject };
      child.stdin.write(`${JSON.stringify({ protocol_version: 1, request_id: requestId++, generation, operation, ...(argumentsValue === undefined ? {} : { arguments: argumentsValue }) })}\n`, error => {
        if (error) { transportError = error; pending?.reject(error); }
      });
    });
  }
  timer = setTimeout(() => {
    transportError = new Error(`Worker discovery exceeded ${timeoutMs} ms`);
    pending?.reject(transportError);
    child.kill();
  }, timeoutMs);
  try {
    await request('initialize', {
      configured_driver: {
        claude_code_compatibility: false,
        authorization: {
          allowed_modes: ['standard'], compatibility_mode: 'standard',
          compatibility_capability_manifest_path: null, compatibility_bounded_manifest_path: null,
          unrestricted_acknowledged: false, max_session_ttl_seconds: 86400, max_idle_ttl_seconds: 3600,
        },
      },
      host_bundle_id: 'com.nomifun.computer-worker.freeze',
    });
    const metadata = await request('metadata');
    const list = await request('list');
    await request('shutdown');
    child.stdin.end();
    const result = await exit;
    if (result.code !== 0 || result.signal) throw new Error(`Worker discovery shutdown failed (code ${result.code}, signal ${result.signal})`);
    return { metadata, list };
  } catch (error) {
    throw new Error(`${error.message}${stderr ? `\nWorker stderr: ${stderr}` : ''}`, { cause: error });
  } finally {
    clearTimeout(timer);
    if (!exited) { child.stdin.destroy(); child.kill(); }
    await exit.catch(() => {});
    const relative = path.relative(path.resolve(tmpdir()), path.resolve(directory));
    if (relative.startsWith('..') || path.isAbsolute(relative) || !path.basename(directory).startsWith('nomifun-computer-freeze-')) {
      throw new Error(`Refusing to remove unexpected discovery directory: ${directory}`);
    }
    await rm(directory, { recursive: true, force: true });
  }
}

async function main(args) {
  let worker;
  let check = false;
  for (let index = 0; index < args.length; index++) {
    if (args[index] === '--worker') worker = args[++index];
    else if (args[index] === '--check') check = true;
    else throw new Error(`Unknown argument: ${args[index]}`);
  }
  if (!worker || !platform) throw new Error('Usage: node scripts/computer/freeze-tools.mjs --worker <binary> [--check]');
  const overrides = JSON.parse(await readFile(overridesPath, 'utf8'));
  let previous = null;
  try { previous = JSON.parse(await readFile(manifestPath, 'utf8')); }
  catch (error) { if (error.code !== 'ENOENT') throw error; }
  const inventory = await captureWorkerInventory(path.resolve(worker));
  const manifest = freezeInventory({ ...inventory, currentPlatform: platform, overrides, previous });
  const serialized = `${JSON.stringify(manifest, null, 2)}\n`;
  if (check) {
    if (!previous || JSON.stringify(canonical(previous)) !== JSON.stringify(manifest)) {
      throw new Error(`Frozen Computer tools differ from the ${platform} worker; regenerate and review ${manifestPath}`);
    }
  } else {
    await writeFile(manifestPath, serialized);
  }
  const captured = Object.keys(manifest.captures).sort();
  const missing = platforms.filter(name => !captured.includes(name));
  process.stdout.write(`Computer tools ${check ? 'verified' : 'frozen'}: ${manifest.tools.length} tools; source: ${upstream.commit}; actual worker list: ${platform}; captured: ${captured.join(', ')}; pending: ${missing.join(', ') || 'none'}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main(process.argv.slice(2)).catch(error => { process.stderr.write(`${error.message}\n`); process.exitCode = 1; });
}
