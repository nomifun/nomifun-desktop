/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Import-report copy catalogue: the client half of `POST /api/miniapps/validate`.
 *
 * The backend sends NO prose — a finding is a `rule_id`, a severity and an
 * optional `detail` (the offending reference, a byte count). Every sentence the
 * user reads is joined on here, which is why the rule id list is duplicated in
 * this module rather than derived: it mirrors `IMPORT_RULE_IDS` in
 * `crates/backend/nomifun-miniapp/src/validation.rs`, and the two records below
 * are `Record<MiniAppImportRuleId, I18nKey>` so adding an id to the union without
 * shipping its copy is a typecheck failure instead of a blank line in the report.
 *
 * A rule id this build has never heard of still renders (see
 * `resolveMiniAppImportRuleKeys`): an older client must degrade to "we don't have
 * an explanation for this one" rather than silently drop a fatal finding.
 *
 * Design spec: docs/specs/2026-08-10-miniapps-v2-workspace.zh.md (D14)
 */

import type { IApiMiniAppImportFinding, IApiMiniAppImportSeverity } from '@/common/adapter/ipcBridge';
import type { I18nKey } from '@/renderer/services/i18n';

/** Every rule the backend can emit, in report order. */
export const MINI_APP_IMPORT_RULE_IDS = [
  'empty_payload',
  'size_over_limit',
  'not_html',
  'no_root_document',
  'fragment_not_document',
  'local_ref_unsupported',
  'dev_server_ref',
  'framework_source_entry',
  'server_template_markers',
  'esm_bare_specifier',
  'external_cdn_ref',
  'web_storage_use',
  'nested_iframe_embed',
] as const;

export type MiniAppImportRuleId = (typeof MINI_APP_IMPORT_RULE_IDS)[number];

/** Short label of what is wrong, one per rule id. */
const MINI_APP_IMPORT_RULE_TITLE_KEY: Record<MiniAppImportRuleId, I18nKey> = {
  empty_payload: 'miniApps.import.rules.empty_payload.title',
  size_over_limit: 'miniApps.import.rules.size_over_limit.title',
  not_html: 'miniApps.import.rules.not_html.title',
  no_root_document: 'miniApps.import.rules.no_root_document.title',
  fragment_not_document: 'miniApps.import.rules.fragment_not_document.title',
  local_ref_unsupported: 'miniApps.import.rules.local_ref_unsupported.title',
  dev_server_ref: 'miniApps.import.rules.dev_server_ref.title',
  framework_source_entry: 'miniApps.import.rules.framework_source_entry.title',
  server_template_markers: 'miniApps.import.rules.server_template_markers.title',
  esm_bare_specifier: 'miniApps.import.rules.esm_bare_specifier.title',
  external_cdn_ref: 'miniApps.import.rules.external_cdn_ref.title',
  web_storage_use: 'miniApps.import.rules.web_storage_use.title',
  nested_iframe_embed: 'miniApps.import.rules.nested_iframe_embed.title',
};

/**
 * The remediation sentence, one per rule id. The five rules that carry a `detail`
 * interpolate `{{detail}}`; the rest ignore it.
 */
const MINI_APP_IMPORT_RULE_FIX_KEY: Record<MiniAppImportRuleId, I18nKey> = {
  empty_payload: 'miniApps.import.rules.empty_payload.fix',
  size_over_limit: 'miniApps.import.rules.size_over_limit.fix',
  not_html: 'miniApps.import.rules.not_html.fix',
  no_root_document: 'miniApps.import.rules.no_root_document.fix',
  fragment_not_document: 'miniApps.import.rules.fragment_not_document.fix',
  local_ref_unsupported: 'miniApps.import.rules.local_ref_unsupported.fix',
  dev_server_ref: 'miniApps.import.rules.dev_server_ref.fix',
  framework_source_entry: 'miniApps.import.rules.framework_source_entry.fix',
  server_template_markers: 'miniApps.import.rules.server_template_markers.fix',
  esm_bare_specifier: 'miniApps.import.rules.esm_bare_specifier.fix',
  external_cdn_ref: 'miniApps.import.rules.external_cdn_ref.fix',
  web_storage_use: 'miniApps.import.rules.web_storage_use.fix',
  nested_iframe_embed: 'miniApps.import.rules.nested_iframe_embed.fix',
};

