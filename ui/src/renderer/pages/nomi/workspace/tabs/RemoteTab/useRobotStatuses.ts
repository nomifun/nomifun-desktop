/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect, useState } from 'react';
import { ipcBridge } from '@/common';
import type { IApiRobotStatus } from '@/common/adapter/ipcBridge';

/**
 * Live phase of every robot this installation owns, keyed by `robot_id`.
 *
 * Same three-part shape every durable realtime projection in this renderer uses
 * (see `useSshLinkStatus`):
 *
 * 1. a snapshot on mount — the socket has no replay buffer, so a robot that was
 *    already connected before this section mounted is only learnable by asking;
 * 2. incremental patches from `robot.status`;
 * 3. a re-snapshot on socket reconnect, because frames dropped while the socket
 *    was down are never replayed and a stale `speaking` is the worst possible
 *    lie for this pill.
 *
 * Not filtered by companion: the map is keyed by device, so rebinding a robot to
 * another companion keeps its live phase instead of blanking it for one refetch.
 * Listeners are installed before the snapshot is requested, so a transition
 * emitted mid-flight cannot fall into a snapshot/subscribe gap; the newest
 * `changed_at` wins if it does arrive out of order.
 */
export function useRobotStatuses(): Record<string, IApiRobotStatus> {
  const [statuses, setStatuses] = useState<Record<string, IApiRobotStatus>>({});

  useEffect(() => {
    let disposed = false;

    const apply = (next: IApiRobotStatus): void => {
      if (disposed) return;
      setStatuses((known) => {
        const prev = known[next.robot_id];
        // An out-of-order delivery must not walk a newer phase backwards.
        if (prev != null && prev.changed_at > next.changed_at) return known;
        return { ...known, [next.robot_id]: next };
      });
    };

    const resnapshot = (): void => {
      void (async () => {
        try {
          const rows = await ipcBridge.robot.statuses.invoke();
          if (disposed) return;
          // Through `apply`, not straight into state: a re-fetch is a READ of
          // the same transitions the events carry, so an in-flight snapshot that
          // answers after a newer event must not walk a pill backwards.
          rows.forEach(apply);
        } catch {
          // A failed snapshot leaves whatever we already knew in place rather
          // than blanking robots that are probably still up. The section's own
          // list request is what reports a broken backend to the user.
        }
      })();
    };

    const offStatus = ipcBridge.robot.onStatus.on(apply);
    const offReconnected = ipcBridge.conversation.reconnected.on(() => {
      resnapshot();
    });

    resnapshot();

    return () => {
      disposed = true;
      offStatus();
      offReconnected();
    };
  }, []);

  return statuses;
}

export default useRobotStatuses;
