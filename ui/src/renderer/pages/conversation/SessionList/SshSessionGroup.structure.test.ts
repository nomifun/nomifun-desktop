/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const groupSource = readFileSync(new URL('./SshSessionGroup.tsx', import.meta.url), 'utf8');
const listSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const syncSource = readFileSync(new URL('./hooks/useConversationListSync.ts', import.meta.url), 'utf8');
const uiStateSource = readFileSync(new URL('./hooks/useWorkpathUiState.ts', import.meta.url), 'utf8');
const filterSource = readFileSync(new URL('./hooks/conversationListFilter.ts', import.meta.url), 'utf8');

describe('SshSessionGroup structure', () => {
  test('reads host-bound sessions from the existing list-sync snapshot', () => {
    expect(groupSource.includes('useConversationListSync')).toBe(true);
    expect(groupSource.includes('sshConversations')).toBe(true);
    // A second full conversation fetch would double the sidebar's cost and could
    // disagree with the store. The group is a projection, never its own loader.
    expect(groupSource.includes('getUserConversations')).toBe(false);
  });

  test('groups sessions by host as a second level', () => {
    expect(groupSource.includes('ssh_host_id')).toBe(true);
    expect(groupSource.includes('new Map<SshHostId, TChatConversation[]>')).toBe(true);
    // Host names come from the host book under the very same SWR key the host
    // settings page uses, so the two mounts share one request.
    expect(groupSource.includes("useSWR('ssh-hosts.list'")).toBe(true);
    expect(groupSource.includes('ipcBridge.ssh.list')).toBe(true);
  });

  test('a host that no longer resolves still gets an honest label', () => {
    expect(groupSource.includes("t('ssh.group.hostMissing')")).toBe(true);
    expect(groupSource.includes("t('ssh.group.hostUnknown')")).toBe(true);
    // Never silently drop the session: an unresolved id falls back to a label,
    // it is not used to filter the row out of the group.
    expect(groupSource.includes('hostNames.get(sshHostId) ??')).toBe(true);
  });

  test('uses the shared session-group chrome and vocabulary', () => {
    expect(groupSource.includes("t('ssh.sessionGroup')")).toBe(true);
    expect(groupSource.includes("t('ssh.group.")).toBe(true);
    expect(groupSource.includes("import { Server } from '@icon-park/react';")).toBe(true);
    expect(groupSource.includes("import { Tooltip } from '@arco-design/web-react';")).toBe(true);
    // Empty group renders nothing at all (mirrors CompanionSessionGroup).
    expect(groupSource.includes('return null')).toBe(true);
    // Theme contract: body text token, arco border token, no raw rgb literals.
    expect(groupSource.includes('text-t-tertiary')).toBe(true);
    expect(groupSource.includes('border-border-2')).toBe(false);
  });

  test('rows are supplied by the container, never rebuilt here', () => {
    expect(groupSource.includes('renderRow')).toBe(true);
    expect(groupSource.includes('ConversationRow')).toBe(false);
    // Collapsed rail = icon-only rows with tooltips, no host sub-headers.
    expect(groupSource.includes('if (collapsed)')).toBe(true);
  });

  test('mounted in both the collapsed rail and the expanded sider', () => {
    expect(listSource.split('<SshSessionGroup').length - 1).toBe(2);
    expect(listSource.includes('expanded={ui.sshGroupExpanded}')).toBe(true);
    expect(listSource.includes('onToggleExpanded={ui.toggleSshGroup}')).toBe(true);
    // SSH rows are outside useBatchSelection's universe (it prunes ids missing
    // from `conversations`), so batch selection is switched off after the spread.
    expect(listSource.includes('batchMode={false}')).toBe(true);
    expect(listSource.includes('checked={false}')).toBe(true);
  });

  test('the list sync exposes host-bound sessions from the same single pass', () => {
    expect(syncSource.includes('sshConversations: TChatConversation[]')).toBe(true);
    expect(syncSource.includes('sshHostIdOf')).toBe(true);
    // Stable array identity, or useSyncExternalStore re-renders on every refresh.
    expect(syncSource.includes('isSameConversationList')).toBe(true);
    // Still exactly one conversation fetch in the whole renderer.
    expect(syncSource.split('getUserConversations').length - 1).toBe(1);
    // The exclusion itself stays untouched — the group is the missing half.
    expect(filterSource.includes('!isSshHostConversation')).toBe(true);
  });

  test('the group fold state is a persisted bare boolean like the companion one', () => {
    expect(uiStateSource.includes("SSH_GROUP_STORAGE_KEY = 'nomifun:ssh-group-expanded'")).toBe(true);
    expect(uiStateSource.includes('sshGroupExpanded: boolean')).toBe(true);
    expect(uiStateSource.includes('toggleSshGroup: () => void')).toBe(true);
    expect(uiStateSource.includes('readSshExpanded')).toBe(true);
  });
});
