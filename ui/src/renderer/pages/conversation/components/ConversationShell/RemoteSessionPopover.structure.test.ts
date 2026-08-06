/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const popoverSource = readFileSync(new URL('./RemoteSessionPopover.tsx', import.meta.url), 'utf8');
const hostBookSource = readFileSync(
  new URL('../../../settings/SshHostSettings/SshHostManagement.tsx', import.meta.url),
  'utf8'
);

describe('remote session entry', () => {
  test('starts a session from the sidebar instead of routing to settings', () => {
    // The whole point of this entry: creating a remote session used to mean
    // Settings → remote hosts → add → back to the home page. A navigation to the
    // settings page here would restore exactly the depth it removed.
    expect(popoverSource.includes('ipcBridge.ssh.list.invoke()')).toBe(true);
    expect(popoverSource.includes('useOpenSshSession')).toBe(true);
    expect(popoverSource.includes("navigate('/settings")).toBe(false);
    expect(/navigate\(/.test(popoverSource)).toBe(false);
  });

  test('reuses the host book form rather than growing a second one', () => {
    expect(popoverSource.includes('SshHostFormModal')).toBe(true);
    expect(popoverSource.includes('ipcBridge.ssh.create.invoke')).toBe(false);
  });

  test('shares one launcher with the settings host book', () => {
    // Two copies of "create a conversation with extra.ssh_host_id" would drift:
    // the model check, the cache seeding and the history refresh all have to
    // happen, and only the hook is guaranteed to do them.
    expect(hostBookSource.includes('useOpenSshSession')).toBe(true);
    expect(hostBookSource.includes('conversation.create.invoke')).toBe(false);
  });

  test('fetches the host list only once the menu has been opened', () => {
    // The sidebar is mounted on every session route; an unopened menu must not
    // cost a request. The SWR key is shared with the settings host book so the
    // two screens cannot disagree about what is saved.
    expect(popoverSource.includes("everOpened ? 'ssh-hosts.list' : null")).toBe(true);
    expect(hostBookSource.includes("useSWR('ssh-hosts.list'")).toBe(true);
  });

  test('keeps the menu open when a launch fails', () => {
    // The failure toast needs its context on screen; closing the menu would
    // leave the user looking at an unchanged sidebar with no explanation.
    expect(popoverSource.includes('if (!opened) return;')).toBe(true);
  });
});
