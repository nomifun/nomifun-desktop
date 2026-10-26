import { officeHub } from '@/common/adapter/officeHubBridge';
import type { OfficeActor, OfficeMutation, OfficeOperation, OfficeStatus } from '@/common/types/officeHub';
import { uuidv7 } from '@/common/utils/uuidv7';
import { useEffect, useRef, useState } from 'react';
import { errorDetails, identityKey } from './model';

export interface WriteIdentity { actor: OfficeActor; profile: string; key: string }
export const captureIdentity = (status: OfficeStatus | null): WriteIdentity | null => status?.actor && status.connection.profile
  ? { actor: { ...status.actor }, profile: status.connection.profile, key: identityKey(status) } : null;

/** A lost response is recovered by its original request ID, never by replaying a write. */
export function useOfficeWrite() {
  const ticket = useRef<{ key: string; id: string; safe: boolean } | null>(null);
  const active = useRef(false);
  const mounted = useRef(true);
  const [result, setResult] = useState<OfficeOperation | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(false);
  const [locked, setLocked] = useState(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const accept = (operation: OfficeOperation) => {
    const safe = operation.items.length > 0 && operation.items.every(item => item.status === 'not_executed');
    if (ticket.current) ticket.current.safe = safe;
    if (mounted.current) { setResult(operation); setLocked(!safe); setError(null); }
    return operation;
  };
  useEffect(() => {
    if (!result || !['queued', 'running'].includes(result.status) || !ticket.current) return;
    const controller = new AbortController(), requestId = ticket.current.id;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const operation = await officeHub.request(requestId, controller.signal);
        if (controller.signal.aborted) return;
        accept(operation);
        if (['queued', 'running'].includes(operation.status)) timer = setTimeout(() => void poll(), 1000);
      } catch (reason) {
        if (!controller.signal.aborted) { setError(reason); timer = setTimeout(() => void poll(), 2000); }
      }
    };
    timer = setTimeout(() => void poll(), 1000);
    return () => { controller.abort(); clearTimeout(timer); };
  }, [result?.id, result?.status]);
  const recover = async () => {
    if (!ticket.current || active.current) return null;
    active.current = true; setLoading(true); setError(null);
    try { return accept(await officeHub.request(ticket.current.id)); }
    catch (reason) { if (mounted.current) setError(reason); return null; }
    finally { active.current = false; if (mounted.current) setLoading(false); }
  };
  const save = async (items: OfficeMutation[], identity: WriteIdentity) => {
    if (active.current) return null;
    // Even changed input cannot escape an unresolved or successful logical write.
    if (ticket.current && !ticket.current.safe) return recover();
    const key = JSON.stringify({ actor: [identity.actor.app_id, identity.actor.user_id], profile: identity.profile, items });
    if (!ticket.current || ticket.current.key !== key || ticket.current.safe) ticket.current = { key, id: uuidv7(), safe: false };
    const request = ticket.current;
    active.current = true; setLoading(true); setLocked(true); setError(null);
    try { return accept(await officeHub.execute({ request_id: request.id, expected_actor: identity.actor, profile: identity.profile, items })); }
    catch (reason) {
      // A positive pre-admission refusal permits a new explicit Save. A 404 or
      // disconnected observer alone is not proof that no write was admitted.
      try { return accept(await officeHub.request(request.id)); }
      catch { if (errorDetails(reason).not_executed === true) request.safe = true; }
      if (mounted.current) { setError(reason); setLocked(!request.safe); }
      return null;
    } finally { active.current = false; if (mounted.current) setLoading(false); }
  };
  const reset = () => {
    if (active.current || (ticket.current && !ticket.current.safe)) return false;
    ticket.current = null;
    setResult(null); setError(null); setLocked(false);
    return true;
  };
  return { save, recover, reset, result, error, loading, locked };
}
