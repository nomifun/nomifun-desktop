/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useState } from 'react';
import { ipcBridge } from '@/common';
import type { IApiSshStatus } from '@/common/adapter/ipcBridge';
import type { SshHostId } from '@/common/types/ids';

/**
 * Live state of one conversation↔host SSH link. Snapshot loading and failure
 * stay separate from a successful snapshot that contains no link.
 *
 * Same three-part shape every durable realtime projection in this renderer uses
 * (see `useTerminalSessions` and other realtime projections):
 *
 * 1. a snapshot on mount — the socket has no replay buffer, so a link that was
 *    already connected before this component mounted is only learnable by
 *    asking;
 * 2. incremental patches from `ssh.status`, filtered on BOTH ids — a host can
 *    carry several conversations at once and the event stream is user-scoped,
 *    so filtering on the host alone would let a sibling session's transitions
 *    overwrite this pill;
 * 3. a re-snapshot on socket reconnect, because frames dropped while the socket
 *    was down are never replayed and a stale `connected` is the worst possible
 *    lie for this particular pill.
 *
 * Listeners are installed before the snapshot is requested, so a transition
 * emitted mid-flight cannot fall into a snapshot/subscribe gap; the newest
 * `changedAt` wins if it does arrive out of order — and because the server
 * reports `changedAt` as the instant the link changed rather than the instant it
 * was asked, that comparison is meaningful across both paths.
 */
export function useSshLinkStatus(
  conversationId: string,
  sshHostId: SshHostId | undefined
): {
  status: IApiSshStatus | null;
  loading: boolean;
  error: string | null;
  refresh: () => void;
} {
  const [status, setStatus] = useState<IApiSshStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refreshVersion, setRefreshVersion] = useState(0);
  const refresh = useCallback(() => setRefreshVersion((version) => version + 1), []);

  useEffect(() => {
    if (!sshHostId) {
      setStatus(null);
      setLoading(false);
      setError(null);
      return;
    }

    let disposed = false;
    let eventRevision = 0;

    const apply = (next: IApiSshStatus): void => {
      if (disposed) return;
      setStatus((prev) =>
        // An out-of-order delivery must not walk a newer state backwards.
        prev != null && prev.changedAt > next.changedAt ? prev : next
      );
    };

    const resnapshot = (): void => {
      const revisionBeforeRead = eventRevision;
      setLoading(true);
      setError(null);
      void (async () => {
        try {
          const rows = await ipcBridge.ssh.statuses.invoke();
          if (disposed) return;
          const mine = rows.find(
            (row) => row.conversationId === conversationId && row.sshHostId === sshHostId
          );
          // A status delivered after this read started is newer than an empty
          // snapshot and must remain visible.
          if (mine == null) {
            if (eventRevision === revisionBeforeRead) setStatus(null);
            return;
          }
          // Through `apply`, not straight into state: a re-fetch is a *read* of
          // the same transitions the events carry (the server stamps both from
          // when the link changed), so an in-flight snapshot that answers after a
          // newer event must not walk the pill backwards.
          apply(mine);
        } catch (caught) {
          if (!disposed) setError(caught instanceof Error ? caught.message : String(caught));
        } finally {
          if (!disposed) setLoading(false);
        }
      })();
    };

    const offStatus = ipcBridge.ssh.onStatus.on((event) => {
      if (event.conversationId !== conversationId) return;
      if (event.sshHostId !== sshHostId) return;
      eventRevision += 1;
      apply(event);
    });
    const offReconnected = ipcBridge.conversation.reconnected.on(() => {
      resnapshot();
    });

    resnapshot();

    return () => {
      disposed = true;
      offStatus();
      offReconnected();
    };
  }, [conversationId, sshHostId, refreshVersion]);

  return { status, loading, error, refresh };
}

export default useSshLinkStatus;
