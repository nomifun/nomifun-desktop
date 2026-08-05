/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IApiSshStatus, ISshLinkPhase } from '@/common/adapter/ipcBridge';
import type { ConversationId, SshHostId } from '@/common/types/ids';
import { SSH_STATUS_COLOR } from '@/renderer/components/capability/capabilityStatusColors';
import type { I18nKey } from '@/renderer/services/i18n';
import { Button, Popover, Tooltip } from '@arco-design/web-react';
import { Server } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';

import { capabilityHeaderButtonClass, capabilityHeaderButtonStyle } from './CapabilityHeaderButton';
import { useSshLinkStatus } from '../hooks/useSshLinkStatus';

/**
 * One label per phase. A lookup of literal {@link I18nKey}s rather than a
 * template-literal key, so a renamed or missing phase label is a typecheck
 * failure instead of a raw key rendered into the header.
 */
const SSH_PHASE_LABEL_KEY: Record<ISshLinkPhase, I18nKey> = {
  idle: 'ssh.status.idle',
  connecting: 'ssh.status.connecting',
  connected: 'ssh.status.connected',
  degraded: 'ssh.status.degraded',
  reconnecting: 'ssh.status.reconnecting',
  dropped: 'ssh.status.dropped',
  closed: 'ssh.status.closed',
};

/**
 * Seconds left before the next dial attempt, or `null` when no retry is pending.
 *
 * Anchored on the local arrival of the status rather than on its `changedAt`,
 * because `changedAt` is the *server's* clock: a few seconds of skew would show
 * a countdown that starts negative or never reaches zero.
 */
function useRetryCountdown(status: IApiSshStatus | null): number | null {
  const budgetMs = status?.nextRetryInMs ?? null;
  const [remainingMs, setRemainingMs] = useState<number | null>(budgetMs);

  useEffect(() => {
    if (budgetMs == null) {
      setRemainingMs(null);
      return;
    }
    const anchor = Date.now();
    setRemainingMs(budgetMs);
    const timer = setInterval(() => {
      setRemainingMs(Math.max(0, budgetMs - (Date.now() - anchor)));
    }, 1_000);
    return () => clearInterval(timer);
    // A fresh transition re-anchors: `changedAt` identifies the publication.
  }, [budgetMs, status?.changedAt]);

  return remainingMs == null ? null : Math.ceil(remainingMs / 1_000);
}

interface Props {
  conversationId: ConversationId;
  /** `extra.ssh_host_id` of the conversation this header belongs to. */
  sshHostId: SshHostId;
}

/**
 * 会话 header 上的「远程主机」药丸（设计 §10 / TODO T1）。
 *
 * An SSH-bound session looks exactly like a local one everywhere else in the
 * chrome, which is how you end up running `rm -rf` on the wrong machine. This
 * pill is the session's identity badge: *which* host it drives, and whether the
 * link to it is actually up right now.
 *
 * Two deliberate refusals:
 *
 * - The colour is a pure function of the wire phase via {@link SSH_STATUS_COLOR}.
 *   `detail` is free-form operator text and is shown verbatim, never parsed —
 *   string-matching it is how "connection refused" ends up green.
 * - The host book's own `status` column is not read at all. It is written once on
 *   first connect and never walked back, so it is permanently green for any host
 *   that has ever worked; the live phase comes only from {@link useSshLinkStatus}.
 *
 * The host name comes from the host book under the same SWR key the settings
 * page and the sidebar group use, so the pill costs no extra round-trip.
 */
