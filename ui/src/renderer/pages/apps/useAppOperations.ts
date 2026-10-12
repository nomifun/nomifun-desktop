import { appHosting } from '@/common/adapter/appHostingBridge';
import type { AppOperation, HostedApp } from '@/common/types/appHosting';
import { useEffect, useRef, useState } from 'react';
import { appErrorMessage } from './appHostingModel';

/** Stop can supersede preparation. Late owner responses cannot restore a stopped UI. */
export function useAppOperations(onChanged: (app: HostedApp) => void, onRefresh: () => void) {
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const pending = useRef(new Map<string, { token: symbol; operation: AppOperation }>());
  useEffect(() => () => { pending.current.clear(); }, []);
  const run = async (app: HostedApp, operation: AppOperation) => {
    const current = pending.current.get(app.id);
    if (current && (operation !== 'stop' || current.operation === 'stop')) return;
    const token = Symbol(operation);
    pending.current.set(app.id, { token, operation });
    setBusy(new Set(pending.current.keys())); setError('');
    try {
      const result = await appHosting.operate.invoke({ id: app.id, operation });
      if (pending.current.get(app.id)?.token === token) {
        onChanged(result);
        return result;
      }
    } catch (e) {
      if (pending.current.get(app.id)?.token === token) { setError(appErrorMessage(e)); onRefresh(); }
    } finally {
      if (pending.current.get(app.id)?.token === token) {
        pending.current.delete(app.id); setBusy(new Set(pending.current.keys()));
      }
    }
  };
  return { run, error, setError, busy, pending };
}
