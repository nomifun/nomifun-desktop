/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const pillSource = readFileSync(new URL('./SshHostStatusPill.tsx', import.meta.url), 'utf8');
const hookSource = readFileSync(
  new URL('../hooks/useSshLinkStatus.ts', import.meta.url),
  'utf8'
);
const conversationSource = readFileSync(new URL('./ChatConversation.tsx', import.meta.url), 'utf8');
const chatLayoutSource = readFileSync(new URL('./ChatLayout/index.tsx', import.meta.url), 'utf8');

describe('SshHostStatusPill structure', () => {
  test('is the shared interactive capability pill, not a passive tag', () => {
    expect(pillSource.includes('capabilityHeaderButtonClass(')).toBe(true);
    expect(pillSource.includes('capabilityHeaderButtonStyle(')).toBe(true);
    expect(pillSource.includes("size='mini'")).toBe(true);
    expect(pillSource.includes("shape='round'")).toBe(true);
    expect(pillSource.includes("type='secondary'")).toBe(true);
    expect(pillSource.includes("<Popover trigger='click' position='br'")).toBe(true);
    expect(pillSource.includes("<span className='inline-flex items-center gap-6px leading-none'>")).toBe(true);
    expect(pillSource.includes("import { Server } from '@icon-park/react';")).toBe(true);
    expect(pillSource.includes('text-12px')).toBe(true);
    expect(pillSource.includes("data-testid='ssh-host-status-pill'")).toBe(true);
    // A disabled Arco button swallows pointer events, so the tooltip needs a
    // wrapper span (same treatment as AutoWork / IDMM / Knowledge).
    expect(pillSource.includes("<span className='inline-flex'>{button}</span>")).toBe(true);
  });

  test('colour comes only from the phase, through the shared table', () => {
    expect(pillSource.includes('SSH_STATUS_COLOR')).toBe(true);
    // No colour may be invented here: not a literal, and never string-matched
    // out of `detail` (which is free-form operator text).
    expect(/rgb\(/.test(pillSource)).toBe(false);
    expect(/#[0-9a-fA-F]{3,8}\b/.test(pillSource)).toBe(false);
    expect(pillSource.includes('detail.includes')).toBe(false);
    expect(pillSource.includes('CAPABILITY_COLORS')).toBe(false);
  });

  test('live state comes from the link hook, never from the stale host row column', () => {
    expect(pillSource.includes('useSshLinkStatus')).toBe(true);
    // `IApiSshHost.status` is written once on first connect and never walked
    // back (V9) — reading it here would resurrect the "always green" bug.
    expect(pillSource.includes('host.status')).toBe(false);
    expect(pillSource.includes('.status ===')).toBe(false);
  });

  test('identity rows are only what the host DTO honestly exposes', () => {
    expect(pillSource.includes("t('ssh.pill.endpoint')")).toBe(true);
    expect(pillSource.includes("t('ssh.pill.hostKey')")).toBe(true);
    // sudoPassword arrives masked as '***' when stored and null otherwise, so
    // "is a sudo password stored" is a fact, not a guess.
    expect(pillSource.includes('host.sudoPassword')).toBe(true);
    expect(pillSource.includes("t('ssh.pill.sudoStored')")).toBe(true);
    expect(pillSource.includes("t('ssh.pill.sudoMissing')")).toBe(true);
    // Live diagnostics: the operator detail, the retry countdown, and the
    // unconfirmed-exit warning (closed + reaped === false).
    expect(pillSource.includes("t('ssh.pill.detail')")).toBe(true);
    expect(pillSource.includes('nextRetryInMs')).toBe(true);
    expect(pillSource.includes("t('ssh.pill.retryIn'")).toBe(true);
    expect(pillSource.includes('reaped === false')).toBe(true);
    expect(pillSource.includes("t('ssh.pill.unconfirmedExit')")).toBe(true);
  });

  test('an unresolvable host still shows an identity, never nothing', () => {
    // Going silent here is the one failure this pill exists to prevent: until the
    // backend's host-deletion cut lands as `closed`, the session is still driving
    // a real machine. A grey chip carrying the host id prefix keeps "which box am
    // I on" answerable; `return null` leaves the header indistinguishable from a
    // local session.
    expect(pillSource.includes('return null')).toBe(false);
    expect(pillSource.includes("t('ssh.group.hostMissing')")).toBe(true);
    expect(pillSource.includes('sshHostId.slice(0,')).toBe(true);
    // Shares the host book's SWR key, so the pill costs no extra round-trip.
    expect(pillSource.includes("useSWR('ssh-hosts.list'")).toBe(true);
    expect(pillSource.includes('ipcBridge.ssh.list')).toBe(true);
    expect(pillSource.includes('ipcBridge.ssh.get')).toBe(false);
  });

  test('the hook snapshots, patches on both ids, and resyncs on reconnect', () => {
    expect(hookSource.includes('ipcBridge.ssh.statuses.invoke()')).toBe(true);
    expect(hookSource.includes('ipcBridge.ssh.onStatus.on(')).toBe(true);
    // The socket has no replay buffer: a durable projection must re-snapshot.
    expect(hookSource.includes('ipcBridge.conversation.reconnected.on(')).toBe(true);
    // Both ids, or a second session on the same host would overwrite this one.
    expect(hookSource.includes('event.conversationId !== conversationId')).toBe(true);
    expect(hookSource.includes('event.sshHostId !== sshHostId')).toBe(true);
    // Every subscription is torn down on unmount.
    expect(hookSource.includes('offStatus()')).toBe(true);
    expect(hookSource.includes('offReconnected()')).toBe(true);
  });

  test('a terminal drop asks for action instead of promising a comeback', () => {
    // `retryable === false` is the backend saying a person has to change
    // something (a rejected credential, a host key that changed). Showing the
    // neutral "it may come back on its own" copy there would be a lie, and the
    // only other way to tell the two apart — parsing `detail` — is banned above.
    expect(pillSource.includes('retryable === false')).toBe(true);
    expect(pillSource.includes("t('ssh.pill.droppedActionRequired')")).toBe(true);
    // The neutral copy survives for the drops that really are transient.
    expect(pillSource.includes("t('ssh.pill.droppedHint')")).toBe(true);
    // Still just a row in the popover: a status may never block the session.
    expect(pillSource.includes('Modal')).toBe(false);
  });

  test('mounted in the existing nomi headerExtra beside the cron manager', () => {
    expect(conversationSource.includes('<SshHostStatusPill')).toBe(true);
    expect(conversationSource.includes('ssh_host_id')).toBe(true);
    expect(conversationSource.includes('sshHostIdOf(conversation)')).toBe(true);
  });

  test('the shared chat layout was not touched to make room for this pill', () => {
    // headerExtra is already the seam; growing ChatLayout for one capability
    // would put SSH knowledge into every backend's header.
    expect(/ssh/i.test(chatLayoutSource)).toBe(false);
  });
});