const SshHostStatusPill: React.FC<Props> = ({ conversationId, sshHostId }) => {
  const { t } = useTranslation();
  const { data: hosts } = useSWR('ssh-hosts.list', () => ipcBridge.ssh.list.invoke());
  const status = useSshLinkStatus(conversationId, sshHostId);
  const retryInSeconds = useRetryCountdown(status);

  const host = useMemo(
    () => (hosts ?? []).find((candidate) => candidate.sshHostId === sshHostId) ?? null,
    [hosts, sshHostId]
  );

  // The book has not answered yet, or the host was deleted while the session
  // survived. Either way there is no honest identity to show, and a badge that
  // says nothing is worse than no badge: stay silent.
  if (host == null) return null;

  // No live link yet is its own truthful phase, not a missing value.
  const phase: ISshLinkPhase = status?.state ?? 'idle';
  const dotColor = SSH_STATUS_COLOR[phase];
  const linked = phase === 'connected';
  const detail = status?.detail ?? null;
  // `reaped === false` is the backend saying it could NOT confirm the remote
  // shell exited — a stray process may still hold the host. That is a warning,
  // not the neutral end of a session.
  const unconfirmedExit = phase === 'closed' && status?.reaped === false;
  // Prefer the fingerprint the live link actually negotiated; fall back to the
  // one pinned in the book.
  const fingerprint = status?.hostFingerprint ?? host.hostFingerprint ?? null;
  // Credential fields arrive masked (a sentinel when stored, null when not), so
  // "is a sudo password stored" is a fact we are told, not a guess.
  const sudoStored = host.sudoPassword != null;

  const row = (label: string, value: React.ReactNode) => (
    <div className='flex items-baseline gap-8px min-w-0'>
      <span className='text-11px text-t-tertiary leading-16px shrink-0'>{label}</span>
      <span className='text-12px text-t-primary leading-16px min-w-0 break-all'>{value}</span>
    </div>
  );

  const panel = (
    <div className='flex flex-col gap-6px min-w-200px max-w-300px'>
      <div className='flex items-center gap-6px min-w-0'>
        <Server theme='outline' size='14' fill={dotColor} className='block shrink-0' style={{ lineHeight: 0 }} />
        <span className='text-13px text-t-primary font-[500] leading-none min-w-0 truncate'>{host.name}</span>
        <span className='text-11px leading-none shrink-0' style={{ color: dotColor }}>
          {t(SSH_PHASE_LABEL_KEY[phase])}
        </span>
      </div>

      {row(t('ssh.pill.endpoint'), `${host.username}@${host.host}:${host.port}`)}
      {row(t('ssh.pill.hostKey'), fingerprint ?? t('ssh.pill.hostKeyUnpinned'))}
      <div className='text-11px text-t-tertiary leading-16px'>
        {sudoStored ? t('ssh.pill.sudoStored') : t('ssh.pill.sudoMissing')}
      </div>

      {status != null && status.attempt > 0 ? (
        <div className='text-11px text-t-tertiary leading-16px'>
          {t('ssh.pill.attempt', { attempt: status.attempt })}
        </div>
      ) : null}
      {retryInSeconds != null ? (
        <div className='text-11px text-t-tertiary leading-16px'>
          {t('ssh.pill.retryIn', { seconds: retryInSeconds })}
        </div>
      ) : null}
      {detail ? row(t('ssh.pill.detail'), detail) : null}
      {phase === 'dropped' ? (
        <div className='text-11px text-t-secondary leading-16px'>{t('ssh.pill.droppedHint')}</div>
      ) : null}
      {unconfirmedExit ? (
        <div className='text-11px text-t-secondary leading-16px'>{t('ssh.pill.unconfirmedExit')}</div>
      ) : null}
    </div>
  );

  const button = (
    <Button
      size='mini'
      shape='round'
      type='secondary'
      disabled={status == null}
      data-testid='ssh-host-status-pill'
      className={capabilityHeaderButtonClass(linked, 'shrink-0')}
      style={capabilityHeaderButtonStyle(dotColor)}
    >
      <span className='inline-flex items-center gap-6px leading-none'>
        {/* The icon carries the phase colour, matching the sidebar host icon —
            the same "no separate dot" treatment AutoWork / IDMM use. */}
        <Server theme='outline' size='14' fill={dotColor} className='block' style={{ lineHeight: 0 }} />
        <span className='text-12px max-w-140px truncate'>{host.name}</span>
      </span>
    </Button>
  );

  if (status == null) {
    // Nothing has been published for this session yet, so there is no link state
    // to open a panel about. The pill stays as a passive identity chip, and a
    // disabled Arco button eats pointer events — hence the wrapper span.
    return (
      <Tooltip content={t('ssh.pill.noLink')}>
        <span className='inline-flex'>{button}</span>
      </Tooltip>
    );
  }

  return (
    <Popover trigger='click' position='br' content={panel}>
      {button}
    </Popover>
  );
};

export default SshHostStatusPill;
