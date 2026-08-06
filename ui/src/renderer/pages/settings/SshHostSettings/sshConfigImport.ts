/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  IApiSshConfigHost,
  IApiSshConfigScan,
  IApiSshImportResult,
} from '@/common/adapter/ipcBridge';
import type { I18nKey } from '@/renderer/services/i18n';

/**
 * An i18n key plus its interpolation values, ready for `t(...)`.
 *
 * Typed as {@link I18nKey} so a clause this module invents cannot reach the UI as
 * a raw key string, and a `count` value pluralizes the way i18next expects.
 */
export type SshImportClause = { key: I18nKey; values?: Record<string, string | number> };

/** What the host book's primary call to action should be. */
export type SshHostBookCta = { kind: 'import'; count: number } | { kind: 'add' };

/** `user@host:port` — the ssh command a candidate stands for. */
export const candidateEndpoint = (
  host: Pick<IApiSshConfigHost, 'host' | 'port' | 'username'>
): string => (host.username ? `${host.username}@${host.host}:${host.port}` : `${host.host}:${host.port}`);

/**
 * Import-first, but only when there is something to import.
 *
 * `undefined` covers both "still loading" and "the scan failed" (an older
 * backend has no such route): either way the user gets the plain Add button
 * rather than an import CTA that opens an empty dialog.
 */
export const hostBookPrimaryCta = (scan: IApiSshConfigScan | undefined): SshHostBookCta =>
  scan && scan.hosts.length > 0 ? { kind: 'import', count: scan.hosts.length } : { kind: 'add' };

/**
 * What the scan could not offer, and why. Shown next to the candidate list so a
 * short list always carries its explanation: without these, a user whose config
 * is entirely bastion-fronted or `Include`-based sees a list that looks broken.
 */
export const scanNotes = (scan: IApiSshConfigScan): SshImportClause[] => {
  const notes: SshImportClause[] = [];
  if (scan.skippedProxy.length > 0) {
    notes.push({
      key: 'ssh.import.noteProxy',
      values: { count: scan.skippedProxy.length, aliases: scan.skippedProxy.join(', ') },
    });
  }
  if (scan.skippedIncludes > 0) {
    notes.push({ key: 'ssh.import.noteIncludes', values: { count: scan.skippedIncludes } });
  }
  return notes;
};

/**
 * Turn an import result into the sentence to show the user.
 *
 * Every non-ideal outcome gets its own clause, and anything less than a wholly
 * clean import is a warning rather than a success — a green "done" over hosts
 * that were skipped or arrived without a credential is how someone ends up with
 * a host book that cannot dial anything.
 */
export const summarizeImport = (
  result: IApiSshImportResult
): { level: 'success' | 'warning'; clauses: SshImportClause[] } => {
  const needsCredential = result.imported.filter((item) => item.needsCredential).length;
  const duplicates = result.skipped.filter(
    (item) => item.reason === 'duplicateName' || item.reason === 'duplicateEndpoint'
  ).length;
  const vanished = result.skipped.filter((item) => item.reason === 'notInConfig').length;

  const clauses: SshImportClause[] =
    result.imported.length > 0
      ? [{ key: 'ssh.import.summaryImported', values: { count: result.imported.length } }]
      : [{ key: 'ssh.import.summaryNothing' }];
  if (needsCredential > 0) {
    clauses.push({ key: 'ssh.import.summaryNeedsCredential', values: { count: needsCredential } });
  }
  if (duplicates > 0) {
    clauses.push({ key: 'ssh.import.summaryDuplicate', values: { count: duplicates } });
  }
  if (vanished > 0) {
    clauses.push({ key: 'ssh.import.summaryVanished', values: { count: vanished } });
  }

  const clean = result.imported.length > 0 && needsCredential === 0 && result.skipped.length === 0;
  return { level: clean ? 'success' : 'warning', clauses };
};
