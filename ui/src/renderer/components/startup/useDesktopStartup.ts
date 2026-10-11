import { useEffect, useRef, useState } from 'react';
import type { DesktopStartupIO, DesktopStartupSnapshot } from '@/common/adapter/tauriStartup';

export function useDesktopStartup(io: DesktopStartupIO) {
  const [snapshot, setSnapshot] = useState<DesktopStartupSnapshot | null>(null);
  const [connectionError, setConnectionError] = useState(false);
  const [retrying, setRetrying] = useState(false);
  const generation = useRef(0);
  const revision = useRef(-1);
  const retryPending = useRef(false);

  useEffect(() => {
    const current = ++generation.current;
    revision.current = -1;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const accept = (next: DesktopStartupSnapshot) => {
      if (disposed || current !== generation.current || !Number.isSafeInteger(next.revision) || next.revision <= revision.current) return;
      revision.current = next.revision;
      setSnapshot(next);
      setConnectionError(false);
    };
    // Listen first. A late snapshot cannot overwrite a newer download event.
    void io.subscribe(accept).then(async (dispose) => {
      if (disposed) {
        dispose();
        return;
      }
      unlisten = dispose;
      try {
        accept(await io.snapshot());
      } catch {
        if (!disposed && revision.current < 0) setConnectionError(true);
      }
    }).catch(() => {
      if (!disposed) setConnectionError(true);
    });
    return () => {
      disposed = true;
      ++generation.current;
      unlisten?.();
    };
  }, [io]);

  const retry = async () => {
    if (retryPending.current || snapshot?.stage !== 'failed' || !snapshot.preparation.error?.retryable) return;
    const current = generation.current;
    const startedRevision = revision.current;
    retryPending.current = true;
    setRetrying(true);
    try {
      const next = await io.retry();
      if (current === generation.current && next.revision > revision.current) {
        revision.current = next.revision;
        setSnapshot(next);
        setConnectionError(false);
      }
    } catch {
      if (current === generation.current && revision.current <= startedRevision) setConnectionError(true);
    } finally {
      retryPending.current = false;
      if (current === generation.current) setRetrying(false);
    }
  };
  const quit = async () => {
    const current = generation.current;
    try {
      await io.quit();
    } catch {
      if (current === generation.current) setConnectionError(true);
    }
  };
  return { snapshot, connectionError, retrying, retry, quit };
}
