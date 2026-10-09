#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const PLATFORM_PREFIX = 'crates/shared/nomi-process-runtime/src/platform/';
const UNIX_PTY_PATH =
  'crates/shared/nomi-process-runtime/src/platform/unix_pty.rs';
// The user-terminal runtime is outside the Wave A Agent command paths. Pin its
// exact reviewed primitive counts until its own migration wave so any added or
// changed ownership path fails closed.
const REVIEWED_EXTERNAL_OWNERSHIP = new Map([
  [
    'crates/backend/nomifun-terminal/src/pty.rs',
    new Map([
      ['pty-owner', 1],
      ['unix-group-owner', 1],
    ]),
  ],
]);
const HAND_OFF_ALLOWLIST = new Set([
  'apps/desktop/src/browser_surface/macos/native/external_browser.rs',
  'crates/agent/nomi-browser-engine/src/backend/cdp.rs',
  'crates/agent/nomi-computer/src/launch.rs',
  'crates/backend/nomifun-shell/src/opener.rs',
  'crates/shared/nomi-process-runtime/src/command_builder.rs',
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
      '*.rs',
      'crates/shared/nomi-process-runtime/Cargo.toml',
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
    source
      .slice(start, end)
      .replace(/[^\r\n]/g, ' ') +
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

function isTopLevelComparison(source, index) {
  const previous = source[index - 1] ?? '';
  const next = source[index + 1] ?? '';
  return (
    source[index] === '<' &&
    !/[A-Za-z0-9_:'">=]/.test(previous) &&
    next !== '<' &&
    next !== '='
  );
}

function attributedItemEnd(source, index) {
  let cursor = skipSpace(source, index);
  while (source[cursor] === '#') {
    const end = attributeEnd(source, cursor);
    if (end === null) break;
    cursor = skipSpace(source, end);
  }
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
      if (isTopLevelComparison(source, cursor)) {
        // A comparison in a const/static initializer cannot begin an item body.
        continue;
      }
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
  const arguments_ = compact.slice(prefix.length, -suffix.length);
  return splitTopLevelArguments(arguments_).includes('test');
}

function testOnlyRanges(source) {
  const masked = lexicalMask(source);
  const ranges = [];
  let index = 0;
  while (index < masked.length) {
    if (masked[index] !== '#' || masked[index + 1] !== '[') {
      index += 1;
      continue;
    }
    const end = attributeEnd(masked, index);
    if (end === null) break;
    if (isTestOnlyCfgAttribute(masked.slice(index, end))) {
      const itemEnd = attributedItemEnd(masked, end);
      ranges.push([index, itemEnd]);
      index = itemEnd;
    } else {
      index = end;
    }
  }
  return { masked, ranges };
}

function productionMask(source) {
  const { masked, ranges } = testOnlyRanges(source);
  let output = masked;
  for (const [start, end] of ranges) {
    output = replaceNonNewline(output, start, end);
  }
  return output;
}

function productionText(source) {
  const { ranges } = testOnlyRanges(source);
  let output = source;
  for (const [start, end] of ranges) {
    output = replaceNonNewline(output, start, end);
  }
  return output;
}

function lineNumber(source, index) {
  return source.slice(0, index).split('\n').length;
}

function snippetAt(source, index) {
  const start = source.lastIndexOf('\n', index - 1) + 1;
  const next = source.indexOf('\n', index);
  return source.slice(start, next === -1 ? source.length : next).trim();
}

function findMatches(source, pattern) {
  pattern.lastIndex = 0;
  return [...source.matchAll(pattern)].map((match) => ({
    index: match.index ?? 0,
    text: match[0],
  }));
}

function stringLiterals(source) {
  const literals = [];
  let index = 0;
  while (index < source.length) {
    if (source.startsWith('//', index)) {
      index = source.indexOf('\n', index + 2);
      if (index === -1) break;
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
      index = cursor;
      continue;
    }
    const rawEnd = rawStringEnd(source, index);
    if (rawEnd !== null) {
      const openingQuote = source.indexOf('"', index);
      const hashes = source.slice(index, openingQuote).split('#').length - 1;
      literals.push({
        index,
        value: source.slice(openingQuote + 1, rawEnd - 1 - hashes),
      });
      index = rawEnd;
      continue;
    }
    if (source[index] === '"' || source.startsWith('b"', index)) {
      const quote = source[index] === '"' ? index : index + 1;
      const end = quotedEnd(source, quote, '"');
      literals.push({
        index,
        value: source.slice(quote + 1, Math.max(quote + 1, end - 1)),
      });
      index = end;
      continue;
    }
    index += 1;
  }
  return literals;
}

function hasTaskkillTreeCall(source) {
  const literals = stringLiterals(source);
  for (const [index, literal] of literals.entries()) {
    if (!/^taskkill(?:\.exe)?$/i.test(literal.value.trim())) continue;
    if (
      literals
        .slice(index + 1)
        .some(
          (candidate) =>
            candidate.index - literal.index <= 600 &&
            /^\/T$/i.test(candidate.value.trim()),
        )
    ) {
      return literal.index;
    }
  }
  return null;
}

function importedLibcNames(source, original) {
  const names = new Set([original]);
  const escaped = original.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  for (const match of source.matchAll(
    new RegExp(
      String.raw`\buse\s+libc\s*::\s*${escaped}(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*;`,
      'g',
    ),
  )) {
    names.add(match[1] ?? original);
  }
  for (const match of source.matchAll(
    /\buse\s+libc\s*::\s*\{([^}]*)\}\s*;/gs,
  )) {
    for (const item of match[1].split(',')) {
      const alias = item
        .trim()
        .match(
          new RegExp(
            String.raw`^${escaped}(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?$`,
          ),
        );
      if (alias) names.add(alias[1] ?? original);
    }
  }
  if (/\buse\s+libc\s*::\s*\*\s*;/.test(source)) names.add(original);
  return names;
}

function callPattern(names, prefix, suffix) {
  const alternatives = [...names]
    .map((name) => name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
    .join('|');
  return new RegExp(`${prefix}(?:${alternatives})${suffix}`, 'g');
}

function scanEntries(entries) {
  const normalizedEntries = entries.map(({ path, source }) => ({
    path: normalizePath(path),
    source,
  }));
  const violations = [];
  const byPath = new Map(
    normalizedEntries.map((entry) => [entry.path, entry.source]),
  );
  const report = (path, original, scanned, index, rule, detail) => {
    violations.push({
      path,
      line: lineNumber(scanned, index),
      rule,
      detail,
      snippet: snippetAt(original, index),
    });
  };

  for (const { path, source } of normalizedEntries) {
    if (!path.endsWith('.rs')) continue;
    if (path.split('/').includes('tests')) {
      continue;
    }
    const production = productionMask(source);

    const handOffPatterns = [
      /\.(?:hand_off|with_detached|that_detached)\s*\(/g,
      /\bopen\s*::\s*(?:that_detached|with_detached)\s*\(/g,
    ];
    for (const pattern of handOffPatterns) {
      for (const match of findMatches(production, pattern)) {
        if (!HAND_OFF_ALLOWLIST.has(path)) {
          report(
            path,
            source,
            production,
            match.index,
            'hand-off-allowlist',
            'explicit hand-off is permitted only in exact allowlisted files',
          );
        }
      }
    }

    const ownershipViolations = [];
    const collectOwnership = (index, rule, detail) => {
      ownershipViolations.push({ index, rule, detail });
    };
    for (const match of findMatches(production, /\bnative_pty_system\s*\(/g)) {
      if (path !== UNIX_PTY_PATH) {
        collectOwnership(
          match.index,
          'pty-owner',
          `native_pty_system is permitted only in ${UNIX_PTY_PATH}`,
        );
      }
    }

    const kqueueNames = importedLibcNames(production, 'kqueue');
    const killNames = importedLibcNames(production, 'kill');
    const ownershipPatterns = [
      ['windows-job-owner', /\bJOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE\b/g],
      ['linux-parent-death-owner', /\bPR_SET_PDEATHSIG\b/g],
      [
        'macos-parent-death-owner',
        callPattern(
          kqueueNames,
          String.raw`(?:\blibc\s*::\s*|\b)`,
          String.raw`\s*\(`,
        ),
      ],
      [
        'unix-group-owner',
        callPattern(
          killNames,
          String.raw`(?:\blibc\s*::\s*|\b)`,
          String.raw`\s*\(\s*-`,
        ),
      ],
    ];
    if (!path.startsWith(PLATFORM_PREFIX)) {
      for (const [rule, pattern] of ownershipPatterns) {
        for (const match of findMatches(production, pattern)) {
          collectOwnership(
            match.index,
            rule,
            'process ownership primitive must live under nomi-process-runtime/src/platform',
          );
        }
      }
      const taskkillIndex = hasTaskkillTreeCall(productionText(source));
      if (taskkillIndex !== null) {
        collectOwnership(
          taskkillIndex,
          'windows-tree-kill-owner',
          'process ownership primitive must live under nomi-process-runtime/src/platform',
        );
      }
    }
    const reviewed = REVIEWED_EXTERNAL_OWNERSHIP.get(path);
    if (reviewed) {
      const observed = new Map();
      for (const violation of ownershipViolations) {
        observed.set(
          violation.rule,
          (observed.get(violation.rule) ?? 0) + 1,
        );
      }
      const exact =
        observed.size === reviewed.size &&
        [...reviewed].every(
          ([rule, expected]) => observed.get(rule) === expected,
        );
      if (exact) ownershipViolations.length = 0;
    }
    for (const violation of ownershipViolations) {
      report(
        path,
        source,
        production,
        violation.index,
        violation.rule,
        violation.detail,
      );
    }
  }

  const processManifestPath = 'crates/shared/nomi-process-runtime/Cargo.toml';
  const processManifest = byPath.get(processManifestPath) ?? '';
  for (const pattern of [
    /\bnomifun-[\w-]*\b/g,
    /\bnomi-(?:types|agent|tools)\b/g,
    /\b(?:rusqlite|sqlx|tauri)\b/g,
  ]) {
    for (const match of findMatches(processManifest, pattern)) {
      report(
        processManifestPath,
        processManifest,
        processManifest,
        match.index,
        'process-runtime-dependency-boundary',
        'nomi-process-runtime must remain backend-neutral',
      );
    }
  }

  return violations;
}

function workspaceEntries() {
  return workspacePaths().flatMap((path) => {
    const absolute = resolve(ROOT, path);
    // `git ls-files --cached` includes paths deleted in the current hard-cut
    // until the change is committed. A source scanner must inspect the working
    // tree, not fail before it can validate the remaining runtime boundary.
    if (!existsSync(absolute)) return [];
    return [{ path, source: readFileSync(absolute, 'utf8') }];
  });
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
  const base = [
    {
      path: 'crates/shared/nomi-process-runtime/Cargo.toml',
      source: '[dependencies]\ntokio = "1"\n',
    },
  ];
  assertNoViolation(base, 'baseline unexpectedly violates the boundary');

  assertViolation(
    base.concat({
      path: 'crates/new/untracked_adapter.rs',
      source: 'fn launch(command: &mut C) { command.hand_off(); }\n',
    }),
    'hand-off-allowlist',
    'failed to reject hand_off in an arbitrary new path',
  );
  assertViolation(
    base.concat({
      path: 'crates/new/untracked_open.rs',
      source: 'fn launch() { open::that_detached("target"); }\n',
    }),
    'hand-off-allowlist',
    'failed to reject open::that_detached in an arbitrary new path',
  );
  assertNoViolation(
    base.concat({
      path: 'apps/desktop/src/browser_surface/macos/native/external_browser.rs',
      source:
        'fn user_handoff() { open::that_detached("https://example.com"); }\n',
    }),
    'reviewed macOS Browser user hand-off path was not exact-allowlisted',
  );
  assertNoViolation(
    base.concat({
      path: 'crates/backend/nomifun-shell/src/opener.rs',
      source:
        'fn launch() { open::that_detached("target"); command.hand_off(); }\n',
    }),
    'exact hand-off allowlist rejected an approved path',
  );
  assertNoViolation(
    base.concat({
      path: 'crates/agent/nomi-browser-engine/src/backend/cdp.rs',
      source:
        'fn retain_cleanup_authority(cleanup: &Cleanup) { cleanup.hand_off(); }\n',
    }),
    'browser construction cleanup hand-off was not exact-allowlisted',
  );
  assertNoViolation(
    base.concat({
      path: UNIX_PTY_PATH.replaceAll('/', '\\'),
      source: 'fn open() { native_pty_system(); }\n',
    }),
    'path normalization or unix_pty allowance failed',
  );
  assertViolation(
    base.concat({
      path: 'crates/shared/nomi-process-runtime/src/platform/unix.rs',
      source: 'fn open() { native_pty_system(); }\n',
    }),
    'pty-owner',
    'failed to enforce the exact native_pty_system owner',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/nomifun-runtime/src/late.rs',
      source:
        '#[cfg(test)]\nmod tests { fn fake() { libc::kill(-1, 9); } }\nfn production() { libc::kill(-2, 9); }\n',
    }),
    'unix-group-owner',
    'production after a cfg(test) item was not scanned',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/nomifun-runtime/src/unicode.rs',
      source:
        '// 非 ASCII 注释不得改变后续扫描偏移\nfn production() { libc::kill(-2, 9); }\n',
    }),
    'unix-group-owner',
    'Unicode before production code corrupted scanner offsets',
  );
  assertNoViolation(
    base.concat({
      path: 'crates/backend/nomifun-runtime/src/comments.rs',
      source:
        '// command.hand_off(); libc::kill(-1, 9);\nconst TEXT: &str = "open::that_detached native_pty_system";\n#[cfg(test)]\nmod tests { fn fake() { libc::kill(-2, 9); command.hand_off(); } }\n',
    }),
    'comments, strings, or test modules produced a false positive',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/nomifun-runtime/src/cfg_attr.rs',
      source:
        '#[cfg_attr(test, allow(dead_code))]\nfn production() { libc::kill(-2, 9); }\n',
    }),
    'unix-group-owner',
    'cfg_attr(test, ...) incorrectly hid production code',
  );
  assertNoViolation(
    base.concat({
      path: 'crates/backend/nomifun-runtime/src/cfg_all_test.rs',
      source:
        '#[cfg(all(test, target_os = "linux"))]\nmod tests { fn fake() { libc::kill(-2, 9); } }\n',
    }),
    'cfg(all(test, ...)) test code produced a false positive',
  );
  assertNoViolation(
    base.concat({
      path: 'crates/backend/nomifun-runtime/src/cfg_all_test_late.rs',
      source:
        '#[cfg(all(target_os = "linux", test))]\nmod tests { fn fake() { libc::kill(-2, 9); } }\n',
    }),
    'cfg(all(..., test)) test code produced a false positive',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/nomifun-runtime/src/const_comparison.rs',
      source:
        '#[cfg(test)]\nconst TEST_ONLY: bool = 1 < 2;\nfn production() { libc::kill(-2, 9); }\n',
    }),
    'unix-group-owner',
    'comparison in a test-only const swallowed following production code',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/other-runtime/src/imported_kill.rs',
      source:
        'use libc::{kill, SIGKILL};\nfn cleanup(pgid: i32) { unsafe { kill(-pgid, SIGKILL); } }\n',
    }),
    'unix-group-owner',
    'failed to reject an imported negative-PID group kill',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/other-runtime/src/imported_kqueue.rs',
      source:
        'use libc::kqueue as make_queue;\nfn watch() { unsafe { make_queue(); } }\n',
    }),
    'macos-parent-death-owner',
    'failed to reject an imported kqueue watchdog primitive',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/other-runtime/src/taskkill.rs',
      source:
        'fn cleanup() { std::process::Command::new("taskkill").args(["/PID", "7", "/T"]); }\n',
    }),
    'windows-tree-kill-owner',
    'failed to reject taskkill /T hidden in Rust string literals',
  );
  assertNoViolation(
    base.concat({
      path: 'crates/backend/other-runtime/src/taskkill_test.rs',
      source:
        '#[cfg(test)]\nfn cleanup() { std::process::Command::new("taskkill").arg("/T"); }\n',
    }),
    'taskkill /T inside cfg(test) produced a false positive',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/other-runtime/src/imported_kill_alias.rs',
      source:
        'use libc::kill as group_kill;\nfn cleanup(pgid: i32) { unsafe { group_kill(-pgid, 9); } }\n',
    }),
    'unix-group-owner',
    'failed to reject an aliased negative-PID group kill',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/other-runtime/src/imported_kill_glob.rs',
      source:
        'use libc::*;\nfn cleanup(pgid: i32) { unsafe { kill(-pgid, SIGKILL); } }\n',
    }),
    'unix-group-owner',
    'failed to reject a glob-imported negative-PID group kill',
  );
  assertNoViolation(
    base.concat({
      path: 'crates/shared/nomi-process-runtime/tests/ownership_fixture.rs',
      source:
        'fn cleanup(pgid: i32) { unsafe { libc::kill(-pgid, 9); } }\n',
    }),
    'integration-test ownership fixture produced a production false positive',
  );
  assertNoViolation(
    base.concat({
      path: 'crates/backend/nomifun-terminal/src/pty.rs',
      source:
        'fn open() { native_pty_system(); }\nfn cleanup(pgid: i32) { unsafe { libc::kill(-pgid, 9); } }\n',
    }),
    'reviewed external ownership signatures were not accepted',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/nomifun-terminal/src/pty.rs',
      source:
        'fn open() { native_pty_system(); }\nfn cleanup(pgid: i32) { unsafe { libc::kill(-pgid, 9); libc::kill(-pgid, 9); } }\n',
    }),
    'unix-group-owner',
    'reviewed external ownership count changed without failing closed',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/nomifun-terminal/src/pty.rs',
      source:
        'fn open() { native_pty_system(); }\nfn cleanup(pgid: i32) { unsafe { libc::kill(-pgid, 9); } command.hand_off(); }\n',
    }),
    'hand-off-allowlist',
    'reviewed ownership exception suppressed another boundary rule',
  );
  assertViolation(
    base.concat({
      path: 'crates/backend/nomifun-terminal/src/new_owner.rs',
      source:
        'fn open() { native_pty_system(); }\nfn cleanup(pgid: i32) { unsafe { libc::kill(-pgid, 9); } }\n',
    }),
    'unix-group-owner',
    'reviewed external runtime exception widened beyond its exact file',
  );
}

if (process.argv.includes('--self-test')) {
  selfTest();
  console.log('process runtime boundary scanner self-test passed');
  process.exit(0);
}

const violations = scanEntries(workspaceEntries());
if (violations.length > 0) {
  for (const violation of violations) {
    console.error(
      `${violation.path}:${violation.line} [${violation.rule}] ${violation.detail}`,
    );
    if (violation.snippet) console.error(`  ${violation.snippet}`);
  }
  console.error(
    `process runtime boundary check failed: ${violations.length} violation(s)`,
  );
  process.exit(1);
}

console.log('process runtime boundary check passed');
