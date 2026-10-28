import { feishuConnector } from '@/common/adapter/feishuConnectorBridge';
import type { AuthorizationFlow, FeishuOperation, FeishuQuery, FeishuQueryResult, FeishuStatus } from '@/common/types/feishuConnector';
import { useCallback, useEffect, useRef, useState } from 'react';
import { liveOperation } from './model';

/** Replaceable reads abort their own observer; no effect is replayed on refresh. */
export function useFeishuQuery(query: FeishuQuery | null, ownerKey = '') {
  const key = JSON.stringify(query);
  const [revision, setRevision] = useState(0);
  const [data, setData] = useState<FeishuQueryResult | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    const request = JSON.parse(key) as FeishuQuery | null;
    if (!request) { setData(null); setError(null); setLoading(false); return; }
    const controller = new AbortController();
    setLoading(true); setError(null); setData(null);
    void feishuConnector.query(request, controller.signal).then(result => {
      if (!controller.signal.aborted) setData(result);
    }).catch(reason => {
      if (!controller.signal.aborted) setError(reason);
    }).finally(() => {
      if (!controller.signal.aborted) setLoading(false);
    });
    return () => controller.abort();
  }, [key, revision, ownerKey]);
  return { data, error, loading, refresh: useCallback(() => setRevision(value => value + 1), []) };
}

export function useFeishuConnector() {
  const [status, setStatus] = useState<FeishuStatus | null>(null);
  const [statusError, setStatusError] = useState<unknown>(null);
  const [statusLoading, setStatusLoading] = useState(true);
  const statusObserver = useRef<AbortController | null>(null);
  const mounted = useRef(true);
  const [operation, setOperationState] = useState<FeishuOperation | null>(null);
  const [operationError, setOperationError] = useState<unknown>(null);
  const [flow, setFlow] = useState<AuthorizationFlow | null>(null);
  const [flowError, setFlowError] = useState<unknown>(null);
  const flowId = useRef<string | null>(null);
  flowId.current = flow?.id ?? null;
  const operationId = useRef<string | null>(null);
  const setOperation = useCallback((value: FeishuOperation | null) => {
    operationId.current = value?.id ?? null;
    setOperationState(value);
    setOperationError(null);
  }, []);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; statusObserver.current?.abort(); };
  }, []);
  const refreshStatus = useCallback(async () => {
    statusObserver.current?.abort();
    const observer = new AbortController(); statusObserver.current = observer;
    setStatusLoading(true); setStatusError(null);
    try {
      const next = await feishuConnector.status(observer.signal);
      if (!observer.signal.aborted && mounted.current) setStatus(next);
      return next;
    } catch (error) {
      if (!observer.signal.aborted && mounted.current) setStatusError(error);
      return null;
    } finally { if (!observer.signal.aborted && mounted.current) setStatusLoading(false); }
  }, []);
  useEffect(() => { void refreshStatus(); }, [refreshStatus]);
  useEffect(() => {
    if (!status || status.runtime.phase !== 'preparing') return;
    const timer = setInterval(() => { void refreshStatus(); }, 1500);
    return () => clearInterval(timer);
  }, [status?.runtime.phase, refreshStatus]);
  const openOperation = useCallback(async (id: string) => {
    operationId.current = id; setOperationError(null);
    try {
      const next = await feishuConnector.operation(id);
      if (mounted.current && operationId.current === id) setOperation(next);
      return next;
    } catch (error) {
      if (mounted.current && operationId.current === id) setOperationError(error);
      throw error;
    }
  }, []);
  useEffect(() => {
    if (!operation || !liveOperation(operation)) return;
    const id = operation.id, controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const next = await feishuConnector.operation(id, controller.signal);
        if (controller.signal.aborted || operationId.current !== id) return;
        setOperation(next); setOperationError(null);
        if (liveOperation(next)) timer = setTimeout(() => { void poll(); }, 1000);
      } catch (error) {
        if (!controller.signal.aborted && operationId.current === id) {
          setOperationError(error);
          timer = setTimeout(() => { void poll(); }, 2500);
        }
      }
    };
    timer = setTimeout(() => { void poll(); }, 650);
    return () => { controller.abort(); clearTimeout(timer); };
  }, [operation?.id, operation?.status]);
  useEffect(() => {
    if (!flow || !['starting', 'waiting_user', 'waiting_admin', 'cancelling'].includes(flow.phase)) return;
    const id = flow.id, controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const next = await feishuConnector.auth(id, controller.signal);
        if (controller.signal.aborted || flowId.current !== id) return;
        setFlow(next); setFlowError(null);
        if (['starting', 'waiting_user', 'waiting_admin', 'cancelling'].includes(next.phase)) timer = setTimeout(() => { void poll(); }, 1000);
        else if (['complete', 'partial'].includes(next.phase)) void refreshStatus();
      } catch (error) {
        if (!controller.signal.aborted && flowId.current === id) {
          setFlowError(error); timer = setTimeout(() => { void poll(); }, 2500);
        }
      }
    };
    timer = setTimeout(() => { void poll(); }, 500);
    return () => { controller.abort(); clearTimeout(timer); };
  }, [flow?.id, flow?.phase, refreshStatus]);
  return {
    status, setStatus, statusError, statusLoading, refreshStatus,
    operation, setOperation, operationError, setOperationError, openOperation,
    flow, setFlow, flowError, setFlowError,
  };
}
