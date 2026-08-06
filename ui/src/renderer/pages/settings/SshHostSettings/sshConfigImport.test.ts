/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IApiSshConfigScan, IApiSshImportResult } from '@/common/adapter/ipcBridge';
import type { SshHostId } from '@/common/types/ids';
import {
  candidateEndpoint,
  hostBookPrimaryCta,
  scanNotes,
  summarizeImport,
} from './sshConfigImport';

const scan = (overrides: Partial<IApiSshConfigScan> = {}): IApiSshConfigScan => ({
  configPath: '/home/tester/.ssh/config',
  hosts: [],
  skippedProxy: [],
  skippedIncludes: 0,
  ...overrides,
});

const candidate = (overrides: Partial<IApiSshConfigScan['hosts'][number]> = {}) => ({
  alias: 'prod-web',
  host: '10.0.3.21',
  port: 22,
  username: 'deploy' as string | null,
  identityFile: '/home/tester/.ssh/id_ed25519' as string | null,
  ...overrides,
});

const id = (value: string) => value as SshHostId;

const result = (overrides: Partial<IApiSshImportResult> = {}): IApiSshImportResult => ({
  imported: [],
  skipped: [],
  ...overrides,
});

describe('candidateEndpoint', () => {
  test('reads as the ssh command the host stands for', () => {
    expect(candidateEndpoint(candidate())).toBe('deploy@10.0.3.21:22');
  });

  test('omits the user when the config named none, rather than inventing one', () => {
    expect(candidateEndpoint(candidate({ username: null }))).toBe('10.0.3.21:22');
  });
});

describe('hostBookPrimaryCta', () => {
  test('offers import when the config has candidates', () => {
    const cta = hostBookPrimaryCta(scan({ hosts: [candidate()] }));
    expect(cta).toEqual({ kind: 'import', count: 1 });
  });

  test('falls back to add when the config has no candidates', () => {
    // A button that opens an empty import dialog is a button that does nothing.
    expect(hostBookPrimaryCta(scan())).toEqual({ kind: 'add' });
  });

  test('falls back to add while the scan is still loading or failed', () => {
    // A backend without these routes (an older build) must not strand the user
    // with no way to add a host at all.
    expect(hostBookPrimaryCta(undefined)).toEqual({ kind: 'add' });
  });

  test('offers import even when every candidate lacks a key', () => {
    // Coordinates alone are worth importing; the credential is one form away.
    const cta = hostBookPrimaryCta(
      scan({ hosts: [candidate({ identityFile: null }), candidate({ alias: 'b', host: '10.0.0.2' })] })
    );
    expect(cta).toEqual({ kind: 'import', count: 2 });
  });
});

describe('scanNotes', () => {
  test('has nothing to say about a config it read whole', () => {
    expect(scanNotes(scan({ hosts: [candidate()] }))).toEqual([]);
  });

  test('names the entries a jump host disqualified', () => {
    expect(scanNotes(scan({ skippedProxy: ['inner-a', 'inner-b'] }))).toEqual([
      { key: 'ssh.import.noteProxy', values: { count: 2, aliases: 'inner-a, inner-b' } },
    ]);
  });

  test('admits that Include directives were not followed', () => {
    expect(scanNotes(scan({ skippedIncludes: 2 }))).toEqual([
      { key: 'ssh.import.noteIncludes', values: { count: 2 } },
    ]);
  });
});

describe('summarizeImport', () => {
  test('a clean import is a success and says how many', () => {
    expect(
      summarizeImport(
        result({
          imported: [
            { alias: 'a', sshHostId: id('h1'), needsCredential: false, needsUsername: false },
            { alias: 'b', sshHostId: id('h2'), needsCredential: false, needsUsername: false },
          ],
        })
      )
    ).toEqual({
      level: 'success',
      clauses: [{ key: 'ssh.import.summaryImported', values: { count: 2 } }],
    });
  });

  test('hosts that came in without a credential are counted out loud', () => {
    // Silence here is how a user ends up with a book of hosts that cannot dial.
    expect(
      summarizeImport(
        result({
          imported: [
            { alias: 'a', sshHostId: id('h1'), needsCredential: false, needsUsername: false },
            { alias: 'b', sshHostId: id('h2'), needsCredential: true, needsUsername: false },
          ],
        })
      )
    ).toEqual({
      level: 'warning',
      clauses: [
        { key: 'ssh.import.summaryImported', values: { count: 2 } },
        { key: 'ssh.import.summaryNeedsCredential', values: { count: 1 } },
      ],
    });
  });

  test('a missing username is its own missing piece, not a credential problem', () => {
    // A stored key does not make a userless host dialable, so folding this into
    // the credential count (or omitting it) would report the host as ready.
    expect(
      summarizeImport(
        result({
          imported: [
            { alias: 'a', sshHostId: id('h1'), needsCredential: false, needsUsername: true },
          ],
        })
      )
    ).toEqual({
      level: 'warning',
      clauses: [
        { key: 'ssh.import.summaryImported', values: { count: 1 } },
        { key: 'ssh.import.summaryNeedsUsername', values: { count: 1 } },
      ],
    });
  });

  test('duplicates and vanished aliases are reported separately', () => {
    // They are different problems: one means "you already have it", the other
    // means "your config changed under us".
    expect(
      summarizeImport(
        result({
          imported: [{ alias: 'a', sshHostId: id('h1'), needsCredential: false, needsUsername: false }],
          skipped: [
            { alias: 'b', reason: 'duplicateName' },
            { alias: 'c', reason: 'duplicateEndpoint' },
            { alias: 'd', reason: 'notInConfig' },
          ],
        })
      )
    ).toEqual({
      level: 'warning',
      clauses: [
        { key: 'ssh.import.summaryImported', values: { count: 1 } },
        { key: 'ssh.import.summaryDuplicate', values: { count: 2 } },
        { key: 'ssh.import.summaryVanished', values: { count: 1 } },
      ],
    });
  });

  test('an import that created nothing says so instead of claiming success', () => {
    expect(
      summarizeImport(result({ skipped: [{ alias: 'b', reason: 'duplicateName' }] }))
    ).toEqual({
      level: 'warning',
      clauses: [
        { key: 'ssh.import.summaryNothing' },
        { key: 'ssh.import.summaryDuplicate', values: { count: 1 } },
      ],
    });
  });
});
