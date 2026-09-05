#!/usr/bin/env node

/**
 * Static dependency inventory for SL-S3-10.
 *
 * This audit is intentionally read-only. It does not build a second Session
 * authority and it does not edit the central application composition. The
 * output separates test/compatibility factories, app-composed transitional
 * adapters, and real production legacy boundaries. The audit must retain
 * genuine product findings rather than turning the check into an unconditional
 * PASS.
 *
 * Usage:
 *   bun scripts/validation/automation-session-dependency-audit.mjs
 *   bun scripts/validation/automation-session-dependency-audit.mjs --json
 *   bun scripts/validation/automation-session-dependency-audit.mjs --self-test
 */

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const TASK_ID = 'SL-S3-10';
export const APP_COMPOSITION_PATH =
  'crates/backend/nomifun-app/src/router/state.rs';

const DOMAIN_SPECS = [
  {
    id: 'cron',
    crate: 'crates/backend/nomifun-cron',
    adapter: 'crates/backend/nomifun-cron/src/session_port.rs',
    consumer: 'crates/backend/nomifun-cron/src/executor.rs',
    port: 'CronSessionPort',
    methods: 6,
    canonicalCoverage: 3,
    rank: 1,
    readiness: 'first-candidate',
    blockers: [
      'canonical Session has no scheduled-session lookup by cron relation',
      'canonical Session has no background runtime-preparation/reconciliation port',
      'Cron still stores legacy conversation_id values and has no production AgentSession binding adapter',
    ],
    appComposition: {
      function: 'build_cron_state',
      evidence: [
        /conversation_owner\s*:\s*Arc<\s*NomiCoreSessionOwner\s*>/,
        /Arc<\s*dyn\s+nomifun_cron::CronSessionPort\s*>\s*=\s*conversation_owner/,
      ],
    },
    compatibilityFactory: 'test_cron_session_port',
  },
  {
    id: 'agent-execution',
    crate: 'crates/backend/nomifun-agent-execution',
    adapter: 'crates/backend/nomifun-agent-execution/src/attempt_runner.rs',
    consumer: 'crates/backend/nomifun-agent-execution/src/production.rs',
    port: 'AgentExecutionSessionPort',
    methods: 11,
    canonicalCoverage: 5,
    rank: 2,
    readiness: 'second-candidate',
    blockers: [
      'attempt delivery still returns Conversation receipt types',
      'steer and assistant-report projection are absent from canonical Session',
      'runtime token/error observations are not part of canonical Session query',
    ],
    appComposition: {
      function: 'build_agent_execution_engine',
      evidence: [
        /conversation_owner\s*:\s*Arc<\s*NomiCoreSessionOwner\s*>/,
        /Arc<\s*dyn\s+nomifun_agent_execution::AgentExecutionSessionPort\s*>\s*=\s*conversation_owner/,
      ],
    },
    compatibilityFactory: null,
  },
  {
    id: 'channel',
    crate: 'crates/backend/nomifun-channel',
    adapter: 'crates/backend/nomifun-channel/src/session_port.rs',
    consumer: 'crates/backend/nomifun-channel/src/message_service.rs',
    port: 'ChannelSessionPort',
    methods: 7,
    canonicalCoverage: 5,
    rank: 3,
    readiness: 'blocked-by-event-translation',
    blockers: [
      'channel relay requires a broadcast AgentStreamEvent subscription',
      'canonical Session query has no channel delivery receipt type',
      'channel creation/get/list operations still use Conversation DTOs',
    ],
    appComposition: {
      function: 'build_channel_state',
      evidence: [
        /conversation_owner\s*:\s*Arc<\s*NomiCoreSessionOwner\s*>/,
        /Arc<\s*dyn\s+nomifun_channel::ChannelSessionPort\s*>\s*=\s*conversation_owner/,
      ],
    },
    compatibilityFactory: null,
  },
  {
    id: 'requirement-autowork',
    crate: 'crates/backend/nomifun-requirement',
    adapter: 'crates/backend/nomifun-requirement/src/conversation_port.rs',
    consumer: 'crates/backend/nomifun-requirement/src/auto_work_runner.rs',
    port: 'AutoWorkConversationPort',
    methods: 7,
    canonicalCoverage: 3,
    rank: 4,
    readiness: 'blocked-by-automation-contract',
    blockers: [
      'AutoWork requires a durable claim-scoped turn authority',
      'attachment activation and runtime preparation are Conversation-owned',
      'reconciliation must distinguish accepted, missing, and ambiguous receipts',
    ],
    appComposition: {
      function: 'build_requirement_state',
      evidence: [
        /conversation_owner\s*:\s*Arc<\s*NomiCoreSessionOwner\s*>/,
        /Arc<\s*dyn\s+nomifun_requirement::AutoWorkConversationPort\s*>\s*=\s*conversation_owner\.clone\(\)/,
      ],
    },
    compatibilityFactory: null,
  },
  {
    id: 'companion',
    crate: 'crates/backend/nomifun-companion',
    adapter: 'crates/backend/nomifun-companion/src/session_port.rs',
    consumer: 'crates/backend/nomifun-companion/src/companion.rs',
    port: 'CompanionSessionPort',
    methods: 7,
    canonicalCoverage: 3,
    rank: 5,
    readiness: 'blocked-by-session-metadata-contract',
    blockers: [
      'companion updates need typed metadata/extra/skill mutation commands',
      'message-local-day indexing is not exposed by canonical Session query',
      'archive/transcript consumers still address Conversation repository rows',
    ],
    appComposition: {
      function: 'build_companion_state',
      evidence: [
        /conversation_owner\s*:\s*Arc<\s*NomiCoreSessionOwner\s*>/,
        /companion_ports_(?:with_session|from_typed_host)\([\s\S]*conversation_owner/,
      ],
    },
    compatibilityFactory: 'conversation_companion_ports',
  },
  {
    id: 'idmm',
    crate: 'crates/backend/nomifun-idmm',
    adapter: 'crates/backend/nomifun-idmm/src/probe.rs',
    consumer: 'crates/backend/nomifun-idmm/src/service.rs',
    port: 'ConversationSessionPort',
    methods: 7,
    canonicalCoverage: 3,
    rank: 6,
    readiness: 'blocked-by-supervision-contract',
    blockers: [
      'IDMM needs an exact active-turn scope query',
      'IDMM needs scoped continuation/steering and provider failover commands',
      'canonical Session currently exposes no live event subscription primitive',
    ],
    appComposition: {
      function: 'build_idmm_state',
      evidence: [
        /conversation_owner\s*:\s*Arc<\s*NomiCoreSessionOwner\s*>/,
        /conversation_session\s*:\s*conversation_owner/,
      ],
    },
    compatibilityFactory: null,
  },
];

const LEGACY_PATTERNS = [
  {
    id: 'conversation-service',
    label: 'ConversationService',
    pattern: /\bConversationService\b|nomifun_conversation::ConversationService\b/g,
  },
  {
    id: 'conversation-module',
    label: 'nomifun_conversation',
    pattern: /\bnomifun_conversation::/g,
  },
  {
    id: 'runtime-registry',
    label: 'AgentRuntimeRegistry',
    pattern: /\bAgentRuntimeRegistry\b/g,
  },
  {
    id: 'runtime-options',
    label: 'AgentRuntimeBuildOptions',
    pattern: /\bAgentRuntimeBuildOptions\b/g,
  },
];

const CANONICAL_PATTERNS = [
  {
    id: 'session-command',
    label: 'AgentSessionCommandPort',
    pattern: /\bAgentSessionCommandPort\b/g,
  },
  {
    id: 'session-query',
    label: 'AgentSessionQueryPort',
    pattern: /\bAgentSessionQueryPort\b/g,
  },
  {
    id: 'canonical-session',
    label: 'CanonicalAgentSessionCommandPort',
    pattern: /\bCanonicalAgentSessionCommandPort\b/g,
  },
  {
    id: 'session-store',
    label: 'AgentSessionStore',
    pattern: /\bAgentSessionStore\b/g,
  },
];

const ADAPTER_FILE_SET = new Set(DOMAIN_SPECS.map((spec) => spec.adapter));

function normalizePath(value) {
  return value.replaceAll('\\', '/');
}

function workspaceRustPaths() {
  const output = execFileSync(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard', '-z', '--', 'crates/backend'],
    { cwd: REPO_ROOT, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 },
  );
  return [
    ...new Set(
      output
        .split('\0')
        .filter((value) => value.endsWith('.rs'))
        .map(normalizePath),
    ),
  ];
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
  return source.slice(0, start) + source.slice(start, end).replace(/[^\r\n]/g, ' ') + source.slice(end);
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
 * Remove comments and literals while retaining line/column offsets. This
 * prevents documentation, error strings, and test fixture JSON from being
 * mistaken for a production dependency.
 */
export function lexicalMask(source) {
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

function lineNumber(source, offset) {
  return source.slice(0, offset).split(/\r?\n/).length;
}

function matchesFor(source, masked, descriptor) {
  const matches = [];
  descriptor.pattern.lastIndex = 0;
  let match;
  while ((match = descriptor.pattern.exec(masked)) !== null) {
    matches.push({
      label: descriptor.label,
      line: lineNumber(source, match.index),
    });
  }
  return matches;
}

function fileRecord(path, root) {
  const absolute = resolve(root, path);
  const source = readFileSync(absolute, 'utf8');
  const lexical = lexicalMask(source);
  const masked = productionMask(source);
  const legacy = LEGACY_PATTERNS.flatMap((descriptor) =>
    matchesFor(source, masked, descriptor),
  );
  const canonical = CANONICAL_PATTERNS.flatMap((descriptor) =>
    matchesFor(source, masked, descriptor),
  );
  const functions = [
    ...masked.matchAll(/\b(?:pub\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/g),
  ].map((match) => ({
    name: match[1],
    line: lineNumber(source, match.index ?? 0),
  }));
  const allFunctions = [
    ...lexical.matchAll(/\b(?:pub\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/g),
  ].map((match) => ({
    name: match[1],
    line: lineNumber(source, match.index ?? 0),
  }));
  const isTest = /(^|\/)(tests?)(\/|$)|_test\.rs$/.test(path);
  return {
    path,
    kind: isTest ? 'test' : 'production',
    isAdapter: ADAPTER_FILE_SET.has(path),
    legacy,
    canonical,
    functions,
    allFunctions,
  };
}

function inCrate(path, crate) {
  return (
    path === `${crate}/src` ||
    path.startsWith(`${crate}/src/`) ||
    path === `${crate}/tests` ||
    path.startsWith(`${crate}/tests/`)
  );
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

function isTestOnlyCfgAttribute(attribute) {
  const compact = attribute.replace(/\s/g, '');
  if (compact === '#[cfg(test)]') return true;
  if (!compact.startsWith('#[cfg(all(') || !compact.endsWith('))]')) {
    return false;
  }
  return compact.slice('#[cfg(all('.length, -3).split(',').includes('test');
}

/**
 * Rust production modules may keep compatibility factories and fixtures
 * inline under `#[cfg(test)]`. Remove those item ranges before classifying
 * legacy references as production dependencies.
 */
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
    if (!isTestOnlyCfgAttribute(masked.slice(index, end))) {
      index = end;
      continue;
    }
    let cursor = skipSpace(masked, end);
    while (masked[cursor] === '#') {
      const next = attributeEnd(masked, cursor);
      if (next === null) break;
      cursor = skipSpace(masked, next);
    }
    let itemEnd = cursor;
    let parenDepth = 0;
    let bracketDepth = 0;
    for (; itemEnd < masked.length; itemEnd += 1) {
      const char = masked[itemEnd];
      if (char === '(') parenDepth += 1;
      if (char === ')') parenDepth = Math.max(0, parenDepth - 1);
      if (char === '[') bracketDepth += 1;
      if (char === ']') bracketDepth = Math.max(0, bracketDepth - 1);
      if (parenDepth !== 0 || bracketDepth !== 0) continue;
      if (char === ';') {
        itemEnd += 1;
        break;
      }
      if (char === '{') {
        itemEnd = matchingBrace(masked, itemEnd);
        break;
      }
    }
    ranges.push([index, itemEnd]);
    index = Math.max(itemEnd, end);
  }
  return ranges;
}

function productionMask(source) {
  let output = lexicalMask(source);
  for (const [start, end] of testOnlyRanges(source)) {
    output = replaceNonNewline(output, start, end);
  }
  return output;
}

function summarizeDomain(spec, files) {
  const domainFiles = files.filter((file) => inCrate(file.path, spec.crate));
  const production = domainFiles.filter((file) => file.kind === 'production');
  const tests = domainFiles.filter((file) => file.kind === 'test');
  const productionLegacy = production.filter(
    (file) => !file.isAdapter && file.legacy.length > 0,
  );
  const testCompatFiles = tests.filter((file) => file.legacy.length > 0);
  const productionCanonical = production.filter((file) => file.canonical.length > 0);
  const consumerRecord = files.find((file) => file.path === spec.consumer);
  const adapterRecord = files.find((file) => file.path === spec.adapter);
  return {
    id: spec.id,
    crate: spec.crate,
    adapter: spec.adapter,
    consumer: spec.consumer,
    port: spec.port,
    interfaceMethods: spec.methods,
    estimatedCanonicalOperations: spec.canonicalCoverage,
    rank: spec.rank,
    readiness: spec.readiness,
    blockers: spec.blockers,
    productionFiles: production.length,
    testFiles: tests.length,
    productionFilesWithLegacyDependencies: productionLegacy.length,
    productionFilesWithCanonicalReferences: productionCanonical.length,
    testCompatFilesWithLegacyDependencies: testCompatFiles.length,
    transitionalAdapter: {
      path: spec.adapter,
      present: Boolean(adapterRecord),
      legacyReferences: adapterRecord?.legacy ?? [],
      canonicalReferences: adapterRecord?.canonical ?? [],
    },
    compatibilityFactory: {
      name: spec.compatibilityFactory,
      present:
        spec.compatibilityFactory == null ||
        Boolean(
          adapterRecord?.functions.some(
            (fn) => fn.name === spec.compatibilityFactory,
          ) ||
            // Compatibility factories are intentionally allowed to live in
            // cfg(test) items.  `productionMask` removes those items for
            // legacy-consumer classification, so inspect the lexical source
            // separately when proving the test-only seam still exists.
            adapterRecord?.allFunctions.some(
              (fn) => fn.name === spec.compatibilityFactory,
            ),
        ),
      line:
        adapterRecord?.functions.find(
          (fn) => fn.name === spec.compatibilityFactory,
        )?.line ?? null,
    },
    adapterLegacyReferences: adapterRecord?.legacy ?? [],
    adapterCanonicalReferences: adapterRecord?.canonical ?? [],
    consumerLegacyReferences: consumerRecord?.legacy ?? [],
    consumerCanonicalReferences: consumerRecord?.canonical ?? [],
    legacyFiles: productionLegacy.map((file) => ({
      path: file.path,
      references: file.legacy,
    })),
    testCompatFiles: testCompatFiles.map((file) => ({
      path: file.path,
      references: file.legacy,
    })),
  };
}

function findFunctionBody(source, functionName) {
  const masked = lexicalMask(source);
  const escaped = functionName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const functionIndex = masked.search(new RegExp(`\\bfn\\s+${escaped}\\s*\\(`));
  if (functionIndex < 0) return null;
  const open = masked.indexOf('{', functionIndex);
  if (open < 0) return null;
  return source.slice(functionIndex, matchingBrace(masked, open));
}

function inspectAppComposition(root = REPO_ROOT) {
  const source = readFileSync(resolve(root, APP_COMPOSITION_PATH), 'utf8');
  const domains = DOMAIN_SPECS.map((spec) => {
    const body = findFunctionBody(source, spec.appComposition.function);
    const evidence = spec.appComposition.evidence.map((pattern) => {
      pattern.lastIndex = 0;
      return { pattern: pattern.source, matched: body !== null && pattern.test(body) };
    });
    return {
      domain: spec.id,
      function: spec.appComposition.function,
      path: APP_COMPOSITION_PATH,
      status:
        body !== null && evidence.every((item) => item.matched)
          ? 'covered'
          : 'missing',
      evidence,
      missing: evidence
        .filter((item) => !item.matched)
        .map((item) => item.pattern),
    };
  });
  return {
    path: APP_COMPOSITION_PATH,
    owner: 'NomiCoreSessionOwner',
    domains,
    coveredDomains: domains.filter((item) => item.status === 'covered').length,
    missingDomains: domains.filter((item) => item.status !== 'covered').length,
  };
}

export function collectAutomationDependencyInventory(root = REPO_ROOT) {
  const paths = workspaceRustPaths()
    .filter((path) => DOMAIN_SPECS.some((spec) => inCrate(path, spec.crate)))
    .sort();
  const files = paths.map((path) => fileRecord(path, root));
  const domains = DOMAIN_SPECS.map((spec) => summarizeDomain(spec, files));
  const productionLegacyFiles = files.filter(
    (file) =>
      file.kind === 'production' &&
      !file.isAdapter &&
      file.legacy.length > 0,
  );
  const adapterPaths = domains.map((domain) => domain.adapter);
  const adaptersWithLegacy = domains.filter(
    (domain) => domain.transitionalAdapter.legacyReferences.length > 0,
  );
  const testCompatFiles = files.filter(
    (file) => file.kind === 'test' && file.legacy.length > 0,
  );
  const appComposition = inspectAppComposition(root);
  const migrationCandidateDomain = domains.find(
    (domain) =>
      domain.productionFilesWithLegacyDependencies > 0 ||
      domain.adapterLegacyReferences.length > 0,
  );
  return {
    task: TASK_ID,
    scope: DOMAIN_SPECS.map((spec) => spec.id),
    summary: {
      scannedRustFiles: files.length,
      productionFiles: files.filter((file) => file.kind === 'production').length,
      testFiles: files.filter((file) => file.kind === 'test').length,
      productionFilesWithLegacyDependencies: productionLegacyFiles.length,
      transitionalAdapters: adapterPaths.length,
      transitionalAdaptersWithLegacyDependencies: adaptersWithLegacy.length,
      testCompatFilesWithLegacyDependencies: testCompatFiles.length,
      appCompositionCoveredDomains: appComposition.coveredDomains,
      appCompositionMissingDomains: appComposition.missingDomains,
    },
    domains,
    appComposition,
    migrationCandidate: migrationCandidateDomain
      ? {
          domain: migrationCandidateDomain.id,
          path: migrationCandidateDomain.adapter,
          consumer: migrationCandidateDomain.consumer,
          rationale:
            'The lowest-rank domain with a remaining production or adapter legacy dependency is the next migration candidate.',
          currentStatus: migrationCandidateDomain.readiness,
          compositionStatus:
            appComposition.domains.find(
              (domain) => domain.domain === migrationCandidateDomain.id,
            )?.status ?? 'missing',
          nextRequiredContract: migrationCandidateDomain.blockers,
        }
      : {
          domain: null,
          path: null,
          consumer: null,
          rationale: 'Every audited production and adapter legacy dependency is cleared.',
          currentStatus: 'complete',
          compositionStatus:
            appComposition.missingDomains === 0 ? 'covered' : 'missing',
          nextRequiredContract: [],
        },
  };
}

export function assertAuditInvariants(report) {
  if (report.task !== TASK_ID) {
    throw new Error(`unexpected task id: ${report.task}`);
  }
  if (report.scope.length !== DOMAIN_SPECS.length) {
    throw new Error('the automation scope must contain all six domains');
  }
  if (report.appComposition.missingDomains !== 0) {
    throw new Error(
      `app composition is missing: ${report.appComposition.domains
        .filter((domain) => domain.status !== 'covered')
        .map((domain) => domain.domain)
        .join(', ')}`,
    );
  }
  for (const domain of report.domains) {
    if (!ADAPTER_FILE_SET.has(domain.adapter)) {
      throw new Error(`missing adapter declaration for ${domain.id}`);
    }
    if (
      domain.compatibilityFactory.name != null &&
      !domain.compatibilityFactory.present
    ) {
      throw new Error(
        `${domain.id} compatibility factory ${domain.compatibilityFactory.name} is missing`,
      );
    }
    if (
      domain.legacyFiles.some(
        (file) =>
          file.path === domain.adapter ||
          file.path.split('/').includes('tests'),
      )
    ) {
      throw new Error(`${domain.id} test/adapter references leaked into production legacy`);
    }
  }
  const expectedCandidate = report.domains.find(
    (domain) =>
      domain.productionFilesWithLegacyDependencies > 0 ||
      domain.adapterLegacyReferences.length > 0,
  );
  if (expectedCandidate == null) {
    if (
      report.migrationCandidate.domain !== null ||
      report.migrationCandidate.currentStatus !== 'complete'
    ) {
      throw new Error('a clean audit must report a complete migration');
    }
  } else if (
    report.migrationCandidate.domain !== expectedCandidate.id ||
    report.migrationCandidate.path !== expectedCandidate.adapter
  ) {
    throw new Error(
      `expected ${expectedCandidate.id} as the next migration candidate, got ${report.migrationCandidate.domain}`,
    );
  }
  return report;
}

function assertSyntheticMask() {
  const nonCode = `
    // ConversationService in a comment must be ignored.
    const text = "ConversationService";
    /* nomifun_conversation::fake */
  `;
  const maskedNonCode = lexicalMask(nonCode);
  if (
    maskedNonCode.includes('ConversationService') ||
    maskedNonCode.includes('nomifun_conversation::')
  ) {
    throw new Error('lexical mask failed to remove comments and literals');
  }
  const realImport = lexicalMask('use nomifun_conversation::ConversationService;');
  if (!realImport.includes('nomifun_conversation::')) {
    throw new Error('lexical mask removed a real import');
  }
  const legacyReferences = (source) =>
    LEGACY_PATTERNS.flatMap((descriptor) =>
      matchesFor(source, productionMask(source), descriptor),
    );
  if (
    legacyReferences('use nomifun_ai_agent::artifact_store::ArtifactStore;')
      .length !== 0
  ) {
    throw new Error('non-runtime agent support was misclassified as a Session dependency');
  }
  if (
    legacyReferences(
      'use nomifun_ai_agent::runtime_registry::AgentRuntimeRegistry;',
    ).length === 0
  ) {
    throw new Error('direct runtime registry dependency was not classified as legacy');
  }
  const testOnly = `
    #[cfg(test)]
    mod tests {
      use nomifun_conversation::ConversationService;
    }
    fn production() {}
  `;
  if (productionMask(testOnly).includes('nomifun_conversation::')) {
    throw new Error('cfg(test) item was incorrectly classified as production');
  }
  const cfgAllTest = `
    #[cfg(all(feature = "x", test))]
    fn fixture() { let _ = nomifun_conversation::ConversationService; }
    fn production() {}
  `;
  if (productionMask(cfgAllTest).includes('nomifun_conversation::')) {
    throw new Error('cfg(all(..., test)) item was incorrectly classified as production');
  }
}

export function assertSelfTest() {
  assertSyntheticMask();
  const report = collectAutomationDependencyInventory();
  assertAuditInvariants(report);
  return {
    status: 'self-test-pass',
    task: TASK_ID,
    domains: report.domains.length,
    candidate: report.migrationCandidate.path,
  };
}

function printHumanReport(report) {
  console.log(`${report.task} automation Session dependency audit`);
  console.log(
    `scanned=${report.summary.scannedRustFiles} production=${report.summary.productionFiles} ` +
      `tests=${report.summary.testFiles} ` +
      `production_legacy_files=${report.summary.productionFilesWithLegacyDependencies} ` +
      `adapters=${report.summary.transitionalAdapters} ` +
      `transitional_adapters_with_legacy_dependencies=${report.summary.transitionalAdaptersWithLegacyDependencies} ` +
      `test_compat_files=${report.summary.testCompatFilesWithLegacyDependencies} ` +
      `app_composition=${report.summary.appCompositionCoveredDomains}/${report.scope.length}`,
  );
  for (const domain of report.domains) {
    const composition = report.appComposition.domains.find(
      (item) => item.domain === domain.id,
    );
    console.log(
      `${domain.rank}. ${domain.id}: ${domain.readiness}; ` +
        `app_composition=${composition?.status ?? 'missing'}; ` +
        `typed_adapter=${domain.transitionalAdapter.present ? 'yes' : 'missing'}; ` +
        `compat_factory=${domain.compatibilityFactory.present ? domain.compatibilityFactory.name : 'missing'}; ` +
        `adapter_legacy=${domain.adapterLegacyReferences.length}; ` +
        `production_legacy_files=${domain.productionFilesWithLegacyDependencies}; ` +
        `test_compat_files=${domain.testCompatFilesWithLegacyDependencies}; ` +
        `canonical_files=${domain.productionFilesWithCanonicalReferences}`,
    );
    for (const file of domain.legacyFiles) {
      console.log(`  production legacy: ${file.path}`);
    }
    for (const file of domain.testCompatFiles) {
      console.log(`  test/compat: ${file.path}`);
    }
  }
  console.log(
    `candidate=${report.migrationCandidate.path ?? 'none'} ` +
      `(status=${report.migrationCandidate.currentStatus})`,
  );
  console.log(
    `candidate composition=${report.migrationCandidate.compositionStatus}`,
  );
  console.log('candidate blockers:');
  for (const blocker of report.migrationCandidate.nextRequiredContract) {
    console.log(`- ${blocker}`);
  }
}

function main(argv = process.argv.slice(2)) {
  if (argv.includes('--self-test')) {
    console.log(JSON.stringify(assertSelfTest(), null, 2));
    return;
  }
  const report = assertAuditInvariants(collectAutomationDependencyInventory());
  if (argv.includes('--json')) {
    console.log(JSON.stringify(report, null, 2));
  } else {
    printHumanReport(report);
  }
  if (
    report.summary.productionFilesWithLegacyDependencies > 0 ||
    report.summary.transitionalAdaptersWithLegacyDependencies > 0
  ) {
    process.exitCode = 1;
  }
}

if (process.argv[1] && normalizePath(resolve(process.argv[1])) === normalizePath(resolve(fileURLToPath(import.meta.url)))) {
  main();
}