/** Rendering order: what blocks first, what is handled for you next, notes last. */
export const MINI_APP_IMPORT_SEVERITY_ORDER: readonly IApiMiniAppImportSeverity[] = [
  'fatal',
  'autofix',
  'warning',
];

const RULE_ID_SET: ReadonlySet<string> = new Set<string>(MINI_APP_IMPORT_RULE_IDS);

export function isMiniAppImportRuleId(value: string): value is MiniAppImportRuleId {
  return RULE_ID_SET.has(value);
}

/**
 * Copy keys for one finding, or `null` when this build has no catalogue entry for
 * the id. A caller falls back to
 * `miniApps.import.rules.unknown.{title,fix}` — never to nothing.
 */
export function resolveMiniAppImportRuleKeys(ruleId: string): { title: I18nKey; fix: I18nKey } | null {
  if (!isMiniAppImportRuleId(ruleId)) return null;
  return { title: MINI_APP_IMPORT_RULE_TITLE_KEY[ruleId], fix: MINI_APP_IMPORT_RULE_FIX_KEY[ruleId] };
}

/** One severity bucket, kept even when empty is pointless — empty buckets are dropped. */
export interface MiniAppImportFindingGroup {
  severity: IApiMiniAppImportSeverity;
  findings: IApiMiniAppImportFinding[];
}

/**
 * Group findings by severity in {@link MINI_APP_IMPORT_SEVERITY_ORDER}, preserving
 * the backend's order inside each bucket and dropping empty buckets.
 *
 * A severity this build does not know about is appended in its own bucket rather
 * than discarded: an unrenderable finding is still information the user needs.
 */
export function groupMiniAppImportFindings(
  findings: readonly IApiMiniAppImportFinding[]
): MiniAppImportFindingGroup[] {
  const groups: MiniAppImportFindingGroup[] = [];
  const bucketOf = (severity: IApiMiniAppImportSeverity): MiniAppImportFindingGroup => {
    const found = groups.find((group) => group.severity === severity);
    if (found) return found;
    const created: MiniAppImportFindingGroup = { severity, findings: [] };
    groups.push(created);
    return created;
  };
  for (const severity of MINI_APP_IMPORT_SEVERITY_ORDER) {
    for (const finding of findings) {
      if (finding.severity === severity) bucketOf(severity).findings.push(finding);
    }
  }
  for (const finding of findings) {
    if (!MINI_APP_IMPORT_SEVERITY_ORDER.includes(finding.severity)) {
      bucketOf(finding.severity).findings.push(finding);
    }
  }
  return groups.filter((group) => group.findings.length > 0);
}

/** Bytes → a size a human reads, used for `size_over_limit`'s detail. */
export function formatMiniAppImportBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

/**
 * The `detail` as it should appear inside a sentence.
 *
 * `size_over_limit` sends a decimal byte count, which reads as noise at 4 MiB
 * scale; every other rule sends a reference/marker that must appear verbatim.
 * Long references are clipped so one pathological `data:`-ish value cannot push
 * the report off screen.
 */
export function formatMiniAppImportDetail(finding: IApiMiniAppImportFinding): string | null {
  const raw = finding.detail?.trim();
  if (!raw) return null;
  if (finding.rule_id === 'size_over_limit') {
    const bytes = Number.parseInt(raw, 10);
    if (Number.isFinite(bytes) && bytes > 0) return formatMiniAppImportBytes(bytes);
  }
  return raw.length > 120 ? `${raw.slice(0, 120)}…` : raw;
}
