#!/usr/bin/env node

/**
 * Enforce the Browser Platform ownership boundary.
 *
 * Conversation Browser dispatches through its native Workspace owner. The
 * retired Headless Hub must not be constructed by any application entry point.
 * Background browser work requires the new isolated owners.
 * Low-level standalone helpers must not leak into application entry points.
 */

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const BOOTSTRAP = 'crates/agent/nomi-agent/src/bootstrap.rs';
const BROWSER_TOOL = 'crates/agent/nomi-browser/src/tool.rs';
const ENGINE_BACKEND = 'crates/agent/nomi-browser-engine/src/backend/cdp.rs';
const PLATFORM_ADAPTER = 'crates/agent/nomi-browser/src/platform_adapter.rs';
const GATEWAY_REGISTRY = 'crates/backend/nomifun-gateway/src/browser_registry.rs';
const HUB_COMPOSITION = 'crates/backend/nomifun-app/src/services.rs';
const KNOWLEDGE_BROWSER_COMPOSITION =
  'crates/backend/nomifun-app/src/services.rs';
const FRESH_V4_COMPOSITION =
  'crates/backend/nomifun-app/src/router/agent_platform_host.rs';

const OWNERSHIP_BOUNDARY_PREFIXES = [
  'apps/desktop/src/',
  'crates/agent/nomi-agent/src/',
  'crates/backend/nomifun-app/src/',
  'crates/backend/nomifun-gateway/src/',
  'crates/backend/nomifun-ai-agent/src/',
];
// Keep the terminology used by the scanner predicates explicit.  The
// application prefixes cover the ownership/composition layer; the low-level
// engine is included separately below so its global-cursor and gate checks
// still run without treating the engine as an application entry point.
const APPLICATION_PRODUCTION_PREFIXES = OWNERSHIP_BOUNDARY_PREFIXES;
const ENGINE_PRODUCTION_PREFIX = 'crates/agent/nomi-browser-engine/src/';
const RETIRED_ENGINE_FILES = new Set([
  'crates/agent/nomi-browser-engine/src/actionability.rs',
  'crates/agent/nomi-browser-engine/src/actions.rs',
  'crates/agent/nomi-browser-engine/src/aria_ref.rs',
  'crates/agent/nomi-browser-engine/src/backend/cdp.rs',
  'crates/agent/nomi-browser-engine/src/backend/mod.rs',
  'crates/agent/nomi-browser-engine/src/debug_capture.rs',
  'crates/agent/nomi-browser-engine/src/domain.rs',
  'crates/agent/nomi-browser-engine/src/errmap.rs',
  'crates/agent/nomi-browser-engine/src/evaluate.rs',
  'crates/agent/nomi-browser-engine/src/firewall.rs',
  'crates/agent/nomi-browser-engine/src/host.rs',
  'crates/agent/nomi-browser-engine/src/nav.rs',
  'crates/agent/nomi-browser-engine/src/observe.rs',
  'crates/agent/nomi-browser-engine/src/progress.rs',
  'crates/agent/nomi-browser-engine/src/selector.rs',
  'crates/agent/nomi-browser-engine/src/storage_state.rs',
  'crates/agent/nomi-browser-engine/src/tabs.rs',
  'crates/agent/nomi-browser-engine/src/test_support.rs',
  'crates/agent/nomi-browser-engine/tests/snapshots/observe_fixtures__observe_inject_contract_iframe.snap',
  'crates/agent/nomi-browser-engine/tests/snapshots/observe_fixtures__observe_iframe_stitched.snap',
  'crates/agent/nomi-browser-engine/tests/snapshots/integration_act__hit_target_contract.snap',
  'crates/agent/nomi-browser-engine/tests/snapshots/integration_act__check_states_contract.snap',
  'crates/agent/nomi-browser-engine/tests/op_mutex_concurrency.rs',
  'crates/agent/nomi-browser-engine/tests/observe_fixtures.rs',
  'crates/agent/nomi-browser-engine/tests/integration_w4c.rs',
  'crates/agent/nomi-browser-engine/tests/integration_w4b.rs',
  'crates/agent/nomi-browser-engine/tests/integration_takeover.rs',
  'crates/agent/nomi-browser-engine/tests/integration_storage_snapshot.rs',
  'crates/agent/nomi-browser-engine/tests/integration_single_tab.rs',
  'crates/agent/nomi-browser-engine/tests/integration_oopif.rs',
  'crates/agent/nomi-browser-engine/tests/integration_nav.rs',
  'crates/agent/nomi-browser-engine/tests/integration_multiorigin_storage.rs',
  'crates/agent/nomi-browser-engine/tests/integration_indexeddb.rs',
  'crates/agent/nomi-browser-engine/tests/integration_factions.rs',
  'crates/agent/nomi-browser-engine/tests/integration_e5.rs',
  'crates/agent/nomi-browser-engine/tests/integration_e4.rs',
  'crates/agent/nomi-browser-engine/tests/integration_debug_capture.rs',
  'crates/agent/nomi-browser-engine/tests/integration_d4.rs',
  'crates/agent/nomi-browser-engine/tests/integration_act.rs',
  'crates/agent/nomi-browser-engine/tests/fixtures/upload.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/switch-frame.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/spa-softnav.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/shadow.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/secrets.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/page-b.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/page-a.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/never-idle.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/modal-overlay.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/input-synth.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/iframe.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/firewall.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/download_exe.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/download.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/c3.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/c2.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/actionability.html',
  'crates/agent/nomi-browser-engine/tests/fixtures/act-c1.html',
  'crates/agent/nomi-browser-engine/tests/engine_lifecycle.rs',
  'crates/agent/nomi-browser-engine/tests/common/mod.rs',
]);
const RETIRED_BROWSER_UI_FILES = new Set([
  'ui/src/renderer/components/media/WebviewHost.tsx',
  'ui/src/renderer/pages/conversation/Preview/components/viewers/URLViewer.tsx',
  'ui/src/renderer/pages/conversation/Preview/components/viewers/HTMLViewer.tsx',
  'ui/src/renderer/components/settings/SettingsModal/contents/BrowserUseSettingsContent.tsx',
  'ui/src/renderer/components/settings/SettingsModal/contents/BrowserUseSettingsContent.test.ts',
  'ui/src/renderer/components/layout/Sider/SiderNav/SiderBrowserEntry.tsx',
  ...['browserSession', 'browserSettings', 'browserDisplayModeController', 'browserTypes']
    .flatMap(name => [`ui/src/common/browser/${name}.ts`, `ui/src/common/browser/${name}.test.ts`]),
]);
const RETIRED_KNOWLEDGE_BROWSER_FILES = new Set([
  'crates/backend/nomifun-ai-agent/src/browser_fetcher.rs',
  'crates/backend/nomifun-ai-agent/src/browser_fetcher_tests.rs',
]);
const RETIRED_BROWSER_VAULT_FILES = new Set([
  'crates/agent/nomi-browser-engine/src/vault.rs',
  'crates/agent/nomi-browser-engine/tests/integration_w4d.rs',
]);

const normalizePath = (path) => path.replaceAll('\\', '/');

function workspacePaths() {
  const output = execFileSync(
    'git',
    [
      'ls-files',
      '--cached',
      '--others',
      '--exclude-standard',
      '-z',
      '--',
      'crates',
      'apps',
      'ui/src',
    ],
    { cwd: ROOT, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 },
  );
  return [...new Set(output.split('\0').filter(Boolean).map(normalizePath))];
}

function isIdent(byte) {
  return (
    (byte >= 48 && byte <= 57) ||
    (byte >= 65 && byte <= 90) ||
    (byte >= 97 && byte <= 122) ||
    byte === 95
  );
}

