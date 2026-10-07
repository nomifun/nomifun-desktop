import { ipcBridge } from '@/common';
import type { AgentSessionCapabilitySelection, AgentSessionCapabilitySelectionState } from '@/common/types/agentPlatform';
import { useCallback, useEffect, useRef, useState } from 'react';
import { fromSessionCapabilitySelection, toSessionCapabilitySelection, type SessionCapabilityDraft } from './model';

const EMPTY_DRAFT: SessionCapabilityDraft = { skillNames: [], mcpServerIds: [] };
const keyOf = (selection: AgentSessionCapabilitySelection) => JSON.stringify(toSessionCapabilitySelection(fromSessionCapabilitySelection(selection)));
const asError = (cause: unknown) => cause instanceof Error ? cause : new Error(String(cause));

type SelectionView = {
  sessionId: string;
  state?: AgentSessionCapabilitySelectionState;
  draft: SessionCapabilityDraft;
  loading: boolean;
  error?: Error;
};

/** The canonical Session owns selection and revision. Local changes are drafts
 * until an idle send admits them; projections and preset documents never grant them. */
export function useSessionCapabilitySelection(sessionId: string) {
  const [state, setState] = useState<AgentSessionCapabilitySelectionState>();
  const [loadedSessionId, setLoadedSessionId] = useState<string>();
  const [draft, setDraft] = useState<SessionCapabilityDraft>(EMPTY_DRAFT);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<Error>();
  const [reloadToken, setReloadToken] = useState(0);
  // Async callbacks update this synchronously. A render must never restore an
  // older batched state over a newer event or admission receipt.
  const latest = useRef<SelectionView>({ sessionId, draft: EMPTY_DRAFT, loading: true });
  if (latest.current.sessionId !== sessionId) latest.current = { sessionId, draft: EMPTY_DRAFT, loading: true };
  const pending = useRef<{ sessionId: string; promise: Promise<AgentSessionCapabilitySelection> } | undefined>(undefined);
  const retry = useCallback(() => setReloadToken((value) => value + 1), []);

  const receiveState = useCallback((owner: string, next: AgentSessionCapabilitySelectionState, replaceDraft = false) => {
    const current = latest.current;
    if (current.sessionId !== owner || current.state && next.binding_version < current.state.binding_version) return false;
    const clean = !current.state || keyOf(current.state.selection) === keyOf(toSessionCapabilitySelection(current.draft));
    const nextDraft = replaceDraft || clean ? fromSessionCapabilitySelection(next.selection) : current.draft;
    latest.current = { ...current, state: next, draft: nextDraft, error: undefined };
    setState(next);
    setLoadedSessionId(owner);
    setDraft(nextDraft);
    setError(undefined);
    return true;
  }, []);

  useEffect(() => {
    let cancelled = false;
    latest.current = { ...latest.current, loading: true, error: undefined };
    setLoading(true);
    if (pending.current?.sessionId !== sessionId) setSaving(false);
    setError(undefined);
    // Preserve the same Session's version floor during refresh. An event or
    // completed PUT may be newer than the GET currently in flight.
    if (!latest.current.state) {
      setState(undefined);
      setLoadedSessionId(undefined);
      setDraft(EMPTY_DRAFT);
    }
    void ipcBridge.agentPlatform.sessions.getCapabilitySelection.invoke({ agent_session_id: sessionId })
      .then((next) => { if (!cancelled) receiveState(sessionId, next); })
      .catch((cause) => {
        if (cancelled || latest.current.sessionId !== sessionId) return;
        const failure = asError(cause);
        latest.current = { ...latest.current, error: failure };
        setError(failure);
      })
      .finally(() => {
        if (cancelled || latest.current.sessionId !== sessionId) return;
        latest.current = { ...latest.current, loading: false };
        setLoading(false);
      });
    return () => { cancelled = true; };
  }, [sessionId, reloadToken, receiveState]);

  useEffect(() => {
    const unsubscribeAgent = ipcBridge.agentPlatform.sessions.onAgentChanged.on((event) => {
      if (String(event.agent_session_id) === sessionId && latest.current.sessionId === sessionId) retry();
    });
    const unsubscribeSelection = ipcBridge.agentPlatform.sessions.onCapabilitiesChanged.on((event) => {
      const current = latest.current;
      if (String(event.agent_session_id) !== sessionId || current.sessionId !== sessionId) return;
      receiveState(sessionId, {
        selection: event.selection,
        binding_version: event.binding_version,
        // Discovery without an editability fact stays read-only until GET.
        editable: event.editable ?? current.state?.editable ?? false,
      });
    });
    return () => { unsubscribeAgent(); unsubscribeSelection(); };
  }, [sessionId, retry, receiveState]);

  const applyBeforeSend = useCallback(async (): Promise<AgentSessionCapabilitySelection> => {
    if (pending.current?.sessionId === sessionId) return pending.current.promise;
    const current = latest.current;
    if (current.sessionId !== sessionId || current.loading || !current.state) {
      throw current.error ?? new Error('Session capabilities are still loading');
    }
    const selection = toSessionCapabilitySelection(current.draft);
    if (keyOf(selection) === keyOf(current.state.selection)) return current.state.selection;
    if (!current.state.editable) throw new Error('Session capabilities cannot change while this session is active or read-only');
    latest.current = { ...current, error: undefined };
    setSaving(true);
    setError(undefined);
    const admission = ipcBridge.agentPlatform.sessions.putCapabilitySelection.invoke({
      agent_session_id: sessionId,
      selection,
      expected_binding_version: current.state.binding_version,
    }).then((next) => {
      const live = latest.current;
      if (live.sessionId !== sessionId) throw new Error('The capability selection belongs to another session');
      const unchangedDraft = keyOf(toSessionCapabilitySelection(live.draft)) === keyOf(selection);
      if (!receiveState(sessionId, next, unchangedDraft)) {
        throw new Error('Session capabilities changed while applying this selection; review them before sending');
      }
      return next.selection;
    }).catch((cause) => {
      if (latest.current.sessionId === sessionId) {
        const failure = asError(cause);
        latest.current = { ...latest.current, error: failure };
        setError(failure);
      }
      throw cause;
    }).finally(() => {
      if (pending.current?.promise === admission) pending.current = undefined;
      if (latest.current.sessionId === sessionId) setSaving(false);
    });
    pending.current = { sessionId, promise: admission };
    return admission;
  }, [sessionId, receiveState]);

  const changeDraft = useCallback((next: SessionCapabilityDraft) => {
    latest.current = { ...latest.current, draft: next };
    setDraft(next);
  }, []);
  const currentState = loadedSessionId === sessionId ? state : undefined;
  const isLoading = loading || (loadedSessionId !== sessionId && !error);
  return { draft: currentState ? draft : EMPTY_DRAFT, setDraft: changeDraft, state: currentState, loading: isLoading, saving, error, retry, applyBeforeSend };
}
