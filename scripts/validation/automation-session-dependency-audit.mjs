#!/usr/bin/env node

/**
 * Static dependency inventory for SL-S3-10.
 *
 * This audit is intentionally read-only. It does not build a second Session
 * authority and it does not edit the central application composition. The
 * output separates production references from test fixtures and identifies
 * the smallest consumer boundary that can be migrated once the canonical
 * Session contract exposes the missing automation operations.
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
      'canonical Session has no operation-scoped delivery receipt query',
      'canonical Session has no background runtime-preparation/reconciliation port',
    ],
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
  {
    id: 'nomi-agent',
    label: 'nomifun_ai_agent',
    pattern: /\bnomifun_ai_agent::/g,
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
  const masked = lexicalMask(source);
  const legacy = LEGACY_PATTERNS.flatMap((descriptor) =>
    matchesFor(source, masked, descriptor),
  );
  const canonical = CANONICAL_PATTERNS.flatMap((descriptor) =>
    matchesFor(source, masked, descriptor),
  );
  const isTest = /(^|\/)(tests?)(\/|$)|_test\.rs$/.test(path);
  return {
    path,
    kind: isTest ? 'test' : 'production',
    isAdapter: ADAPTER_FILE_SET.has(path),
    legacy,
    canonical,
  };
}

function inCrate(path, crate) {
  return path === `${crate}/src` || path.startsWith(`${crate}/src/`);
}

function summarizeDomain(spec, files) {
  const domainFiles = files.filter((file) => inCrate(file.path, spec.crate));
  const production = domainFiles.filter((file) => file.kind === 'production');
  const tests = domainFiles.filter((file) => file.kind === 'test');
  const productionLegacy = production.filter((file) => file.legacy.length > 0);
  const productionCanonical = production.filter((file) => file.canonical.length > 0);
  const adapterRecord = files.find((file) => file.path === spec.adapter);
  const consumerRecord = files.find((file) => file.path === spec.consumer);
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
    adapterLegacyReferences: adapterRecord?.legacy ?? [],
    adapterCanonicalReferences: adapterRecord?.canonical ?? [],
    consumerLegacyReferences: consumerRecord?.legacy ?? [],
    consumerCanonicalReferences: consumerRecord?.canonical ?? [],
    legacyFiles: productionLegacy.map((file) => ({
      path: file.path,
      references: file.legacy,
    })),
  };
}

export function collectAutomationDependencyInventory(root = REPO_ROOT) {
  const paths = workspaceRustPaths()
    .filter((path) => DOMAIN_SPECS.some((spec) => inCrate(path, spec.crate)))
    .sort();
  const files = paths.map((path) => fileRecord(path, root));
  const domains = DOMAIN_SPECS.map((spec) => summarizeDomain(spec, files));
  const productionLegacyFiles = files.filter(
    (file) => file.kind === 'production' && file.legacy.length > 0,
  );
  const adapterPaths = domains.map((domain) => domain.adapter);
  const adaptersWithLegacy = domains.filter(
    (domain) => domain.adapterLegacyReferences.length > 0,
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
    },
    domains,
    migrationCandidate: {
      domain: 'cron',
      path: 'crates/backend/nomifun-cron/src/session_port.rs',
      consumer: 'crates/backend/nomifun-cron/src/executor.rs',
      rationale:
        'Cron is the first scheduled producer, has the smallest consumer-facing port, and is already isolated behind one adapter. Migrate its adapter only after canonical Session adds receipt/reconciliation and cron-session lookup operations.',
      currentStatus: 'not-safe-to-wire-without-central-composition',
      nextRequiredContract: [
        'open or reuse an AgentSession from an explicit frozen binding',
        'start a keyed turn and query its terminal receipt',
        'observe/reconcile an accepted turn without resend authority',
        'cancel the exact active turn',
      ],
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
  if (report.migrationCandidate.domain !== 'cron') {
    throw new Error('Cron must remain the first migration candidate');
  }
  if (report.migrationCandidate.currentStatus !== 'not-safe-to-wire-without-central-composition') {
    throw new Error('the audit must not claim a production migration without composition');
  }
  for (const domain of report.domains) {
    if (!ADAPTER_FILE_SET.has(domain.adapter)) {
      throw new Error(`missing adapter declaration for ${domain.id}`);
    }
    if (domain.adapterLegacyReferences.length === 0) {
      throw new Error(`${domain.id} adapter no longer exposes the expected legacy boundary`);
    }
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
      `tests=${report.summary.testFiles} production_legacy_files=${report.summary.productionFilesWithLegacyDependencies}`,
  );
  for (const domain of report.domains) {
    console.log(
      `${domain.rank}. ${domain.id}: ${domain.readiness}; ` +
        `adapter_legacy=${domain.adapterLegacyReferences.length}; ` +
        `production_legacy_files=${domain.productionFilesWithLegacyDependencies}; ` +
        `canonical_files=${domain.productionFilesWithCanonicalReferences}`,
    );
  }
  console.log(
    `candidate=${report.migrationCandidate.path} ` +
      `(status=${report.migrationCandidate.currentStatus})`,
  );
  console.log('candidate blockers:');
  for (const blocker of report.domains[0].blockers) {
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
}

if (process.argv[1] && normalizePath(resolve(process.argv[1])) === normalizePath(resolve(fileURLToPath(import.meta.url)))) {
  main();
}
