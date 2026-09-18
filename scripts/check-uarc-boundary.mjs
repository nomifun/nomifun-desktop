#!/usr/bin/env node

/**
 * UARC legacy-reachability and ownership baseline.
 *
 * The normal gate allows an inventoried legacy surface to shrink in place, but
 * rejects count growth, same-size movement into other files, missing ownership,
 * stale task IDs, or a malformed platform/timing inventory. `--completion` additionally requires
 * all legacy groups and platform gaps to be closed.
 */

import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, extname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const INVENTORY_PATH =
  'docs/specs/2026-09-16-unified-agent-overhaul/UARC-001-INVENTORY.json';
const MANIFEST_PATH =
  'docs/specs/2026-09-16-unified-agent-overhaul/TASK-MANIFEST.json';
const SOURCE_EXTENSIONS = new Set(['.json', '.rs', '.sql', '.toml', '.ts', '.tsx']);

const normalizePath = (path) => path.replaceAll('\\', '/');
const sortedUnique = (values) => [...new Set(values)].sort();
const digestValues = (values) => createHash('sha256').update(values.join('\n')).digest('hex');

function isRustIdent(byte) {
  return (
    (byte >= 48 && byte <= 57) ||
    (byte >= 65 && byte <= 90) ||
    (byte >= 97 && byte <= 122) ||
    byte === 95
  );
}

function replaceNonNewline(source, start, end) {
  return source.slice(0, start) + source.slice(start, end).replace(/[^\r\n]/g, ' ') + source.slice(end);
}