function replaceNonNewline(source, start, end) {
  return (
    source.slice(0, start) +
    source.slice(start, end).replace(/[^\r\n]/g, ' ') +
    source.slice(end)
  );
}

function rawStringEnd(source, index) {
  let cursor = index;
  if (source[cursor] === 'b') cursor += 1;
  if (source[cursor] !== 'r') return null;
  cursor += 1;
  let hashes = 0;
  while (source[cursor] === '#') {
    hashes += 1;
    cursor += 1;
  }
  if (source[cursor] !== '"') return null;
  const terminator = `"${'#'.repeat(hashes)}`;
  const end = source.indexOf(terminator, cursor + 1);
  return end === -1 ? source.length : end + terminator.length;
}

function quotedEnd(source, index, quote) {
  let cursor = index + 1;
  while (cursor < source.length) {
    if (source[cursor] === '\\') {
      cursor += 2;
    } else if (source[cursor] === quote) {
      return cursor + 1;
    } else {
      cursor += 1;
    }
  }
  return source.length;
}

function charLiteralEnd(source, index) {
  const end = quotedEnd(source, index, "'");
  if (end >= source.length || source[end - 1] !== "'") return null;
  const body = source.slice(index + 1, end - 1);
  if (
    body.length === 1 ||
    /^\\(?:[nrt0'"\\]|x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\})$/.test(body)
  ) {
    return end;
  }
  return null;
}

/**
 * Replace comments and literals while preserving byte offsets and newlines.
 * Browser ownership terms in documentation and error strings must not become
 * false boundary violations.
 */
function lexicalMask(source, { keepLiterals = false } = {}) {
  let output = source;
  let index = 0;
  while (index < source.length) {
    if (source.startsWith('//', index)) {
      const end = source.indexOf('\n', index + 2);
      const stop = end === -1 ? source.length : end;
      output = replaceNonNewline(output, index, stop);
      index = stop;
      continue;
    }
    if (source.startsWith('/*', index)) {
      let cursor = index + 2;
      let depth = 1;
      while (cursor < source.length && depth > 0) {
        if (source.startsWith('/*', cursor)) {
          depth += 1;
          cursor += 2;
        } else if (source.startsWith('*/', cursor)) {
          depth -= 1;
          cursor += 2;
        } else {
          cursor += 1;
        }
      }
      output = replaceNonNewline(output, index, cursor);
      index = cursor;
      continue;
    }
    const rawEnd = rawStringEnd(source, index);
    if (rawEnd !== null) {
      if (!keepLiterals) output = replaceNonNewline(output, index, rawEnd);
      index = rawEnd;
      continue;
    }
    if (source[index] === '"' || source.startsWith('b"', index)) {
      const quote = source[index] === '"' ? index : index + 1;
      const end = quotedEnd(source, quote, '"');
      if (!keepLiterals) output = replaceNonNewline(output, index, end);
      index = end;
      continue;
    }
    if (
      source[index] === "'" &&
      (index === 0 || !isIdent(source.charCodeAt(index - 1)))
    ) {
      const end = charLiteralEnd(source, index);
      if (end !== null) {
        if (!keepLiterals) output = replaceNonNewline(output, index, end);
        index = end;
        continue;
      }
    }
    index += 1;
  }
  return output;
}

function skipSpace(source, index) {
  while (index < source.length && /\s/.test(source[index])) index += 1;
  return index;
}

function attributeEnd(source, index) {
  if (source[index] !== '#' || source[index + 1] !== '[') return null;
  let depth = 1;
  for (let cursor = index + 2; cursor < source.length; cursor += 1) {
    if (source[cursor] === '[') depth += 1;
    if (source[cursor] === ']') {
      depth -= 1;
      if (depth === 0) return cursor + 1;
    }
  }
  return source.length;
}

function matchingBrace(source, open) {
  let depth = 1;
  for (let cursor = open + 1; cursor < source.length; cursor += 1) {
    if (source[cursor] === '{') depth += 1;
    if (source[cursor] === '}') {
      depth -= 1;
      if (depth === 0) return cursor + 1;
    }
  }
  return source.length;
}

function splitTopLevelArguments(source) {
  const arguments_ = [];
  let start = 0;
  let depth = 0;
  for (let index = 0; index < source.length; index += 1) {
    if (source[index] === '(') depth += 1;
    if (source[index] === ')') depth = Math.max(0, depth - 1);
    if (source[index] === ',' && depth === 0) {
      arguments_.push(source.slice(start, index));
      start = index + 1;
    }
  }
  arguments_.push(source.slice(start));
  return arguments_;
}

function isTestOnlyCfgAttribute(attribute) {
  const compact = attribute.replace(/\s/g, '');
  if (compact === '#[cfg(test)]') return true;
  const prefix = '#[cfg(all(';
  const suffix = '))]';
  if (!compact.startsWith(prefix) || !compact.endsWith(suffix)) return false;
  return splitTopLevelArguments(compact.slice(prefix.length, -suffix.length)).some(
    (argument) => argument === 'test',
  );
}

function leadingAttributes(source, index) {
  const attributes = [];
  let cursor = skipSpace(source, index);
  while (source[cursor] === '#') {
    const end = attributeEnd(source, cursor);
    if (end === null) break;
    attributes.push({
      start: cursor,
      end,
      source: source.slice(cursor, end),
    });
    cursor = skipSpace(source, end);
  }
  return { attributes, itemStart: cursor };
}

function attributedItemEnd(source, index) {
  let cursor = leadingAttributes(source, index).itemStart;
  let paren = 0;
  let bracket = 0;
  for (; cursor < source.length; cursor += 1) {
    const char = source[cursor];
    if (char === '(') paren += 1;
    if (char === ')') paren = Math.max(0, paren - 1);
    if (char === '[') bracket += 1;
    if (char === ']') bracket = Math.max(0, bracket - 1);
    if (paren === 0 && bracket === 0) {
      if (char === ';') return cursor + 1;
      if (char === '{') return matchingBrace(source, cursor);
    }
  }
  return source.length;
}

function productionMask(source, { keepLiterals = false } = {}) {
  const masked = lexicalMask(source);
  let output = keepLiterals ? source : masked;
  let index = 0;
  while (index < masked.length) {
    if (masked[index] !== '#' || masked[index + 1] !== '[') {
      index += 1;
      continue;
    }
    const group = leadingAttributes(masked, index);
    if (group.attributes.length === 0) {
      index += 1;
      continue;
    }
    if (
      group.attributes.some((attribute) =>
        isTestOnlyCfgAttribute(attribute.source),
      )
    ) {
      const itemEnd = attributedItemEnd(masked, index);
      output = replaceNonNewline(output, index, itemEnd);
      index = itemEnd;
    } else {
      index = group.itemStart;
    }
  }
  return output;
}

function lineNumber(source, index) {
  return source.slice(0, index).split('\n').length;
}

function snippetAt(source, index) {
  const start = source.lastIndexOf('\n', index - 1) + 1;
  const end = source.indexOf('\n', index);
  return source.slice(start, end === -1 ? source.length : end).trim();
}

function findMatches(source, pattern) {
  pattern.lastIndex = 0;
  return [...source.matchAll(pattern)].map((match) => ({
    index: match.index ?? 0,
    text: match[0],
  }));
}

function readEntry(path) {
  const absolute = resolve(ROOT, path);
  if (!existsSync(absolute)) return null;
  return { path, source: readFileSync(absolute, 'utf8') };
}

function isRustSourcePath(path) {
  return (
    path.endsWith('.rs') &&
    !path.includes('/tests/') &&
    !path.includes('/examples/') &&
    !path.includes('/benches/') &&
    !path.endsWith('_tests.rs') &&
    !path.endsWith('_test.rs')
  );
}

function isUiProductionSourcePath(path) {
  return (
    (path.endsWith('.ts') || path.endsWith('.tsx')) &&
    path.startsWith('ui/src/') &&
    !path.includes('/tests/') &&
    !path.endsWith('.test.ts') &&
    !path.endsWith('.test.tsx') &&
    !path.endsWith('.spec.ts') &&
    !path.endsWith('.spec.tsx') &&
    !path.endsWith('.d.ts')
  );
}

function isApplicationProductionPath(path) {
  return (
    isRustSourcePath(path) &&
    APPLICATION_PRODUCTION_PREFIXES.some((prefix) => path.startsWith(prefix))
  );
}

function isOwnershipBoundaryPath(path) {
  return isApplicationProductionPath(path);
}

function isEngineProductionPath(path) {
  return isRustSourcePath(path) && path.startsWith(ENGINE_PRODUCTION_PREFIX);
}

function browserToolConstructorPattern() {
  return /\b(?:[A-Za-z_]\w*\s*::\s*)*BrowserTool\s*::\s*(?:new|new_standalone|with_data_dir)\s*(?:::<[^;{}()]*>\s*)?\(/g;
}

function knowledgeBrowserBypassPattern() {
  return /\b(?:nomifun_ai_agent\s*::\s*)?BrowserFetcher\b|\.set_render_fetcher\s*\(/g;
}

function browserPolicyConstructorPattern() {
  return /\b(?:[A-Za-z_]\w*\s*::\s*)*BrowserTool\s*::\s*with_policy\s*(?:::<[^;{}()]*>\s*)?\(/g;
}

function hubConstructorPattern() {
  return /\b(?:[A-Za-z_]\w*\s*::\s*)*BrowserSessionHub\s*::\s*(?:new|with_clock)\s*(?:::<[^;{}()]*>\s*)?\(/g;
}

function privateEngineConstructorPattern() {
  return /\b(?:nomi_browser_engine\s*::\s*)create_engine\s*(?:::<[^;{}()]*>\s*)?\(/g;
}

function managedHostLaunchPattern() {
  // Production Platform callers must use the authority-carrying launch path;
  // retain `launch` in the matcher as well so legacy/direct launches outside
  // the adapter remain boundary violations.
  return /\b(?:[A-Za-z_]\w*\s*::\s*)*ManagedBrowserHost\s*::\s*launch(?:_platform_managed(?:_with_cleanup_lease)?)?\s*(?:::<[^;{}()]*>\s*)?\(/g;
}

function managedBrowserFacadeConstructorPattern() {
  return /\b(?:[A-Za-z_]\w*\s*::\s*)*BrowserTool\s*::\s*new_managed\s*(?:::<[^;{}()]*>\s*)?\(/g;
}

function standaloneBrowserConstructorPattern() {
  return /\b(?:Self|BrowserTool)\s*::\s*(?:new|new_standalone|with_data_dir)\s*(?:::<[^;{}()]*>\s*)?\(/g;
}

function standaloneProfileAllocationPattern() {
  return /\b(?:allocate_profile_dir\s*\(|profile_dir\s*:(?!\s*PathBuf\s*::\s*new\s*\(\s*\)))/g;
}

function structRanges(source) {
  const ranges = [];
  const pattern = /\bstruct\s+([A-Za-z_]\w*)\b/g;
  pattern.lastIndex = 0;
  for (const match of source.matchAll(pattern)) {
    const index = match.index ?? 0;
    const open = source.indexOf('{', index + match[0].length);
    if (open < 0) continue;
    ranges.push({
      name: match[1],
      start: open,
      end: matchingBrace(source, open),
    });
  }
  return ranges;
}

function enclosingStructAt(ranges, index) {
  return ranges
    .filter((range) => index >= range.start && index < range.end)
    .sort((left, right) => (left.end - left.start) - (right.end - right.start))[0] ??
    null;
}

function isLaneOwnedStruct(name) {
  return name === 'CdpBackend' || /(?:lane|route|tab)/i.test(name);
}

function normalizedIdentifier(identifier) {
  return identifier
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .replaceAll('-', '_')
    .toLowerCase();
}

function suspiciousGateIdentifier(identifier) {
  const normalized = normalizedIdentifier(identifier);
  if (!/(?:mutex|lock|gate)$/.test(normalized)) return false;
  return /(?:browser|companion|execution|operation|engine|per_companion)/.test(
    normalized,
  );
}

function namedDeclarations(source) {
  const declarations = [];
  const pattern = /\b(static\s+(?:mut\s+)?)?([A-Za-z_]\w*)\s*:/g;
  pattern.lastIndex = 0;
  for (const match of source.matchAll(pattern)) {
    declarations.push({
      index: match.index ?? 0,
      text: match[0],
      isStatic: Boolean(match[1]),
      name: match[2],
    });
  }
  return declarations;
}

function staticDeclarations(source) {
  const declarations = [];
  const pattern = /\bstatic\s+(?:mut\s+)?([A-Za-z_]\w*)\s*:/g;
  pattern.lastIndex = 0;
  for (const match of source.matchAll(pattern)) {
    const name = match[1];
    const nameIndex = (match.index ?? 0) + match[0].indexOf(name);
    declarations.push({
      index: nameIndex,
      text: match[0],
      isStatic: true,
      name,
    });
  }
  return declarations;
}

function structFieldDeclarations(source, ranges) {
  const declarations = [];
  const fieldPattern =
    /^\s*(?:pub(?:\s*\([^)]*\))?\s+)?([A-Za-z_]\w*)\s*:/gm;
  for (const range of ranges) {
    const bodyStart = range.start + 1;
    const body = source.slice(bodyStart, Math.max(bodyStart, range.end - 1));
    fieldPattern.lastIndex = 0;
    for (const match of body.matchAll(fieldPattern)) {
      const name = match[1];
      const index =
        bodyStart + (match.index ?? 0) + match[0].indexOf(name);
      declarations.push({
        index,
        text: match[0],
        isStatic: false,
        name,
        structName: range.name,
      });
    }
  }
  return declarations;
}

function privateEngineMatches(masked) {
  return findMatches(masked, /\bcreate_engine\s*\(/g).filter((match) => {
    const prefix = masked.slice(Math.max(0, match.index - 80), match.index);
    return /\bnomi_browser_engine\s*::\s*$/.test(prefix) || !/::\s*$/.test(prefix);
  });
}

function maybeReportGatewayLegacyGate(path, source, masked, report) {
  if (path !== GATEWAY_REGISTRY) return;
  for (const declaration of namedDeclarations(masked)) {
    if (!suspiciousGateIdentifier(declaration.name)) continue;
    report(
      path,
      source,
      masked,
      declaration.index,
      'gateway-global-execution-gate',
      'Gateway must not own a browser/companion/execution mutex or gate; scheduling belongs to BrowserSessionHub',
    );
  }
}

function maybeReportEngineState(path, source, masked, report) {
  if (!isEngineProductionPath(path)) return;
  const structs = structRanges(masked);

  const declarations = [
    ...staticDeclarations(masked),
    ...structFieldDeclarations(masked, structs),
  ];
  for (const declaration of declarations) {
    const normalized = normalizedIdentifier(declaration.name);
    const enclosing =
      declaration.structName
        ? { name: declaration.structName }
        : enclosingStructAt(structs, declaration.index);
    if (
      declaration.isStatic &&
      /^(?:active|current|global)_(?:target|frame)(?:_id)?$/.test(normalized)
    ) {
      report(
        path,
        source,
        masked,
        declaration.index,
        'global-browser-cursor',
        'active target/frame state must be lane-scoped, never static or process-global',
      );
      continue;
    }

    if (
      suspiciousGateIdentifier(declaration.name) &&
      (!enclosing || !isLaneOwnedStruct(enclosing.name))
    ) {
      report(
        path,
        source,
        masked,
        declaration.index,
        'engine-wide-operation-gate',
        'the browser engine must not retain a global/engine-wide browser operation mutex or gate',
      );
    }

    if (
      /^(?:active|current|global)_(?:target|frame)(?:_id)?$/.test(normalized) &&
      (!enclosing || !isLaneOwnedStruct(enclosing.name))
    ) {
      report(
        path,
        source,
        masked,
        declaration.index,
        'engine-wide-active-cursor',
        'active target/frame state must remain inside a lane-owned backend or route',
      );
    }
  }
}

function maybeReportRendererRawCdpSurface(path, source, masked, report) {
  if (!isUiProductionSourcePath(path)) return;

  for (const match of findMatches(
    masked,
    /\b(?:getCdpStatus|updateCdpConfig|ICdpStatus|ICdpConfig)\b/g,
  )) {
    report(
      path,
      source,
      masked,
      match.index,
      'renderer-raw-cdp-api',
      'renderer code must not expose or configure a raw Chromium CDP endpoint',
    );
  }

  // String literals are intentionally masked for Rust ownership checks, but
  // these command-line switches are themselves the renderer exposure. Search
  // the original UI source so copied MCP snippets cannot bypass the boundary.
  for (const match of findMatches(
    source,
    /--(?:browser-url|cdp-endpoint)(?:=|\b)/g,
  )) {
    report(
      path,
      source,
      source,
      match.index,
      'renderer-raw-cdp-config',
      'renderer code must not publish raw CDP connection flags or MCP configuration',
    );
  }
}

function isScannedPath(path) {
  return isOwnershipBoundaryPath(path);
}

function scanEntries(entries) {
  const byPath = new Map(entries.map((entry) => [normalizePath(entry.path), entry]));
  const violations = [];
  const hubConstructors = [];
  const report = (path, source, masked, index, rule, detail) => {
    violations.push({
      path,
      line: lineNumber(source, index),
      rule,
      detail,
      snippet: snippetAt(source, index),
    });
  };

  for (const entry of entries) {
    const path = normalizePath(entry.path);
    const masked = productionMask(entry.source);

    if (path === `${ENGINE_PRODUCTION_PREFIX}attached_browser.rs`
      || (path.startsWith(`${ENGINE_PRODUCTION_PREFIX}attached_browser/`) && isRustSourcePath(path) && !path.endsWith('/tests.rs'))) {
      const production = lexicalMask(productionMask(entry.source, { keepLiterals: true }), { keepLiterals: true });
      for (const match of findMatches(production, /\b(?:CdpBackend|Launched|ChildProcessBuilder|kill_process_tree|enable_auto_attach|run_attach_loop|CloseTargetParams|CloseParams)\b|Browser\.close|Target\.closeTarget/g)) {
        report(path, entry.source, production, match.index, 'attached-browser-connection-only', 'An attached user browser is not owned: do not launch, globally attach, or inherit browser/process/target cleanup');
      }
    }

    if (path.startsWith('apps/desktop/')) {
      for (const match of findMatches(masked, /\.plugin\s*\(\s*tauri_plugin_(?:dialog|notification)\s*::\s*init\s*\(/g)) {
        report(path, entry.source, masked, match.index, 'native-dialog-global-shim', 'Use the scoped native API adapters; global initializers replace standard website APIs');
      }
    }
    if (path.startsWith('apps/desktop/src/browser_surface/') && isRustSourcePath(path)) {
      for (const match of findMatches(masked, /\.devtools\s*\(\s*true\s*\)|\b(?:OpenDevToolsWindow|open_devtools|close_devtools|is_devtools_open)\b/g)) {
        report(path, entry.source, masked, match.index, 'embedded-devtools-unsupported', 'Embedded Browser v2 does not expose F12, Inspect, or a DevTools window; keep host-only protocol transport internal');
      }
      for (const match of findMatches(masked, /\.proxy_url\s*\(|\b(?:ProxyConfig|WebResourceRequestedEventHandler)\b|additional_browser_args\s*\([^)]*proxy-server/g)) {
        report(path, entry.source, masked, match.index, 'embedded-browser-native-network', 'Conversation Browser uses the native system network stack; do not restore an application proxy, IP/port allowlist, or request firewall');
      }
    }
    if (path === 'apps/desktop/src/browser_surface/network_policy.rs') {
      report(path, entry.source, masked, 0, 'embedded-browser-native-network', 'The retired interactive-browser network policy module must not be restored');
    }
    if (path === 'apps/desktop/examples/support/browser_devtools.rs') {
      report(path, entry.source, masked, 0, 'embedded-devtools-unsupported', 'The retired visible-DevTools probe must not be restored');
    }
    if (path === 'apps/desktop/examples/support/browser_composition_probe.rs'
      || path === 'apps/desktop/examples/support/webview2_drag_bindings.rs') {
      report(path, entry.source, masked, 0, 'retired-composition-drag-probe', 'Do not restore the failed Composition/OLE drag experiment; cross-session drag is an explicit unsupported boundary');
    }
    if (path === 'apps/desktop/examples/browser_workspace_smoke.rs') {
      for (const match of findMatches(masked, /\.reparent\s*\(|browser-smoke-popout|composition-drag-(?:target-entry-)?only/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-popout-probe', 'The Windows MVP has no popout/reparent or Composition drag product path');
      }
    }

    if (path.startsWith('crates/backend/nomifun-app/src/') && entry.source.includes('browser.inventory.changed')) {
      report(path, entry.source, entry.source, entry.source.indexOf('browser.inventory.changed'), 'retired-browser-inventory-event', 'The retired Browser inventory has no renderer consumer; use the generic resync event only');
    }

    if (path.startsWith('ui/src/') && !path.includes('.test.') && !path.endsWith('.d.ts')) {
      for (const match of findMatches(entry.source, /\b(?:openDevTools|writeRendererLog|logStream)\b|chrome-devtools/g)) {
        report(path, entry.source, entry.source, match.index, 'retired-renderer-devtools', 'Do not restore F12/DevTools renderer bridges or a Chrome DevTools special-case product path');
      }
    }

    if (path.startsWith('ui/src/renderer/pages/browser/') || RETIRED_BROWSER_UI_FILES.has(path)) {
      report(path, entry.source, masked, 0, 'retired-browser-ui-path', 'Browser v2 lives in the conversation workspace; do not restore the v1 product or adapters');
    }
    if (path.startsWith('crates/agent/nomi-browser/')) {
      report(path, entry.source, masked, 0, 'retired-browser-facade', 'The legacy BrowserTool/managed/visual-fallback crate must not be restored');
    }
    if (path.startsWith('crates/backend/nomifun-browser-platform/src/')) {
      for (const match of findMatches(masked, /\b(?:BrowserSessionHub|BrowserLaneClient|OwnerLeaseService|BrowserLaneScheduler)\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-hub-core', 'Browser Platform contains native Workspace contracts, not the retired Hub/Lane ownership model');
      }
    }
    if (path === 'crates/agent/nomi-config/src/config.rs') {
      for (const match of findMatches(masked, /\b(?:BrowserConfig|browser_data_dir|default_browser_source)\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-config', 'Browser capability and runtime supply must not restore old TOML/profile settings');
      }
    }
    if (path === 'crates/backend/nomifun-app/src/router/browser_login.rs') {
      report(path, entry.source, masked, 0, 'retired-browser-login', 'Sign-in belongs to the real conversation browser, not a compatibility login Lane');
    }
    if (RETIRED_KNOWLEDGE_BROWSER_FILES.has(path)) {
      report(path, entry.source, masked, 0, 'retired-knowledge-browser-fetcher', 'Do not restore the retired Hub-backed knowledge renderer or its private lifecycle tests');
    }
    if (['crates/backend/nomifun-app/src/browser_lane_provider.rs',
      'crates/backend/nomifun-ai-agent/src/factory/browser_lane.rs'].includes(path)) {
      report(path, entry.source, masked, 0, 'retired-headless-provider', 'The old Hub lease issuer and its private cleanup executor are retired');
    }
    if (path === 'crates/backend/nomifun-app/src/browser_resource.rs') {
      report(path, entry.source, masked, 0, 'retired-browser-resource-supply', 'Do not restore the unused legacy packaged-Chrome discovery channel');
    }
    if (path === `${ENGINE_PRODUCTION_PREFIX}acquire.rs`) {
      report(path, entry.source, masked, 0, 'retired-engine-browser-acquisition', 'Browser executable supply belongs to the v2 host, not the retired engine downloader');
    }
    if (path.startsWith(ENGINE_PRODUCTION_PREFIX)) {
      for (const match of findMatches(masked, /\b(?:CdpBackend|CdpHostRuntime|BrowserEngine|LaneOperationGate|LaneEngineConfig|CdpTaskResources|TargetOwnership|TaskTabReservationAuthority|TaskDownloadReservationAuthority|DeferredObjectGroupRelease|ObjectGroupReleaseDispatcher|defer_object_group_release|ReliableEventTaskBudget|ReliableTaskEventReceiver|subscribe_reliable_for_task|TaskSessionAuthority|TaskSessionAdmission|SessionResourceScope|LegacyUnscoped|enable_task_session_quota_routing|claim_task_session_authority|run_attach_loop|close_quota_rejected_attached_target)\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-engine-runtime', 'Use the v2 native/attached/isolated page owners, not the retired standalone Host/Lane executor');
      }
      for (const match of findMatches(masked, /\b(?:ChromeSource|resolve_chrome_path(?:_with_source)?|chrome_source|bundled_dir)\b|\bmod\s+acquire\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-engine-browser-acquisition', 'Do not restore engine browser-source preferences or implicit acquisition');
      }
      for (const match of findMatches(masked, /\b(?:create_engine|EngineConfig|ManagedBrowserHost|StandaloneResourceScope|HostLaneCoordinator|TaskTabReconcileCoordinator|resolve_user_data_dir|launch_semaphore)\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-engine-implicit-owner', 'Use caller-owned runtime admission instead of the retired default engine/profile/Host owner');
      }
    }
    if (path === `${ENGINE_PRODUCTION_PREFIX}display.rs`) {
      report(path, entry.source, masked, 0, 'retired-engine-implicit-owner', 'Native surface availability must not fall back to the old display/headless selector');
    }
    if (RETIRED_ENGINE_FILES.has(path)) {
      report(path, entry.source, masked, 0, 'retired-engine-file', 'Do not restore the retired standalone runtime or its private tests/fixtures');
    }
    if (APPLICATION_PRODUCTION_PREFIXES.some(prefix => path.startsWith(prefix))) {
      const production = productionMask(entry.source, { keepLiterals: true });
      for (const match of findMatches(production, /\b(?:NOMIFUN_BUNDLED_CHROME_DIR|BUNDLED_CHROME_DIR_ENV)\b/g)) {
        report(path, entry.source, production, match.index, 'retired-browser-resource-supply', 'Use the verified v2 runtime supply instead of the retired environment seam');
      }
    }
    if (RETIRED_BROWSER_VAULT_FILES.has(path)) {
      report(path, entry.source, masked, 0, 'retired-browser-vault', 'Do not restore the removed encrypted shared-login repository or its disk-sharing tests');
    }
    if (path.startsWith(ENGINE_PRODUCTION_PREFIX) || path === BROWSER_TOOL || path === BOOTSTRAP) {
      for (const match of findMatches(masked, /\b(?:load_storage_state|save_storage_state|shared_storage_state_path|storage_state_path|VaultError|persistent_login_key|persist_login_coordinator|PersistLoginCoordinator|PersistLoginOutcome|PERSIST_LOGIN_COORDINATORS)\b|\bmod\s+vault\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-vault', 'Browser identity snapshots are in-memory host data, not a shared login vault');
      }
    }
    if (path.startsWith('crates/backend/nomifun-ai-agent/src/')) {
      for (const match of findMatches(masked, /\b(?:BrowserLaneBinding|BrowserOwnerLeaseGuard|browser_lane_client)\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-headless-provider', 'Nomi runtime must not restore old Headless bindings or lease teardown');
      }
      const production = productionMask(entry.source);
      for (const match of findMatches(production, /\b(?:BrowserFetcher|browser_fetcher)\b/g)) {
        report(path, entry.source, production, match.index, 'retired-knowledge-browser-fetcher', 'Knowledge rendering belongs to its typed canonical render-content port, not the old Agent-layer fetcher');
      }
    }
    if (path === 'crates/backend/nomifun-app/src/router/browser_management.rs') {
      report(path, entry.source, masked, 0, 'retired-browser-management', 'Do not restore the v1 management/visibility/resource-policy coordinator');
    }
    if (path === 'crates/backend/nomifun-app/src/services.rs' || path === 'crates/backend/nomifun-ai-agent/src/factory/nomi.rs') {
      const production = productionMask(entry.source, { keepLiterals: true });
      for (const match of findMatches(production, /["'](?:agent\.browserUse(?:\.[^"']*)?|browser\.resourcePolicy)["']/g)) {
        report(path, entry.source, production, match.index, 'retired-browser-preference-read', 'Browser startup/factory code must not read or migrate v1 preferences');
      }
    }
    if (path === 'crates/backend/nomifun-app/src/router/routes.rs') {
      for (const match of findMatches(entry.source, /["']\/api\/browser\//g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-management', 'Browser product routes must be scoped to a conversation');
      }
      for (const match of findMatches(entry.source, /["']\/api\/browser\/login\//g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-login', 'Do not restore the retired browser login endpoints');
      }
    }
    if (path.startsWith('crates/backend/nomifun-ai-agent/src/')) {
      for (const match of findMatches(masked, /\b(?:browser_(?:source|full_power|persistent_login|site_memory|visual_fallback)|persistent_login_key)\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-config-forwarding', 'Do not reintroduce legacy browser preference fields into Agent application configuration');
      }
    }
    if (path.startsWith('crates/backend/nomifun-app/src/')) {
      for (const match of findMatches(masked, /\.set_browser_render_content_port\s*\(/g)) {
        if (!/^\.set_browser_render_content_port\s*\(\s*super::knowledge_browser::KnowledgeBrowserPort::bind\s*\(/.test(masked.slice(match.index))) {
          report(path, entry.source, masked, match.index, 'knowledge-render-kernel-port', 'Knowledge rendering must use the exact non-Agent Kernel Provider port');
        }
      }
      for (const match of findMatches(masked, /\b(?:BrowserRoleRuntime|BoundBrowserRoleInvoker|build_browser_session_hub|with_browser_hub)\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-role-hub', 'Do not restore the unreachable v1 Browser role owner or its legacy Profile/Hub startup');
      }
      const production = productionMask(entry.source);
      for (const match of findMatches(production, /\b(?:load_storage_state|save_storage_state|shared_storage_state_path|with_identity_vault|persisted_identity_seed_coverage)\b/g)) {
        report(path, entry.source, production, match.index, 'retired-browser-vault', 'Browser v2 must not import or persist the old shared login vault');
      }
    }
    if (path === PLATFORM_ADAPTER) {
      for (const match of findMatches(masked, /\b(?:IdentitySnapshotPersister|identity_snapshot_persister|ManagedLanePolicyDecorator|with_lane_policy|with_identity_vault)\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-vault', 'Do not restore the removed shared-vault persistence/decorator hooks');
      }
    }
    if (path === 'crates/backend/nomifun-ai-agent/src/factory/mod.rs') {
      for (const match of findMatches(masked, /\bpub\s+encryption_key\s*:/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-vault', 'Agent factory dependencies must not retain the unused application-wide browser vault key');
      }
    }
    if (path === HUB_COMPOSITION) {
      const production = productionMask(entry.source, { keepLiterals: true });
      for (const match of findMatches(production, /["'](?:browser-data|platform-profiles)["']/g)) {
        report(path, entry.source, production, match.index, 'retired-browser-profile-root', 'Launch and recovery must use the same fresh v2-owned roots, never legacy profiles');
      }
    }
    if (path === 'crates/backend/nomifun-common/src/enums.rs') {
      for (const match of findMatches(masked, /enum\s+PreviewContentType\s*\{[^}]*\bUrl\b/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-preview-type', 'Web pages are not document preview content');
      }
    }
    if (path === 'ui/src/renderer/components/layout/Router.tsx') {
      for (const match of findMatches(entry.source, /path=['"]\/(?:browser|settings\/browser-use)['"]/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-route', 'Do not restore the global Browser page or its settings redirect');
      }
    }
    if (path === 'ui/src/common/config/configKeys.ts') {
      for (const match of findMatches(entry.source, /['"]agent\.browserUse(?:\.[^'"]+)?['"]\s*:/g)) {
        report(path, entry.source, masked, match.index, 'retired-browser-setting', 'Browser v2 must not publish the old browser settings or migration keys');
      }
    }

    if (isScannedPath(path)) {
      for (const match of privateEngineMatches(masked)) {
        report(
          path,
          entry.source,
          masked,
          match.index,
          'private-engine-create',
          'production Browser Platform entry points must not construct a facade-owned engine',
        );
      }

      for (const match of findMatches(masked, browserToolConstructorPattern())) {
        report(
          path,
          entry.source,
          masked,
          match.index,
          'private-browser-tool',
          'production App/Gateway/Agent paths must use a bound BrowserLaneClient',
        );
      }

      // `with_policy` only builds the security facade. Native bootstrap must
      // immediately switch that facade to managed-only below; any other
      // production construction is a private ownership path.
      for (const match of findMatches(masked, browserPolicyConstructorPattern())) {
        if (path !== BOOTSTRAP) {
          report(
            path,
            entry.source,
            masked,
            match.index,
            'private-browser-policy-facade',
            'only Native bootstrap may construct the BrowserTool policy facade',
          );
        }
      }

      for (const match of findMatches(masked, managedHostLaunchPattern())) {
        report(
          path,
          entry.source,
          masked,
          match.index,
          'private-host-launch',
          'only the Browser Platform engine adapter may launch a managed Chromium host',
        );
      }

      for (const match of findMatches(
        masked,
        /\b(?:chromiumoxide|chromiumoxide_cdp)\b/g,
      )) {
        report(
          path,
          entry.source,
          masked,
          match.index,
          'engine-dependency-leak',
          'Chromium/CDP implementation dependencies must stay below the Browser Platform adapter',
        );
      }

      maybeReportGatewayLegacyGate(path, entry.source, masked, report);
    }

    if (path === KNOWLEDGE_BROWSER_COMPOSITION) {
      for (const match of findMatches(
        masked,
        knowledgeBrowserBypassPattern(),
      )) {
        report(
          path,
          entry.source,
          masked,
          match.index,
          'knowledge-browser-provider-bypass',
          'Knowledge rendered sources must use canonical browser.render_content rather than a Hub-backed BrowserFetcher',
        );
      }
    }

    maybeReportEngineState(path, entry.source, masked, report);
    maybeReportRendererRawCdpSurface(
      path,
      entry.source,
      masked,
      report,
    );

    // Hub construction is a composition-root invariant, so inspect every
    // production Rust source file rather than only the app/gateway paths.
    if (isRustSourcePath(path) && (path.startsWith('crates/') || path.startsWith('apps/'))) {
      for (const match of findMatches(masked, hubConstructorPattern())) {
        hubConstructors.push({
          path,
          source: entry.source,
          masked,
          index: match.index,
        });
      }
    }
  }

  for (const constructor of hubConstructors) {
    report(
      constructor.path,
      constructor.source,
      constructor.masked,
      constructor.index,
      'hub-composition-contract',
      'The retired Headless BrowserSessionHub must not be constructed in production',
    );
  }

  const bootstrap = byPath.get(BOOTSTRAP);
  if (!bootstrap) {
    violations.push({
      path: BOOTSTRAP,
      line: 1,
      rule: 'bootstrap-missing',
      detail: 'Native Agent bootstrap source is missing',
      snippet: '',
    });
  } else {
    const masked = productionMask(bootstrap.source);
    const managedConstructors = findMatches(
      masked,
      managedBrowserFacadeConstructorPattern(),
    );
    if (managedConstructors.length !== 0) {
      report(
        BOOTSTRAP,
        bootstrap.source,
        masked,
        managedConstructors[0]?.index ?? 0,
        'bootstrap-managed-constructor-contract',
        'Bootstrap must not construct the retired BrowserTool facade',
      );
    }
    if (findMatches(masked, browserPolicyConstructorPattern()).length > 0) {
      report(
        BOOTSTRAP,
        bootstrap.source,
        masked,
        0,
        'bootstrap-legacy-policy-constructor',
        'Bootstrap must not restore the legacy BrowserTool policy constructor',
      );
    }
    for (const match of findMatches(
      masked,
      standaloneBrowserConstructorPattern(),
    )) {
      report(
        BOOTSTRAP,
        bootstrap.source,
        masked,
        match.index,
        'bootstrap-standalone-constructor',
        'Native bootstrap must not construct a standalone BrowserTool',
      );
    }
    for (const match of findMatches(masked, standaloneProfileAllocationPattern())) {
      report(
        BOOTSTRAP,
        bootstrap.source,
        masked,
        match.index,
        'bootstrap-profile-allocation',
        'Native bootstrap must not allocate or own a browser profile',
      );
    }
  }

  return violations;
}

function assertNoViolation(entries, message) {
  const violations = scanEntries(entries);
  if (violations.length > 0) {
    throw new Error(
      `${message}: ${violations.map((item) => item.rule).join(', ')}`,
    );
  }
}

function assertViolation(entries, rule, message) {
  if (!scanEntries(entries).some((violation) => violation.rule === rule)) {
    throw new Error(message);
  }
}

function selfTest() {
  const baseline = [
    {
      path: BOOTSTRAP,
      source: `
        #[cfg(test)]
        mod tests { fn private() { nomi_browser_engine::create_engine(); } }
        fn production() {
          // Browser capabilities are supplied by native host composition.
        }
      `,
    },
    {
      path: GATEWAY_REGISTRY,
      source: '// BrowserTool::with_policy is intentionally mentioned in docs',
    },
    {
      path: HUB_COMPOSITION,
      source: 'fn service() { /* retired Hub has no production composition */ }',
    },
    {
      path: FRESH_V4_COMPOSITION,
      source: 'fn fresh_v4() { /* no legacy Browser owner */ }',
    },
  ];
  assertNoViolation(baseline, 'baseline unexpectedly violates the Browser Platform boundary');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}backend/cdp.rs`,source:'// old runtime'}),
    'retired-engine-file', 'failed to reject restoration of the retired CDP backend');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}replacement.rs`,source:'struct CdpBackend {}'}),
    'retired-engine-runtime', 'failed to reject relocation of the retired CDP executor');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}transport.rs`,source:'fn defer_object_group_release() {}'}),
    'retired-engine-runtime', 'failed to reject restoring the retired object release dispatcher');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}session.rs`,source:'struct ReliableEventTaskBudget {}'}),
    'retired-engine-runtime', 'failed to reject restoring the retired cross-Host event authority');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}session.rs`,source:'enum SessionResourceScope { LegacyUnscoped }'}),
    'retired-engine-runtime', 'failed to reject restoring legacy task/Lane session routing');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}transport.rs`,source:'fn close_quota_rejected_attached_target() {}'}),
    'retired-engine-runtime', 'failed to reject restoring target closure in generic transport');

  assertViolation(
    baseline.concat({
      path: 'crates/backend/nomifun-gateway/src/browser_registry.rs',
      source: 'fn bad() { nomi_browser_engine::create_engine(config); }',
    }),
    'private-engine-create',
    'failed to reject a Gateway-owned engine construction',
  );
  assertViolation(
    baseline.concat({
      path: 'crates/backend/nomifun-ai-agent/src/factory/nomi.rs',
      source: 'fn bad() { BrowserTool::with_data_dir(path, false); }',
    }),
    'private-browser-tool',
    'failed to reject a factory-owned BrowserTool',
  );
  assertViolation(
    baseline.map((entry) =>
      entry.path === KNOWLEDGE_BROWSER_COMPOSITION
        ? {
            ...entry,
            source:
              `${entry.source}\nfn wire() { knowledge.set_render_fetcher(BrowserFetcher::new()); }`,
          }
        : entry,
    ),
    'knowledge-browser-provider-bypass',
    'failed to reject a Knowledge BrowserFetcher bypass in the application composition root',
  );
  assertViolation(
    baseline.concat({
      path: 'crates/backend/nomifun-ai-agent/src/factory/nomi.rs',
      source: 'fn bad() { BrowserTool::with_policy(&config, false, false, false, None, None, None); }',
    }),
    'private-browser-policy-facade',
    'failed to reject a factory-owned BrowserTool policy facade',
  );
  assertViolation(
    baseline.map((entry) =>
      entry.path === BOOTSTRAP
        ? { ...entry, source: 'fn production() { BrowserTool::new_managed(&config); }' }
        : entry,
    ),
    'bootstrap-managed-constructor-contract',
    'failed to reject the retired bootstrap managed constructor',
  );
  assertViolation(
    baseline.map((entry) =>
      entry.path === BOOTSTRAP
        ? {
            ...entry,
            source: `
              fn production() {
                let browser_tool = BrowserTool::new_standalone(&config);
              }
            `,
          }
        : entry,
    ),
    'bootstrap-standalone-constructor',
    'failed to reject a bootstrap-owned standalone BrowserTool',
  );
  assertViolation(
    baseline.map((entry) =>
      entry.path === BOOTSTRAP
        ? {
            ...entry,
            source: `
              fn production() {
                let browser_tool = BrowserTool::new_managed(&config);
                let profile_dir = allocate_profile_dir(&data_dir);
              }
            `,
          }
        : entry,
    ),
    'bootstrap-profile-allocation',
    'failed to reject bootstrap-owned browser profile allocation',
  );
  assertViolation(
    baseline.concat({
      path: 'crates/backend/nomifun-app/src/desktop.rs',
      source: 'fn second_hub() { BrowserSessionHub::new(); }',
    }),
    'hub-composition-contract',
    'failed to reject a second production BrowserSessionHub constructor',
  );
  assertViolation(
    baseline.concat({
      path: FRESH_V4_COMPOSITION,
      source: `
        fn fresh_v4() {
          BrowserSessionHub::new();
        }
      `,
    }),
    'hub-composition-contract',
    'failed to reject any restored Fresh-v4 BrowserSessionHub constructor',
  );
  assertViolation(
    baseline.concat({
      path: 'ui/src/renderer/components/settings/RawBrowserDebug.tsx',
      source: 'application.getCdpStatus.invoke();',
    }),
    'renderer-raw-cdp-api',
    'failed to reject a renderer raw-CDP status API',
  );
  assertViolation(
    baseline.concat({
      path: 'ui/src/renderer/components/settings/RawBrowserDebug.tsx',
      source:
        'const config = ["--cdp-endpoint", "http://127.0.0.1:9222"];',
    }),
    'renderer-raw-cdp-config',
    'failed to reject renderer-published raw CDP connection flags',
  );
  assertViolation(baseline.concat({path:'ui/src/renderer/pages/browser/index.tsx',source:'export const oldPage = true;'}),
    'retired-browser-ui-path','failed to reject the retired Browser page');
  assertViolation(baseline.concat({path:'ui/src/renderer/components/layout/Router.tsx',source:'<Route path="/settings/browser-use" />'}),
    'retired-browser-route','failed to reject the old browser settings redirect');
  assertViolation(baseline.concat({path:'ui/src/common/config/configKeys.ts',source:'type Keys = { "agent.browserUse": boolean };'}),
    'retired-browser-setting','failed to reject old browser configuration keys');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-app/src/router/routes.rs',source:'route("/api/browser/login/open", handler)'}),
    'retired-browser-login','failed to reject the retired login API');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-common/src/enums.rs',source:'enum PreviewContentType { Markdown, Url }'}),
    'retired-browser-preview-type','failed to reject URL document previews');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-app/src/router/routes.rs',source:'route("/api/browser/display-mode", handler)'}),
    'retired-browser-management','failed to reject the retired global management API');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-ai-agent/src/factory/nomi.rs',source:'fn read() { preferences.get("agent.browserUse.source"); }'}),
    'retired-browser-preference-read','failed to reject restored v1 preference reads');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-ai-agent/src/types.rs',source:'pub struct Config { pub browser_full_power: bool }'}),
    'retired-browser-config-forwarding','failed to reject legacy browser configuration forwarding');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-app/src/router/agent_role_host.rs',source:'struct BrowserRoleRuntime { hub: BrowserSessionHub }'}),
    'retired-browser-role-hub','failed to reject the retired Browser role owner');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-app/src/bootstrap/canonical_host.rs',source:'let hub = build_browser_session_hub(data).await?;'}),
    'retired-browser-role-hub','failed to reject the retired Browser profile startup');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-ai-agent/src/browser_fetcher.rs',source:'// Even an empty retired module is not a replacement.'}),
    'retired-knowledge-browser-fetcher','failed to reject restoration of the retired renderer file');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-ai-agent/src/lib.rs',source:'pub use old_renderer::BrowserFetcher;'}),
    'retired-knowledge-browser-fetcher','failed to reject a renamed legacy renderer export');
  assertViolation(baseline.concat({path:HUB_COMPOSITION,source:'fn boot() { nomi_browser_engine::load_storage_state(path, key); }'}),
    'retired-browser-vault','failed to reject legacy cookie import');
  assertViolation(baseline.concat({path:HUB_COMPOSITION,source:'fn boot() { data.join("browser-data").join("platform-profiles"); }'}),
    'retired-browser-profile-root','failed to reject a legacy profile recovery root');
  assertViolation(baseline.concat({path:PLATFORM_ADAPTER,source:'pub fn with_identity_vault() {}'}),
    'retired-browser-vault','failed to reject the removed vault persistence hook');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-ai-agent/src/types.rs',source:'pub struct Config { pub persistent_login_key: Option<[u8;32]> }'}),
    'retired-browser-config-forwarding','failed to reject forwarding a legacy browser encryption key');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-ai-agent/src/factory/mod.rs',source:'pub struct AgentFactoryDeps { pub encryption_key: [u8;32] }'}),
    'retired-browser-vault','failed to reject retention of the unused application key in Agent factory dependencies');
  assertViolation(baseline.concat({path:'apps/desktop/src/main.rs',source:'fn boot() { builder.plugin(tauri_plugin_dialog::init()); }'}),
    'native-dialog-global-shim','failed to reject global website dialog overrides');
  assertViolation(baseline.concat({path:'apps/desktop/src/main.rs',source:'fn boot() { builder.plugin(tauri_plugin_notification::init()); }'}),
    'native-dialog-global-shim','failed to reject global website notification overrides');
  assertViolation(baseline.concat({path:'apps/desktop/src/browser_surface/host.rs',source:'fn build(builder: Builder) { builder.devtools(true); }'}),
    'embedded-devtools-unsupported','failed to reject enabling embedded DevTools');
  assertViolation(baseline.concat({path:'apps/desktop/src/browser_surface/host.rs',source:'fn open(core: Core) { core.OpenDevToolsWindow(); }'}),
    'embedded-devtools-unsupported','failed to reject a DevTools window entry point');
  assertViolation(baseline.concat({path:'apps/desktop/examples/support/browser_devtools.rs',source:'// retired visible DevTools probe'}),
    'embedded-devtools-unsupported','failed to reject restoration of the DevTools probe');
  assertViolation(baseline.concat({path:'apps/desktop/examples/support/browser_composition_probe.rs',source:'// retired composition probe'}),
    'retired-composition-drag-probe','failed to reject restoration of the Composition drag probe');
  assertViolation(baseline.concat({path:'apps/desktop/examples/browser_workspace_smoke.rs',source:'fn popout(view: View, window: Window) { view.reparent(&window); }'}),
    'retired-browser-popout-probe','failed to reject restoration of browser popout/reparent');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-app/src/router/routes.rs',source:'fn lag() { broadcast("browser.inventory.changed"); }'}),
    'retired-browser-inventory-event','failed to reject restoration of the legacy browser inventory event');
  assertViolation(baseline.concat({path:'ui/src/renderer/components/layout/Layout.tsx',source:'function debug() { ipcBridge.application.openDevTools.invoke(); }'}),
    'retired-renderer-devtools','failed to reject restoration of the renderer DevTools bridge');
  assertViolation(baseline.concat({path:'ui/src/renderer/pages/conversation/Preview/components/viewers/HTMLViewer.tsx',source:'export default function HTMLViewer() {}'}),
    'retired-browser-ui-path','failed to reject restoration of the iframe HTML viewer');
  assertViolation(baseline.concat({path:'apps/desktop/src/browser_surface/host.rs',source:'fn build(builder: Builder, proxy: Url) { builder.proxy_url(proxy); }'}),
    'embedded-browser-native-network','failed to reject an interactive Browser proxy');
  assertViolation(baseline.concat({path:'apps/desktop/src/browser_surface/network_policy.rs',source:'struct NetworkPolicy;'}),
    'embedded-browser-native-network','failed to reject restoration of the interactive Browser network-policy module');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-app/src/services.rs',source:'fn boot() { knowledge.set_browser_render_content_port(Arc::new(DirectEngine)); }'}),
    'knowledge-render-kernel-port','failed to reject direct Knowledge renderer injection');
  assertViolation(baseline.concat({path:HUB_COMPOSITION,source:'fn boot() { BrowserSessionHub::new(); }'}),
    'hub-composition-contract','failed to reject restoring the old primary Hub composition');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-app/src/browser_lane_provider.rs',source:'// old issuer'}),
    'retired-headless-provider','failed to reject restoring the old lease issuer');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-ai-agent/src/factory/browser_lane.rs',source:'// old binding'}),
    'retired-headless-provider','failed to reject restoring the old binding module');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs',source:'struct Runtime { binding: BrowserLaneBinding }'}),
    'retired-headless-provider','failed to reject restoring old runtime binding ownership');
  assertViolation(baseline.concat({path:PLATFORM_ADAPTER,source:'// old adapter'}),
    'retired-browser-facade','failed to reject restoring the retired facade crate');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-browser-platform/src/hub.rs',source:'pub struct BrowserSessionHub {}'}),
    'retired-browser-hub-core','failed to reject restoring the retired Hub implementation');
  assertViolation(baseline.concat({path:'crates/agent/nomi-config/src/config.rs',source:'pub struct BrowserConfig {}'}),
    'retired-browser-config','failed to reject restoring old browser configuration');
  assertViolation(baseline.concat({path:'crates/backend/nomifun-app/src/browser_resource.rs',source:'// old resource supply'}),
    'retired-browser-resource-supply','failed to reject restoring old packaged-Chrome discovery');
  assertViolation(baseline.concat({path:'apps/desktop/src/main.rs',source:'fn boot() { std::env::set_var("NOMIFUN_BUNDLED_CHROME_DIR", path); }'}),
    'retired-browser-resource-supply','failed to reject restoring the unconsumed browser environment variable');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}acquire.rs`,source:'// retired downloader'}),
    'retired-engine-browser-acquisition','failed to reject restoring engine acquisition');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}lib.rs`,source:'pub mod acquire;'}),
    'retired-engine-browser-acquisition','failed to reject restoring acquisition under another file path');
  assertViolation(baseline.concat({path:ENGINE_BACKEND,source:'fn launch() { resolve_chrome_path_with_source(data, None, ChromeSource::Managed); }'}),
    'retired-engine-browser-acquisition','failed to reject restoring implicit CDP executable supply');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}lib.rs`,source:'pub async fn create_engine(config: EngineConfig) {}'}),
    'retired-engine-implicit-owner','failed to reject the retired default engine constructor');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}host.rs`,source:'pub struct StandaloneResourceScope {}'}),
    'retired-engine-implicit-owner','failed to reject implicit standalone task resource issuance');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}display.rs`,source:'// removed display fallback'}),
    'retired-engine-implicit-owner','failed to reject the removed display/headless fallback');
  for (const source of ['fn attach() { conn.enable_auto_attach(); }',
    'struct Attached { owner: Launched }', 'fn stop() { send("Browser.close"); }',
    'fn stop() { send("Target.closeTarget"); }', 'fn stop() { kill_process_tree(child); }']) {
    assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}attached_browser.rs`,source}),
      'attached-browser-connection-only','failed to reject managed-browser ownership in personal-browser attach');
  }
  assertNoViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}attached_browser.rs`,source:
    '// Never use Launched or Browser.close.\nfn disconnect() { connection.shutdown(); }\n#[cfg(test)] fn fixture() { send("Browser.close"); }'}),
    'attach-only ownership comments or isolated fixtures became production violations');
  assertViolation(baseline.concat({path:`${ENGINE_PRODUCTION_PREFIX}attached_browser/tabs.rs`,source:'fn stop() { kill_process_tree(child); }'}),
    'attached-browser-connection-only','failed to reject managed cleanup in an attach-only submodule');
  assertViolation(baseline.concat({path:'crates/agent/nomi-browser-engine/src/vault.rs',source:'// retired file'}),
    'retired-browser-vault','failed to reject the retired vault implementation path');
  assertViolation(baseline.concat({path:BROWSER_TOOL,source:'fn navigate() { save_storage_state(state, path, key); }'}),
    'retired-browser-vault','failed to reject implicit state persistence in BrowserTool');
  assertViolation(baseline.concat({path:BOOTSTRAP,source:'pub fn persistent_login_key() {}'}),
    'retired-browser-vault','failed to reject restoring browser key forwarding in bootstrap');
}

if (process.argv.includes('--self-test')) {
  selfTest();
  console.log('browser platform boundary scanner self-test passed');
  process.exit(0);
}

const violations = scanEntries(
  workspacePaths().map(readEntry).filter((entry) => entry !== null),
);
if (violations.length > 0) {
  for (const violation of violations) {
    console.error(
      `${violation.path}:${violation.line} [${violation.rule}] ${violation.detail}`,
    );
    if (violation.snippet) console.error(`  ${violation.snippet}`);
  }
  console.error(
    `browser platform boundary check failed: ${violations.length} violation(s)`,
  );
  process.exit(1);
}

console.log('browser platform boundary check passed');
