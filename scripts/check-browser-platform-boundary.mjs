#!/usr/bin/env node

/**
 * Enforce the Browser Platform ownership boundary.
 *
 * Production application/agent entry points may only dispatch through the
 * process-wide BrowserSessionHub.  The low-level browser engine and facade
 * retain standalone compatibility helpers for tests and explicit embeddings,
 * but those helpers must not leak into App, Gateway, or Agent factory
 * production paths.
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
const BROWSER_STDIO_BRIDGE =
  'crates/backend/nomifun-app/src/commands/browser_stdio.rs';
const GATEWAY_REGISTRY = 'crates/backend/nomifun-gateway/src/browser_registry.rs';
const HUB_COMPOSITION = 'crates/backend/nomifun-app/src/services.rs';

const BRIDGE_RESOURCE_SYMBOLS = [
  [
    'bridge-current-exe',
    /\bcurrent_exe\b/gi,
    'the browser stdio bridge must not resolve resources from the current executable',
  ],
  [
    'bridge-bundled-chrome-resource',
    /(?:chrome-for-testing|\bbundled_chrome(?:_dir)?\b)/gi,
    'the browser stdio bridge must not mention the bundled Chrome resource',
  ],
  [
    'bridge-profile-resource',
    /\b\w*profile\w*\b/gi,
    'the browser stdio bridge must not contain browser profile symbols',
  ],
  [
    'bridge-user-data-resource',
    /\b\w*user_data\w*\b/gi,
    'the browser stdio bridge must not contain Chromium user-data symbols',
  ],
  [
    'bridge-cdp-symbol',
    /\b\w*cdp\w*\b/gi,
    'the browser stdio bridge must not expose CDP state or symbols',
  ],
  [
    'bridge-launch-symbol',
    /\b\w*launch\w*\b/gi,
    'the browser stdio bridge must not contain browser launch symbols',
  ],
];

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
function lexicalMask(source) {
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
      output = replaceNonNewline(output, index, rawEnd);
      index = rawEnd;
      continue;
    }
    if (source[index] === '"' || source.startsWith('b"', index)) {
      const quote = source[index] === '"' ? index : index + 1;
      const end = quotedEnd(source, quote, '"');
      output = replaceNonNewline(output, index, end);
      index = end;
      continue;
    }
    if (
      source[index] === "'" &&
      (index === 0 || !isIdent(source.charCodeAt(index - 1)))
    ) {
      const end = charLiteralEnd(source, index);
      if (end !== null) {
        output = replaceNonNewline(output, index, end);
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

function productionMask(source) {
  const masked = lexicalMask(source);
  let output = masked;
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

function managedBrowserToolConstructorPattern() {
  return /\b(?:[A-Za-z_]\w*\s*::\s*)*BrowserTool\s*::\s*with_managed_engine\s*(?:::<[^;{}()]*>\s*)?\(/g;
}

function managedBrowserFacadeConstructorPattern() {
  return /\b(?:[A-Za-z_]\w*\s*::\s*)*BrowserTool\s*::\s*new_managed\s*(?:::<[^;{}()]*>\s*)?\(/g;
}

function functionBody(source, name) {
  const signaturePattern = new RegExp(`\\bfn\\s+${name}\\s*\\(`, 'g');
  const signature = signaturePattern.exec(source);
  if (!signature) return null;
  const open = source.indexOf('{', signature.index);
  if (open < 0) return null;
  const end = matchingBrace(source, open);
  return {
    signatureIndex: signature.index,
    open,
    end,
    body: source.slice(open + 1, Math.max(open + 1, end - 1)),
  };
}

function standaloneBrowserConstructorPattern() {
  return /\b(?:Self|BrowserTool)\s*::\s*(?:new|new_standalone|with_data_dir)\s*(?:::<[^;{}()]*>\s*)?\(/g;
}

function browserProfileAllocationPattern() {
  return /\b(?:allocate_profile_dir\s*\(|profile_dir\s*:)/g;
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

  if (hubConstructors.length === 0) {
    violations.push({
      path: HUB_COMPOSITION,
      line: 1,
      rule: 'hub-composition-contract',
      detail: `production must construct exactly one BrowserSessionHub in ${HUB_COMPOSITION} (found 0)`,
      snippet: '',
    });
  } else {
    for (const constructor of hubConstructors) {
      if (
        hubConstructors.length === 1 &&
        constructor.path === HUB_COMPOSITION
      ) {
        continue;
      }
      report(
        constructor.path,
        constructor.source,
        constructor.masked,
        constructor.index,
        'hub-composition-contract',
        `production must construct exactly one BrowserSessionHub in ${HUB_COMPOSITION} (found ${hubConstructors.length})`,
      );
    }
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
    if (managedConstructors.length !== 1) {
      report(
        BOOTSTRAP,
        bootstrap.source,
        masked,
        managedConstructors[0]?.index ?? 0,
        'bootstrap-managed-constructor-contract',
        `Native bootstrap must construct exactly one managed BrowserTool facade (found ${managedConstructors.length})`,
      );
    }
    if (findMatches(masked, browserPolicyConstructorPattern()).length > 0) {
      report(
        BOOTSTRAP,
        bootstrap.source,
        masked,
        0,
        'bootstrap-legacy-policy-constructor',
        'Native bootstrap must use BrowserTool::new_managed rather than the standalone policy constructor',
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

  const adapter = byPath.get(PLATFORM_ADAPTER);
  if (!adapter) {
    violations.push({
      path: PLATFORM_ADAPTER,
      line: 1,
      rule: 'adapter-missing',
      detail: 'Managed Browser engine adapter source is missing',
      snippet: '',
    });
  } else {
    const masked = productionMask(adapter.source);
    const launches = findMatches(masked, managedHostLaunchPattern());
    if (launches.length !== 1) {
      report(
        PLATFORM_ADAPTER,
        adapter.source,
        masked,
        launches[0]?.index ?? 0,
        'adapter-launch-contract',
        `the adapter must contain exactly one ManagedBrowserHost managed launch call (found ${launches.length})`,
      );
    }

    const managedConstructors = findMatches(
      masked,
      managedBrowserToolConstructorPattern(),
    );
    if (managedConstructors.length !== 1) {
      report(
        PLATFORM_ADAPTER,
        adapter.source,
        masked,
        managedConstructors[0]?.index ?? 0,
        'adapter-managed-tool-contract',
        `the managed adapter must construct exactly one BrowserTool through with_managed_engine (found ${managedConstructors.length})`,
      );
    }

    for (const match of findMatches(
      masked,
      standaloneBrowserConstructorPattern(),
    )) {
      report(
        PLATFORM_ADAPTER,
        adapter.source,
        masked,
        match.index,
        'adapter-standalone-constructor',
        'the managed Browser Platform adapter must not construct a standalone BrowserTool',
      );
    }

    for (const match of findMatches(masked, standaloneProfileAllocationPattern())) {
      report(
        PLATFORM_ADAPTER,
        adapter.source,
        masked,
        match.index,
        'adapter-profile-allocation',
        'the managed Browser Platform adapter must not allocate or own a facade profile',
      );
    }
  }

  const browserTool = byPath.get(BROWSER_TOOL);
  if (!browserTool) {
    violations.push({
      path: BROWSER_TOOL,
      line: 1,
      rule: 'managed-policy-facade-missing',
      detail: 'BrowserTool managed policy facade source is missing',
      snippet: '',
    });
  } else {
    const masked = productionMask(browserTool.source);
    const managedFactory = functionBody(masked, 'new_managed');
    const managedEngineFactory = functionBody(masked, 'with_managed_engine');
    if (!managedFactory) {
      report(
        BROWSER_TOOL,
        browserTool.source,
        masked,
        0,
        'managed-policy-constructor-missing',
        'BrowserTool must expose a managed constructor that does not allocate standalone browser state',
      );
    } else {
      if (!/\bmanaged_only\s*:\s*true\s*,/.test(managedFactory.body)) {
        report(
          BROWSER_TOOL,
          browserTool.source,
          masked,
          managedFactory.signatureIndex,
          'managed-policy-private-fallback',
          'new_managed must mark its facade managed-only so it can never launch private Chromium',
        );
      }
      for (const match of findMatches(
        managedFactory.body,
        standaloneBrowserConstructorPattern(),
      )) {
        report(
          BROWSER_TOOL,
          browserTool.source,
          masked,
          managedFactory.open + 1 + match.index,
          'managed-policy-standalone-constructor',
          'new_managed must not delegate to a standalone BrowserTool constructor',
        );
      }
      for (const match of findMatches(
        managedFactory.body,
        standaloneProfileAllocationPattern(),
      )) {
        report(
          BROWSER_TOOL,
          browserTool.source,
          masked,
          managedFactory.open + 1 + match.index,
          'managed-policy-profile-allocation',
          'new_managed must not allocate or own a facade profile; the managed host owns profiles',
        );
      }
    }

    if (!managedEngineFactory) {
      report(
        BROWSER_TOOL,
        browserTool.source,
        masked,
        0,
        'managed-policy-factory-missing',
        'BrowserTool must expose a managed-engine policy factory for the Browser Platform adapter',
      );
    } else {
      const managedDelegates = findMatches(
        managedEngineFactory.body,
        /\b(?:Self|BrowserTool)\s*::\s*new_managed\s*(?:::<[^;{}()]*>\s*)?\(/g,
      );
      if (managedDelegates.length !== 1) {
        report(
          BROWSER_TOOL,
          browserTool.source,
          masked,
          managedEngineFactory.signatureIndex,
          'managed-policy-factory-contract',
          `with_managed_engine must delegate exactly once to new_managed (found ${managedDelegates.length})`,
        );
      }
    }
  }

  const bridge = byPath.get(BROWSER_STDIO_BRIDGE);
  if (!bridge) {
    violations.push({
      path: BROWSER_STDIO_BRIDGE,
      line: 1,
      rule: 'browser-stdio-bridge-missing',
      detail: 'browser stdio bridge source is missing',
      snippet: '',
    });
  } else {
    const masked = productionMask(bridge.source);
    for (const [rule, pattern, detail] of BRIDGE_RESOURCE_SYMBOLS) {
      // These terms are forbidden from the bridge entirely, including
      // literals and documentation. Resource discovery belongs to App
      // composition and the bridge should describe only proxy behavior.
      for (const match of findMatches(bridge.source, pattern)) {
        report(
          BROWSER_STDIO_BRIDGE,
          bridge.source,
          bridge.source,
          match.index,
          rule,
          detail,
        );
      }
    }
    if (!/\bScopedBridgeClient\b/.test(masked)) {
      report(
        BROWSER_STDIO_BRIDGE,
        bridge.source,
        masked,
        0,
        'bridge-not-proxy',
        'the browser stdio bridge must remain a scoped capability proxy',
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
          let browser_tool = BrowserTool::new_managed(&config);
        }
      `,
    },
    {
      path: PLATFORM_ADAPTER,
      source: `
        ManagedBrowserHost::launch_platform_managed_with_cleanup_lease(config, cleanup_lease).await?;
        BrowserTool::with_managed_engine(engine);
      `,
    },
    {
      path: BROWSER_TOOL,
      source: `
        impl BrowserTool {
          fn new_managed() {
            Self {
              profile_dir: PathBuf::new(),
              managed_only: true,
            }
          }
          fn with_managed_engine() {
            let mut tool = Self::new_managed();
          }
        }
      `,
    },
    {
      path: BROWSER_STDIO_BRIDGE,
      source: 'struct Bridge { client: ScopedBridgeClient<Scope> }',
    },
    {
      path: GATEWAY_REGISTRY,
      source: '// BrowserTool::with_policy is intentionally mentioned in docs',
    },
    {
      path: HUB_COMPOSITION,
      source: 'fn service() { BrowserSessionHub::new(); }',
    },
  ];
  assertNoViolation(baseline, 'baseline unexpectedly violates the Browser Platform boundary');

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
    baseline.concat({
      path: 'crates/backend/nomifun-ai-agent/src/factory/nomi.rs',
      source: 'fn bad() { BrowserTool::with_policy(&config, false, false, false, None, None, None); }',
    }),
    'private-browser-policy-facade',
    'failed to reject a factory-owned BrowserTool policy facade',
  );
  assertViolation(
    baseline.concat({
      path: 'crates/backend/nomifun-app/src/commands/browser_stdio.rs',
      source: 'fn bad() { ManagedBrowserHost::launch(config).await?; }',
    }),
    'private-host-launch',
    'failed to reject a bridge-owned Chromium host launch',
  );
  assertViolation(
    baseline.map((entry) =>
      entry.path === BOOTSTRAP
        ? { ...entry, source: 'fn production() { BrowserTool::new(&config); }' }
        : entry,
    ),
    'bootstrap-managed-constructor-contract',
    'failed to enforce the bootstrap managed-constructor contract',
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
    baseline.map((entry) =>
      entry.path === PLATFORM_ADAPTER
        ? {
            ...entry,
            source: `
              ManagedBrowserHost::launch_platform_managed(config).await?;
              BrowserTool::with_data_dir(data_dir, false);
            `,
          }
        : entry,
    ),
    'adapter-standalone-constructor',
    'failed to reject a managed adapter standalone BrowserTool constructor',
  );
  assertViolation(
    baseline.map((entry) =>
      entry.path === PLATFORM_ADAPTER
        ? {
            ...entry,
            source: `
              ManagedBrowserHost::launch_platform_managed(config).await?;
              BrowserTool::with_managed_engine(engine);
              let profile_dir = allocate_profile_dir(&data_dir);
            `,
          }
        : entry,
    ),
    'adapter-profile-allocation',
    'failed to reject managed adapter browser profile allocation',
  );
  assertViolation(
    baseline.map((entry) =>
      entry.path === BROWSER_STDIO_BRIDGE
        ? { ...entry, source: 'struct Bridge;' }
        : entry,
    ),
    'bridge-not-proxy',
    'failed to enforce the browser stdio bridge proxy contract',
  );
  const bridgeForbiddenSamples = [
    ['bridge-current-exe', 'fn bad() { std::env::current_exe(); }'],
    ['bridge-bundled-chrome-resource', 'let resource = "chrome-for-testing";'],
    ['bridge-bundled-chrome-resource', 'fn bundled_chrome_dir() {}'],
    ['bridge-profile-resource', 'let profile_dir = path;'],
    ['bridge-user-data-resource', 'let user_data = path;'],
    ['bridge-cdp-symbol', 'let cdp_endpoint = value;'],
    ['bridge-launch-symbol', 'fn launch() {}'],
  ];
  for (const [rule, forbiddenSource] of bridgeForbiddenSamples) {
    assertViolation(
      baseline.map((entry) =>
        entry.path === BROWSER_STDIO_BRIDGE
          ? {
              ...entry,
              source: `struct Bridge { client: ScopedBridgeClient<Scope> }\n${forbiddenSource}`,
            }
          : entry,
      ),
      rule,
      `failed to reject bridge resource/launch symbol: ${rule}`,
    );
  }
  assertViolation(
    baseline.concat({
      path: 'crates/backend/nomifun-app/src/desktop.rs',
      source: 'fn second_hub() { BrowserSessionHub::new(); }',
    }),
    'hub-composition-contract',
    'failed to reject a second production BrowserSessionHub constructor',
  );
  assertViolation(
    baseline.map((entry) =>
      entry.path === BROWSER_TOOL
        ? {
            ...entry,
            source: `
              impl BrowserTool {
                fn new_managed() {
                  Self {
                    profile_dir: PathBuf::new(),
                    managed_only: false,
                  }
                }
                fn with_managed_engine() { let tool = Self::new_managed(); }
              }
            `,
          }
        : entry,
    ),
    'managed-policy-private-fallback',
    'failed to enforce managed-only BrowserTool policy facades',
  );
  assertViolation(
    baseline.map((entry) =>
      entry.path === BROWSER_TOOL
        ? {
            ...entry,
            source: `
              impl BrowserTool {
                fn new_managed() {
                  let profile_dir = allocate_profile_dir(&data_dir);
                  Self { profile_dir, managed_only: true }
                }
                fn with_managed_engine() { let tool = Self::new_managed(); }
              }
            `,
          }
        : entry,
    ),
    'managed-policy-profile-allocation',
    'failed to reject profile allocation inside BrowserTool::new_managed',
  );
  assertViolation(
    baseline.map((entry) =>
      entry.path === BROWSER_TOOL
        ? {
            ...entry,
            source: `
              impl BrowserTool {
                fn new_managed() {
                  Self {
                    profile_dir: PathBuf::new(),
                    managed_only: true,
                  }
                }
                fn with_managed_engine() { let tool = Self::with_data_dir(); }
              }
            `,
          }
        : entry,
    ),
    'managed-policy-factory-contract',
    'failed to require with_managed_engine to delegate to new_managed',
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