function rustRawStringEnd(source, index) {
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

function rustQuotedEnd(source, index, quote) {
  let cursor = index + 1;
  while (cursor < source.length) {
    if (source[cursor] === '\\') cursor += 2;
    else if (source[cursor] === quote) return cursor + 1;
    else cursor += 1;
  }
  return source.length;
}

function rustCharLiteralEnd(source, index) {
  const end = rustQuotedEnd(source, index, "'");
  if (end >= source.length || source[end - 1] !== "'") return null;
  const body = source.slice(index + 1, end - 1);
  return body.length === 1 || /^\\(?:[nrt0'"\\]|x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\})$/.test(body)
    ? end
    : null;
}

function rustLexicalMask(source) {
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
        if (source.startsWith('/*', cursor)) { depth += 1; cursor += 2; }
        else if (source.startsWith('*/', cursor)) { depth -= 1; cursor += 2; }
        else cursor += 1;
      }
      output = replaceNonNewline(output, index, cursor);
      index = cursor;
      continue;
    }
    const rawEnd = rustRawStringEnd(source, index);
    if (rawEnd !== null) {
      output = replaceNonNewline(output, index, rawEnd);
      index = rawEnd;
      continue;
    }
    if (source[index] === '"' || source.startsWith('b"', index)) {
      const quote = source[index] === '"' ? index : index + 1;
      const end = rustQuotedEnd(source, quote, '"');
      output = replaceNonNewline(output, index, end);
      index = end;
      continue;
    }
    if (source[index] === "'" && (index === 0 || !isRustIdent(source.charCodeAt(index - 1)))) {
      const end = rustCharLiteralEnd(source, index);
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

function skipRustSpace(source, index) {
  while (index < source.length && /\s/.test(source[index])) index += 1;
  return index;
}

function rustAttributeEnd(source, index) {
  if (source[index] !== '#' || source[index + 1] !== '[') return null;
  let depth = 1;
  for (let cursor = index + 2; cursor < source.length; cursor += 1) {
    if (source[cursor] === '[') depth += 1;
    if (source[cursor] === ']' && --depth === 0) return cursor + 1;
  }
  return source.length;
}

function rustMatchingBrace(source, open) {
  let depth = 1;
  for (let cursor = open + 1; cursor < source.length; cursor += 1) {
    if (source[cursor] === '{') depth += 1;
    if (source[cursor] === '}' && --depth === 0) return cursor + 1;
  }
  return source.length;
}

function rustAttributedItemEnd(source, index) {
  let cursor = skipRustSpace(source, index);
  while (source[cursor] === '#') {
    const end = rustAttributeEnd(source, cursor);
    if (end === null) break;
    cursor = skipRustSpace(source, end);
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
      if (char === '{') return rustMatchingBrace(source, cursor);
    }
  }
  return source.length;
}

function rustTestOnlyAttribute(attribute) {
  const compact = attribute.replace(/\s/g, '');
  return compact === '#[cfg(test)]' || (compact.startsWith('#[cfg(all(') && compact.endsWith('))]') &&
    compact.slice(10, -3).split(',').includes('test'));
}

export function rustProductionText(source) {
  const masked = rustLexicalMask(source);
  let output = source;
  let index = 0;
  while (index < masked.length) {
    if (masked[index] !== '#' || masked[index + 1] !== '[') { index += 1; continue; }
    const end = rustAttributeEnd(masked, index);
    if (end === null) break;
    if (rustTestOnlyAttribute(masked.slice(index, end))) {
      const itemEnd = rustAttributedItemEnd(masked, end);
      output = replaceNonNewline(output, index, itemEnd);
      index = itemEnd;
    } else index = end;
  }
  return output;
}

function invariant(condition, message) {
  if (!condition) throw new Error(message);
}

function readJson(path) {
  return JSON.parse(readFileSync(resolve(ROOT, path), 'utf8'));
}

function workspacePaths() {
  const output = execFileSync(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard', '-z', '--', 'apps', 'crates', 'ui/src'],
    { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  );
  return sortedUnique(output.split('\0').filter(Boolean).map(normalizePath));
}

function isProductionPath(path, scope) {
  if (!scope.roots.some((root) => path === root || path.startsWith(`${root}/`))) return false;
  if (!SOURCE_EXTENSIONS.has(extname(path))) return false;
  if (scope.excluded_prefixes.some((prefix) => path.startsWith(prefix))) return false;
  if (scope.excluded_fragments.some((fragment) => path.includes(fragment))) return false;
  if (/\.(?:test|spec)\.[^.]+$/.test(path)) return false;
  if (/(?:^|\/)(?:tests?|test_support)\.rs$/.test(path)) return false;
  if (/(?:^|\/)[^/]+_(?:test|tests)\.rs$/.test(path)) return false;
  return true;
}

function selectedPaths(paths, matcher) {
  const includes = matcher.include_paths ?? [];
  const excludes = matcher.exclude_paths ?? [];
  return paths.filter((path) => {
    const included = includes.length === 0 || includes.some((prefix) =>
      prefix.endsWith('/') ? path.startsWith(prefix) : path === prefix || path.startsWith(`${prefix}/`));
    const excluded = excludes.some((prefix) =>
      prefix.endsWith('/') ? path.startsWith(prefix) : path === prefix || path.startsWith(`${prefix}/`));
    return included && !excluded;
  });
}

function lineNumber(source, index) {
  return source.slice(0, index).split('\n').length;
}

function collectLiteralMatches(source, terms) {
  const matches = [];
  for (const term of terms) {
    let cursor = 0;
    while (cursor <= source.length - term.length) {
      const index = source.indexOf(term, cursor);
      if (index < 0) break;
      matches.push({ term, line: lineNumber(source, index) });
      cursor = index + Math.max(1, term.length);
    }
  }
  return matches;
}

function collectIdentifierMatches(source, terms) {
  const matches = [];
  const identifierCharacter = /[A-Za-z0-9._\/:\-]/;
  for (const term of terms) {
    let cursor = 0;
    while (cursor <= source.length - term.length) {
      const index = source.indexOf(term, cursor);
      if (index < 0) break;
      const before = index > 0 ? source[index - 1] : '';
      const after = index + term.length < source.length ? source[index + term.length] : '';
      if ((!before || !identifierCharacter.test(before)) && (!after || !identifierCharacter.test(after))) {
        matches.push({ term, line: lineNumber(source, index) });
      }
      cursor = index + Math.max(1, term.length);
    }
  }
  return matches;
}

function collectRegexMatches(source, matcher) {
  const flags = matcher.flags?.includes('g') ? matcher.flags : `${matcher.flags ?? ''}g`;
  const pattern = new RegExp(matcher.pattern, flags);
  return [...source.matchAll(pattern)].map((match) => ({
    term: match[0],
    line: lineNumber(source, match.index ?? 0),
  }));
}

function extractCapabilityIds(catalogPath) {
  const catalog = readJson(catalogPath);
  if (Array.isArray(catalog.retired_capability_ids)) {
    invariant(catalog.inventory_kind === 'uarc-retired-capability-ids', `${catalogPath}: unexpected retirement inventory`);
    invariant(catalog.canonical_module_id_reuse?.some((entry) => entry.id === 'workspace.artifacts'),
      `${catalogPath}: canonical workspace.artifacts reuse must remain explicit`);
    return sortedUnique(catalog.retired_capability_ids);
  }
  invariant(Array.isArray(catalog.packages), `${catalogPath}: packages must be an array`);
  return sortedUnique(catalog.packages.flatMap((entry) =>
    (entry.capabilities ?? []).map((capability) => capability?.capability?.id).filter(Boolean)));
}

function scanTextGroup(group, paths) {
  const candidates = selectedPaths(paths, group.matcher);
  const records = [];
  const terms = group.matcher.kind === 'catalog_ids'
    ? extractCapabilityIds(group.matcher.catalog_path)
    : group.matcher.terms;
  for (const path of candidates) {
    const absolute = resolve(ROOT, path);
    if (!existsSync(absolute)) continue;
    const rawSource = readFileSync(absolute, 'utf8');
    const source = extname(path) === '.rs' ? rustProductionText(rawSource) : rawSource;
    const matches = group.matcher.kind === 'regex'
      ? collectRegexMatches(source, group.matcher)
      : group.matcher.kind === 'catalog_ids'
        ? collectIdentifierMatches(source, terms)
        : collectLiteralMatches(source, terms);
    if (matches.length > 0) records.push({ path, matches });
  }
  return {
    match_count: records.reduce((sum, record) => sum + record.matches.length, 0),
    file_count: records.length,
    files: records.map((record) => record.path),
    catalog_item_count: group.matcher.kind === 'catalog_ids' ? terms.length : undefined,
    records,
  };
}

function scanPathGroup(group, paths) {
  const files = sortedUnique(group.matcher.paths.flatMap((entry) => {
    const normalized = normalizePath(entry);
    if (normalized.endsWith('/')) {
      return paths.filter((path) => path.startsWith(normalized) && existsSync(resolve(ROOT, path)));
    }
    return paths.filter((path) =>
      (path === normalized || path.startsWith(`${normalized}/`)) && existsSync(resolve(ROOT, path)));
  }));
  return {
    match_count: files.length,
    file_count: files.length,
    files,
    records: files.map((path) => ({ path, matches: [{ term: '<path>', line: 1 }] })),
  };
}

function scanGroup(group, productionPaths) {
  const result = group.matcher.kind === 'paths'
    ? scanPathGroup(group, productionPaths)
    : scanTextGroup(group, productionPaths);
  return { ...result, files_digest: digestValues(result.files) };
}

function validateInventorySchema(inventory, manifest) {
  invariant(inventory.schema_version === '1.0.0', 'inventory schema_version must be 1.0.0');
  invariant(inventory.inventory_kind === 'uarc-baseline-inventory', 'unexpected inventory_kind');
  invariant(inventory.source?.barrier_commit, 'source.barrier_commit is required');
  invariant(Array.isArray(inventory.legacy_reachability), 'legacy_reachability must be an array');
  invariant(Array.isArray(inventory.storage_ownership), 'storage_ownership must be an array');
  invariant(Array.isArray(inventory.timing_baseline), 'timing_baseline must be an array');
  invariant(Array.isArray(inventory.baseline_anomalies), 'baseline_anomalies must be an array');
  invariant(Array.isArray(inventory.macos_gaps), 'macos_gaps must be an array');

  const taskIds = new Set(manifest.tasks.map((task) => task.id));
  const groupIds = new Set();
  for (const group of inventory.legacy_reachability) {
    invariant(group.id && !groupIds.has(group.id), `duplicate or missing legacy group id: ${group.id}`);
    groupIds.add(group.id);
    invariant(group.completion_requirement === 'zero_production_references', `${group.id}: invalid completion requirement`);
    invariant(group.owner_tasks?.length > 0, `${group.id}: owner_tasks required`);
    for (const taskId of group.owner_tasks) invariant(taskIds.has(taskId), `${group.id}: unknown task ${taskId}`);
    invariant(['literal', 'regex', 'paths', 'catalog_ids'].includes(group.matcher?.kind), `${group.id}: unsupported matcher`);
    invariant(Number.isInteger(group.baseline?.match_count) && group.baseline.match_count >= 0,
      `${group.id}: baseline.match_count must be a non-negative integer`);
    invariant(Number.isInteger(group.baseline?.file_count) && group.baseline.file_count >= 0,
      `${group.id}: baseline.file_count must be a non-negative integer`);
    invariant(/^[a-f0-9]{64}$/.test(group.baseline?.files_digest ?? ''),
      `${group.id}: baseline.files_digest must be SHA-256`);
  }

  for (const owner of inventory.storage_ownership) {
    invariant(owner.id && owner.current_facts?.length > 0, 'storage owner requires id/current_facts');
    invariant(owner.target_owner && owner.target_facts?.length > 0, `${owner.id}: target owner/facts required`);
    for (const taskId of owner.owner_tasks ?? []) invariant(taskIds.has(taskId), `${owner.id}: unknown task ${taskId}`);
  }
  for (const timing of inventory.timing_baseline) {
    invariant(timing.command && timing.exit_code === 0, 'timing command must have a successful sample');
    invariant(Number.isFinite(timing.wall_time_ms) && timing.wall_time_ms > 0, `${timing.command}: invalid timing`);
  }
  for (const anomaly of inventory.baseline_anomalies) {
    invariant(anomaly.id && taskIds.has(anomaly.owner_task), `baseline anomaly has unknown owner: ${anomaly.id}`);
    invariant(['open', 'closed'].includes(anomaly.state), `${anomaly.id}: invalid anomaly state`);
    invariant(anomaly.observed && anomaly.control, `${anomaly.id}: observed/control evidence required`);
  }
  for (const gap of inventory.macos_gaps) {
    invariant(gap.id && taskIds.has(gap.owner_task), `macOS gap has unknown owner: ${gap.id}`);
    invariant(['pending', 'implemented_unverified', 'verified', 'blocked'].includes(gap.state), `${gap.id}: invalid state`);
    invariant(gap.required_evidence?.length > 0, `${gap.id}: evidence list required`);
  }
}

function collectReport(inventory) {
  const manifest = readJson(MANIFEST_PATH);
  validateInventorySchema(inventory, manifest);
  execFileSync('git', ['cat-file', '-e', `${inventory.source.barrier_commit}^{commit}`], { cwd: ROOT });
  const allPaths = workspacePaths();
  const productionPaths = allPaths.filter((path) => isProductionPath(path, inventory.production_scope));
  const groups = inventory.legacy_reachability.map((group) => ({
    id: group.id,
    owner_tasks: group.owner_tasks,
    actual: scanGroup(group, productionPaths),
    baseline: group.baseline,
  }));
  return {
    schema_version: inventory.schema_version,
    source: inventory.source,
    summary: {
      production_files_scanned: productionPaths.length,
      legacy_groups: groups.length,
      production_legacy_matches: groups.reduce((sum, group) => sum + group.actual.match_count, 0),
      baseline_anomalies_open: inventory.baseline_anomalies.filter((anomaly) => anomaly.state !== 'closed').length,
      macos_gaps_pending: inventory.macos_gaps.filter((gap) => gap.state !== 'verified').length,
    },
    groups,
    storage_ownership: inventory.storage_ownership,
    timing_baseline: inventory.timing_baseline,
    baseline_anomalies: inventory.baseline_anomalies,
    macos_gaps: inventory.macos_gaps,
  };
}

function validateBaseline(report) {
  const errors = [];
  for (const group of report.groups) {
    if (group.actual.match_count > group.baseline.match_count) {
      errors.push(`${group.id}: match count grew ${group.baseline.match_count} -> ${group.actual.match_count}`);
    }
    if (group.actual.file_count > group.baseline.file_count) {
      errors.push(`${group.id}: file count grew ${group.baseline.file_count} -> ${group.actual.file_count}`);
    }
    if (group.actual.match_count === group.baseline.match_count &&
        group.actual.file_count === group.baseline.file_count &&
        group.actual.files_digest !== group.baseline.files_digest) {
      errors.push(`${group.id}: unchanged-size legacy set moved into different files`);
    }
    if (group.baseline.catalog_item_count !== undefined &&
        group.actual.catalog_item_count > group.baseline.catalog_item_count) {
      errors.push(`${group.id}: catalog item count grew ${group.baseline.catalog_item_count} -> ${group.actual.catalog_item_count}`);
    }
  }
  return errors;
}

function validateCompletion(report) {
  const errors = report.groups
    .filter((group) => group.actual.match_count !== 0)
    .map((group) => `${group.id}: ${group.actual.match_count} production reference(s) remain`);
  for (const gap of report.macos_gaps) {
    if (gap.state !== 'verified') errors.push(`${gap.id}: macOS state is ${gap.state}`);
  }
  for (const anomaly of report.baseline_anomalies) {
    if (anomaly.state !== 'closed') errors.push(`${anomaly.id}: baseline anomaly is ${anomaly.state}`);
  }
  return errors;
}

export function assertSelfTest() {
  const sample = 'alpha\nbeta alpha\n';
  const literals = collectLiteralMatches(sample, ['alpha']);
  invariant(literals.length === 2 && literals[1].line === 2, 'literal scanner self-test failed');
  const identifiers = collectIdentifierMatches(
    'fs.read ssh.fs.read workspace.files/read fs.read.detail computer/a11y.observe a11y.observe',
    ['fs.read', 'a11y.observe'],
  );
  invariant(identifiers.length === 2 && identifiers.map((match) => match.term).join(',') === 'fs.read,a11y.observe',
    'identifier scanner self-test failed');
  const rustProduction = rustProductionText(
    '#[cfg(test)]\nmod tests { const SQL: &str = "FROM conversations"; }\nconst SQL: &str = "FROM agent_sessions";\n',
  );
  invariant(!rustProduction.includes('FROM conversations') && rustProduction.includes('FROM agent_sessions'),
    'Rust production masking self-test failed');
  const regex = collectRegexMatches(sample, { pattern: 'a(?:lpha)?', flags: '' });
  invariant(regex.length === 3, 'regex scanner self-test failed');
  invariant(isProductionPath('crates/x/src/lib.rs', {
    roots: ['crates'], excluded_prefixes: [], excluded_fragments: ['/tests/'],
  }), 'production path self-test rejected source');
  invariant(!isProductionPath('crates/x/tests/e2e.rs', {
    roots: ['crates'], excluded_prefixes: [], excluded_fragments: ['/tests/'],
  }), 'production path self-test admitted tests');
  const shrinkReport = { groups: [{
    id: 'sample', baseline: { match_count: 2, file_count: 1, files_digest: digestValues(['crates/x/src/lib.rs']) },
    actual: { match_count: 1, file_count: 1, files_digest: digestValues(['crates/x/src/lib.rs']) },
  }] };
  invariant(validateBaseline(shrinkReport).length === 0, 'baseline must allow shrinkage');
  shrinkReport.groups[0].actual.match_count = 2;
  shrinkReport.groups[0].actual.files_digest = digestValues(['crates/y/src/lib.rs']);
  invariant(validateBaseline(shrinkReport).length === 1, 'baseline must reject same-size legacy movement');
  return { status: 'self-test-pass' };
}

function printHuman(report) {
  console.log(`UARC boundary inventory: ${report.summary.production_files_scanned} production files`);
  for (const group of report.groups) {
    console.log(`${group.id}: matches=${group.actual.match_count} files=${group.actual.file_count} owners=${group.owner_tasks.join(',')}`);
  }
  console.log(`baseline anomalies open=${report.summary.baseline_anomalies_open}`);
  console.log(`macOS gaps pending=${report.summary.macos_gaps_pending}`);
}

function main(argv = process.argv.slice(2)) {
  const inventory = readJson(INVENTORY_PATH);
  assertSelfTest();
  const report = collectReport(inventory);
  const collectOnly = argv.includes('--collect');
  const errors = collectOnly ? [] : validateBaseline(report);
  if (argv.includes('--completion')) errors.push(...validateCompletion(report));
  if (argv.includes('--baseline-json')) {
    console.log(JSON.stringify(Object.fromEntries(report.groups.map((group) => [group.id, {
      match_count: group.actual.match_count,
      file_count: group.actual.file_count,
      files_digest: group.actual.files_digest,
      ...(group.actual.catalog_item_count === undefined ? {} : { catalog_item_count: group.actual.catalog_item_count }),
    }])), null, 2));
  } else if (argv.includes('--json')) console.log(JSON.stringify(report, null, 2));
  else printHuman(report);
  if (argv.includes('--self-test') && !argv.includes('--json')) {
    console.log('UARC boundary scanner self-test passed');
  }
  if (errors.length > 0) {
    console.error(`UARC boundary check failed: ${errors.length} violation(s)`);
    for (const error of errors) console.error(`  - ${error}`);
    process.exitCode = 1;
  }
}

if (process.argv[1] && normalizePath(resolve(process.argv[1])) === normalizePath(resolve(fileURLToPath(import.meta.url)))) {
  main();
}
