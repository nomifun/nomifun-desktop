import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { useAuth } from '@/renderer/hooks/context/AuthContext';
import { loadSalesAgentSnapshot } from './salesAgentSync';
import { mergeSalesAgentSnapshot } from './salesAgentReport';
import {
  SALES_WORKSPACE_STORAGE_KEY,
  archiveSalesTask,
  buildSalesTask,
  createEmptySalesWorkspace,
  parseSalesWorkspace,
  type NewSalesTask,
  type SalesCompanyProfile,
  type SalesTaskStatus,
  type SalesWorkspace,
} from './salesWorkspace';
import { loadSalesAccess, loadSalesWorkspace, saveSalesWorkspace } from './salesTenantApi';

type SalesWorkspaceContextValue = {
  workspace: SalesWorkspace;
  saveCompanyProfile: (profile: SalesCompanyProfile) => void;
  addTask: (task: NewSalesTask, status?: SalesTaskStatus) => string;
  updateTaskStatus: (taskId: string, status: SalesTaskStatus) => void;
  linkTaskConversation: (taskId: string, conversationId: string) => void;
  syncAgentProgress: () => Promise<void>;
  syncingAgentProgress: boolean;
  removeTask: (taskId: string) => void;
  workspaceReady: boolean;
  workspaceLoaded: boolean;
  workspaceError: string | null;
  retryWorkspaceLoad: () => void;
};

const SalesWorkspaceContext = createContext<SalesWorkspaceContextValue | null>(null);

const newId = () =>
  globalThis.crypto?.randomUUID?.() ?? `sales-${Date.now()}-${Math.random().toString(16).slice(2)}`;

export const SalesWorkspaceProvider: React.FC<React.PropsWithChildren> = ({ children }) => {
  const { user } = useAuth();
  const activeUserId = user?.id;
  const [workspace, setWorkspace] = useState<SalesWorkspace>(createEmptySalesWorkspace);
  const [workspaceReady, setWorkspaceReady] = useState(false);
  const [workspaceLoaded, setWorkspaceLoaded] = useState(false);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const workspaceRef = useRef(workspace);
  const activeUserIdRef = useRef(activeUserId);
  const hydratedRef = useRef(false);
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const syncingRef = useRef(false);
  const [syncingAgentProgress, setSyncingAgentProgress] = useState(false);

  useEffect(() => {
    workspaceRef.current = workspace;
  }, [workspace]);

  useEffect(() => {
    activeUserIdRef.current = activeUserId;
  }, [activeUserId]);

  useEffect(() => {
    let active = true;
    hydratedRef.current = false;
    setWorkspaceReady(false);
    setWorkspaceLoaded(false);
    setWorkspaceError(null);

    if (!activeUserId) {
      setWorkspaceError('无法确认当前登录账号');
      setWorkspaceReady(true);
      return () => {
        active = false;
      };
    }

    void Promise.all([loadSalesWorkspace(), loadSalesAccess()])
      .then(async ([remoteValue, access]) => {
        let next = parseSalesWorkspace(JSON.stringify(remoteValue));
        const legacyRaw = window.localStorage.getItem(SALES_WORKSPACE_STORAGE_KEY);
        const legacy = parseSalesWorkspace(legacyRaw);
        const remoteIsEmpty =
          !next.companyProfile.companyName &&
          next.tasks.length === 0 &&
          next.companies.length === 0 &&
          next.results.length === 0;
        const legacyHasData =
          Boolean(legacy.companyProfile.companyName) ||
          legacy.tasks.length > 0 ||
          legacy.companies.length > 0 ||
          legacy.results.length > 0;

        // One-time migration for the existing single-user installation. The
        // legacy key is removed only after the authenticated backend confirms
        // the write, so a later account can never import the same workspace.
        if (access.isInstanceOwner && remoteIsEmpty && legacyRaw && legacyHasData) {
          await saveSalesWorkspace(legacy, activeUserId);
          window.localStorage.removeItem(SALES_WORKSPACE_STORAGE_KEY);
          next = legacy;
        }

        if (!active) return;
        hydratedRef.current = true;
        workspaceRef.current = next;
        setWorkspace(next);
        setWorkspaceLoaded(true);
        setWorkspaceReady(true);
      })
      .catch((error) => {
        if (!active) return;
        setWorkspaceError(error instanceof Error ? error.message : '销售工作台加载失败');
        setWorkspaceReady(true);
      });

    return () => {
      active = false;
    };
  }, [activeUserId, loadAttempt]);

  useEffect(() => {
    if (!hydratedRef.current) return;
    const snapshot = workspace;
    saveQueueRef.current = saveQueueRef.current
      .catch(() => undefined)
      .then(() => {
        if (!activeUserId) throw new Error('无法确认当前登录账号');
        return saveSalesWorkspace(snapshot, activeUserId);
      })
      .then(() => {
        if (activeUserIdRef.current === activeUserId) setWorkspaceError(null);
      })
      .catch((error) => {
        if (activeUserIdRef.current === activeUserId) {
          setWorkspaceError(error instanceof Error ? error.message : '销售工作台保存失败');
        }
      });
  }, [activeUserId, workspace]);

  const saveCompanyProfile = useCallback((companyProfile: SalesCompanyProfile) => {
    setWorkspace((current) => ({ ...current, companyProfile }));
  }, []);

  const addTask = useCallback((input: NewSalesTask, status: SalesTaskStatus = 'draft') => {
    const taskId = newId();
    const task = { ...buildSalesTask(input, taskId, new Date().toISOString()), status };
    setWorkspace((current) => ({ ...current, tasks: [task, ...current.tasks] }));
    return taskId;
  }, []);

  const updateTaskStatus = useCallback((taskId: string, status: SalesTaskStatus) => {
    setWorkspace((current) => ({
      ...current,
      tasks: current.tasks.map((task) => (task.id === taskId ? { ...task, status } : task)),
    }));
  }, []);

  const linkTaskConversation = useCallback((taskId: string, conversationId: string) => {
    setWorkspace((current) => ({
      ...current,
      tasks: current.tasks.map((task) =>
        task.id === taskId
          ? { ...task, conversationId, status: 'running', agentUpdatedAt: new Date().toISOString() }
          : task
      ),
    }));
  }, []);

  const syncAgentProgress = useCallback(async () => {
    if (syncingRef.current) return;
    const linkedTasks = workspaceRef.current.tasks.filter((task) => task.conversationId);
    if (linkedTasks.length === 0) return;

    syncingRef.current = true;
    setSyncingAgentProgress(true);
    try {
      const snapshots = (
        await Promise.all(
          linkedTasks.map((task) => loadSalesAgentSnapshot(task).catch(() => null))
        )
      ).filter((snapshot) => snapshot !== null);
      if (snapshots.length > 0) {
        setWorkspace((current) =>
          snapshots.reduce((next, snapshot) => mergeSalesAgentSnapshot(next, snapshot), current)
        );
      }
    } finally {
      syncingRef.current = false;
      setSyncingAgentProgress(false);
    }
  }, []);

  const linkedExecutionKey = workspace.tasks
    .filter((task) => task.conversationId)
    .map((task) => `${task.id}:${task.conversationId ?? ''}:${task.status}`)
    .join('|');
  const hasRunningExecution = workspace.tasks.some(
    (task) => task.conversationId && task.status === 'running'
  );

  useEffect(() => {
    if (!linkedExecutionKey) return;
    void syncAgentProgress();
    if (!hasRunningExecution) return;
    const interval = window.setInterval(() => void syncAgentProgress(), 8000);
    return () => window.clearInterval(interval);
  }, [hasRunningExecution, linkedExecutionKey, syncAgentProgress]);

  const removeTask = useCallback((taskId: string) => {
    setWorkspace((current) => archiveSalesTask(current, taskId, new Date().toISOString()));
  }, []);

  const retryWorkspaceLoad = useCallback(() => setLoadAttempt((attempt) => attempt + 1), []);

  const value = useMemo(
    () => ({
      workspace,
      saveCompanyProfile,
      addTask,
      updateTaskStatus,
      linkTaskConversation,
      syncAgentProgress,
      syncingAgentProgress,
      removeTask,
      workspaceReady,
      workspaceLoaded,
      workspaceError,
      retryWorkspaceLoad,
    }),
    [
      addTask,
      linkTaskConversation,
      removeTask,
      saveCompanyProfile,
      syncAgentProgress,
      syncingAgentProgress,
      updateTaskStatus,
      workspace,
      workspaceError,
      workspaceLoaded,
      workspaceReady,
      retryWorkspaceLoad,
    ]
  );

  return <SalesWorkspaceContext.Provider value={value}>{children}</SalesWorkspaceContext.Provider>;
};

export const useSalesWorkspace = () => {
  const context = useContext(SalesWorkspaceContext);
  if (!context) throw new Error('useSalesWorkspace must be used inside SalesWorkspaceProvider');
  return context;
};
