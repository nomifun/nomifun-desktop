/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ICronJob, ICronJobRun, IUpdateCronJobParams } from '@/common/adapter/ipcBridge';
import {
  indexCronJobsByConversation,
  reconcileCronJobsForConversation,
  upsertCronJobByConversation,
} from './cronJobConversationMap';
import { parseConversationId, type ConversationId, type CronJobId } from '@/common/types/ids';
import { emitter } from '@/renderer/utils/emitter';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { repairCronJobTimeZones } from '@renderer/pages/cron/repairCronJobTimeZone';
import { browserStorageGenerationKey } from '@/common/utils/browserStorageKey';

const isJobErrorLike = (job: ICronJob): boolean => {
  return job.state.last_status === 'error' || job.state.last_status === 'missed';
};

// Only retain events received during the current GET (including timezone
// repair). A deletion must override the snapshot without dropping other jobs.
type CronJobChanges = Map<CronJobId, ICronJob | null>;
function mergeCronSnapshot(jobs: ICronJob[], changes: CronJobChanges): ICronJob[] {
  const merged = new Map(jobs.map((job) => [job.cron_job_id, job]));
  for (const [id, job] of changes) {
    if (job) merged.set(id, job);
    else merged.delete(id);
  }
  return [...merged.values()];
}

/**
 * Common cron job actions
 */
interface CronJobActionsResult {
  pauseJob: (cron_job_id: CronJobId) => Promise<void>;
  resumeJob: (cron_job_id: CronJobId) => Promise<void>;
  deleteJob: (cron_job_id: CronJobId) => Promise<void>;
  updateJob: (cron_job_id: CronJobId, updates: IUpdateCronJobParams) => Promise<ICronJob>;
}

/**
 * Creates common cron job action handlers
 */
function useCronJobActions(
  onJobUpdated?: (cron_job_id: CronJobId, job: ICronJob) => void,
  onJobDeleted?: (cron_job_id: CronJobId) => void
): CronJobActionsResult {
  const pauseJob = useCallback(
    async (cron_job_id: CronJobId) => {
      const updated = await ipcBridge.cron.updateJob.invoke({ cron_job_id, updates: { enabled: false } });
      onJobUpdated?.(cron_job_id, updated);
    },
    [onJobUpdated]
  );

  const resumeJob = useCallback(
    async (cron_job_id: CronJobId) => {
      const updated = await ipcBridge.cron.updateJob.invoke({ cron_job_id, updates: { enabled: true } });
      onJobUpdated?.(cron_job_id, updated);
    },
    [onJobUpdated]
  );

  const deleteJob = useCallback(
    async (cron_job_id: CronJobId) => {
      await ipcBridge.cron.removeJob.invoke({ cron_job_id });
      onJobDeleted?.(cron_job_id);
    },
    [onJobDeleted]
  );

  const updateJob = useCallback(
    async (cron_job_id: CronJobId, updates: IUpdateCronJobParams) => {
      const updated = await ipcBridge.cron.updateJob.invoke({ cron_job_id, updates });
      onJobUpdated?.(cron_job_id, updated);
      return updated;
    },
    [onJobUpdated]
  );

  return { pauseJob, resumeJob, deleteJob, updateJob };
}

/**
 * Event handlers for cron job subscription
 */
interface CronJobEventHandlers {
  onJobCreated: (job: ICronJob) => void;
  onJobUpdated: (job: ICronJob) => void;
  onJobRemoved: (data: { cron_job_id: CronJobId }) => void;
}

/**
 * Subscribe to cron job events with unified cleanup.
 *
 * WebSocket delivery has no replay: any gap (reconnect, server lag resync)
 * may have dropped cron job events, so `onResync` reloads the caller's
 * durable snapshot after every reconnect.
 */
function useCronJobSubscription(handlers: CronJobEventHandlers, onResync?: () => void | Promise<void>) {
  useEffect(() => {
    const unsubCreate = ipcBridge.cron.onJobCreated.on(handlers.onJobCreated);
    const unsubUpdate = ipcBridge.cron.onJobUpdated.on(handlers.onJobUpdated);
    const unsubRemove = ipcBridge.cron.onJobRemoved.on(handlers.onJobRemoved);
    const unsubReconnected = onResync ? ipcBridge.conversation.reconnected.on(() => void onResync()) : undefined;

    return () => {
      unsubCreate();
      unsubUpdate();
      unsubRemove();
      unsubReconnected?.();
    };
  }, [handlers.onJobCreated, handlers.onJobUpdated, handlers.onJobRemoved, onResync]);
}

/**
 * Hook for managing cron jobs for a specific conversation
 * @param conversation_id - The conversation ID to fetch jobs for
 */
export function useCronJobs(conversation_id?: ConversationId) {
  const [jobs, setJobs] = useState<ICronJob[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const pendingRef = useRef<CronJobChanges | null>(null);

  // Fetch jobs for the conversation
  const fetchJobs = useCallback(async () => {
    const changes: CronJobChanges = new Map();
    pendingRef.current = changes;
    if (conversation_id == null) {
      pendingRef.current = null;
      setJobs([]);
      setError(null);
      setLoading(false);
      return;
    }

    setLoading(true);
    setError(null);

    try {
      const result = await ipcBridge.cron.listJobsByConversation.invoke({ conversation_id });
      if (pendingRef.current !== changes) return;
      const repaired = await repairCronJobTimeZones(mergeCronSnapshot(result || [], changes));
      if (pendingRef.current !== changes) return;
      setJobs(mergeCronSnapshot(repaired, changes).filter((job) => job.metadata.conversation_id === conversation_id));
    } catch (err) {
      if (pendingRef.current !== changes) return;
      setError(err instanceof Error ? err : new Error('Failed to fetch cron jobs'));
    } finally {
      if (pendingRef.current === changes) {
        pendingRef.current = null;
        setLoading(false);
      }
    }
  }, [conversation_id]);

  // Initial fetch
  useEffect(() => {
    setJobs([]);
    void fetchJobs();
    return () => { pendingRef.current = null; };
  }, [fetchJobs]);

  // Event handlers
  const eventHandlers = useMemo<CronJobEventHandlers>(
    () => ({
      onJobCreated: (job: ICronJob) => {
        if (conversation_id && job.metadata.conversation_id === conversation_id) {
          pendingRef.current?.set(job.cron_job_id, job);
          setJobs((prev) => (prev.some((j) => j.cron_job_id === job.cron_job_id) ? prev : [...prev, job]));
        }
      },
      onJobUpdated: (job: ICronJob) => {
        if (!conversation_id) return;
        pendingRef.current?.set(job.cron_job_id, job.metadata.conversation_id === conversation_id ? job : null);
        setJobs((prev) => reconcileCronJobsForConversation(prev, conversation_id, job));
      },
      onJobRemoved: ({ cron_job_id }: { cron_job_id: CronJobId }) => {
        pendingRef.current?.set(cron_job_id, null);
        setJobs((prev) => prev.filter((j) => j.cron_job_id !== cron_job_id));
      },
    }),
    [conversation_id]
  );

  useCronJobSubscription(eventHandlers, fetchJobs);

  // Actions (without local state updates, rely on events)
  const actions = useCronJobActions();

  // Computed values
  const hasJobs = jobs.length > 0;
  const activeJobsCount = jobs.filter((j) => j.enabled).length;
  const hasError = jobs.some(isJobErrorLike);

  return {
    jobs,
    loading,
    error,
    hasJobs,
    activeJobsCount,
    hasError,
    refetch: fetchJobs,
    ...actions,
  };
}

/**
 * Hook for managing all cron jobs across all conversations
 */
export function useAllCronJobs() {
  const [jobs, setJobs] = useState<ICronJob[]>([]);
  const [loading, setLoading] = useState(true);
  const pendingRef = useRef<CronJobChanges | null>(null);

  // Fetch all jobs
  const fetchJobs = useCallback(async () => {
    const changes: CronJobChanges = new Map();
    pendingRef.current = changes;
    setLoading(true);
    try {
      const allJobs = await ipcBridge.cron.listJobs.invoke();
      if (pendingRef.current !== changes) return;
      const repaired = await repairCronJobTimeZones(mergeCronSnapshot(allJobs || [], changes));
      if (pendingRef.current !== changes) return;
      setJobs(mergeCronSnapshot(repaired, changes));
    } catch (err) {
      if (pendingRef.current === changes) console.error('[useAllCronJobs] Failed to fetch jobs:', err);
    } finally {
      if (pendingRef.current === changes) {
        pendingRef.current = null;
        setLoading(false);
      }
    }
  }, []);

  // Initial fetch
  useEffect(() => {
    void fetchJobs();
    return () => { pendingRef.current = null; };
  }, [fetchJobs]);

  // Event handlers
  const eventHandlers = useMemo<CronJobEventHandlers>(
    () => ({
      onJobCreated: (job: ICronJob) => {
        pendingRef.current?.set(job.cron_job_id, job);
        setJobs((prev) => (prev.some((j) => j.cron_job_id === job.cron_job_id) ? prev : [...prev, job]));
      },
      onJobUpdated: (job: ICronJob) => {
        pendingRef.current?.set(job.cron_job_id, job);
        setJobs((prev) => mergeCronSnapshot(prev, new Map([[job.cron_job_id, job]])));
      },
      onJobRemoved: ({ cron_job_id }: { cron_job_id: CronJobId }) => {
        pendingRef.current?.set(cron_job_id, null);
        setJobs((prev) => prev.filter((j) => j.cron_job_id !== cron_job_id));
      },
    }),
    []
  );

  useCronJobSubscription(eventHandlers, fetchJobs);

  // Actions with local state updates
  const handleJobUpdated = useCallback((cron_job_id: CronJobId, job: ICronJob) => {
    pendingRef.current?.set(cron_job_id, job);
    setJobs((prev) => prev.map((j) => (j.cron_job_id === cron_job_id ? job : j)));
  }, []);

  const handleJobDeleted = useCallback((cron_job_id: CronJobId) => {
    pendingRef.current?.set(cron_job_id, null);
    setJobs((prev) => prev.filter((j) => j.cron_job_id !== cron_job_id));
  }, []);

  const actions = useCronJobActions(handleJobUpdated, handleJobDeleted);

  // Computed values
  const activeCount = useMemo(() => jobs.filter((j) => j.enabled).length, [jobs]);
  const hasError = useMemo(() => jobs.some(isJobErrorLike), [jobs]);

  return {
    jobs,
    loading,
    activeCount,
    hasError,
    refetch: fetchJobs,
    ...actions,
  };
}

/**
 * Hook for getting cron job status for all conversations
 * Used by ChatHistory to show indicators
 */
export function useCronJobsMap() {
  const [jobsMap, setJobsMap] = useState<Map<ConversationId, ICronJob[]>>(new Map());
  const [loading, setLoading] = useState(true);
  const pendingRef = useRef<CronJobChanges | null>(null);
  const unreadStorageKey = browserStorageGenerationKey('cron-unread');
  // Track conversations with unread cron executions (red dot indicator)
  const [unreadConversations, setUnreadConversations] = useState<Set<ConversationId>>(() => {
    // Restore only from the current backend dataset generation. The old
    // unscoped key is intentionally not read after a hard database reset.
    try {
      const stored = localStorage.getItem(unreadStorageKey);
      if (stored) {
        const parsed = JSON.parse(stored);
        if (Array.isArray(parsed)) {
          const ids = parsed.flatMap((value) => {
            try {
              return [parseConversationId(value)];
            } catch {
              return [];
            }
          });
          return new Set(ids);
        }
      }
    } catch {
      // ignore
    }
    return new Set<ConversationId>();
  });
  // Track last_run_at_ms for each job to detect new executions
  const lastRunAtMapRef = useRef<Map<CronJobId, number>>(new Map());
  // Track current active conversation (use ref to access latest value in event handlers)
  const activeConversationIdRef = useRef<ConversationId | null>(null);

  // Persist unread state to localStorage
  useEffect(() => {
    try {
      localStorage.setItem(unreadStorageKey, JSON.stringify([...unreadConversations]));
    } catch {
      // ignore
    }
  }, [unreadConversations, unreadStorageKey]);

  // Fetch all jobs and group by conversation
  const fetchAllJobs = useCallback(async () => {
    const changes: CronJobChanges = new Map();
    pendingRef.current = changes;
    setLoading(true);
    try {
      const allJobs = await ipcBridge.cron.listJobs.invoke();
      if (pendingRef.current !== changes) return;
      const repaired = await repairCronJobTimeZones(mergeCronSnapshot(allJobs || [], changes));
      if (pendingRef.current !== changes) return;
      const jobs = mergeCronSnapshot(repaired, changes);
      const map = indexCronJobsByConversation(jobs);

      lastRunAtMapRef.current.clear();
      for (const job of jobs) {
        // Initialize lastRunAtMap for detecting new executions
        if (job.state.last_run_at_ms) {
          lastRunAtMapRef.current.set(job.cron_job_id, job.state.last_run_at_ms);
        }
      }

      setJobsMap(map);
    } catch (err) {
      if (pendingRef.current === changes) console.error('[useCronJobsMap] Failed to fetch jobs:', err);
    } finally {
      if (pendingRef.current === changes) {
        pendingRef.current = null;
        setLoading(false);
      }
    }
  }, []);

  // Initial fetch
  useEffect(() => {
    void fetchAllJobs();
    return () => { pendingRef.current = null; };
  }, [fetchAllJobs]);

  // Event handlers
  const eventHandlers = useMemo<CronJobEventHandlers>(
    () => ({
      onJobCreated: (job: ICronJob) => {
        pendingRef.current?.set(job.cron_job_id, job);
        const convId = job.metadata.conversation_id;
        if (!convId) return;
        setJobsMap((prev) => upsertCronJobByConversation(prev, job));
        // Refresh conversation list to update sorting (modifyTime was updated)
        console.log('[useCronJobsMap] onJobCreated, triggering chat.history.refresh');
        emitter.emit('chat.history.refresh');
      },
      onJobUpdated: (job: ICronJob) => {
        pendingRef.current?.set(job.cron_job_id, job);
        const convId = job.metadata.conversation_id;

        // Check if this is a new execution (last_run_at_ms changed)
        const prevLastRunAt = lastRunAtMapRef.current.get(job.cron_job_id);
        const newLastRunAt = job.state.last_run_at_ms;
        if (convId && newLastRunAt && newLastRunAt !== prevLastRunAt) {
          lastRunAtMapRef.current.set(job.cron_job_id, newLastRunAt);

          // Mark as unread only if user is not currently viewing this conversation
          // Use ref to access the latest activeConversationId value
          if (activeConversationIdRef.current !== convId) {
            setUnreadConversations((prev) => {
              if (prev.has(convId)) return prev;
              const newSet = new Set(prev);
              newSet.add(convId);
              return newSet;
            });
          }

          // Refresh conversation list to update sorting (modifyTime was updated after execution)
          emitter.emit('chat.history.refresh');
        }

        setJobsMap((prev) => upsertCronJobByConversation(prev, job));
      },
      onJobRemoved: ({ cron_job_id }: { cron_job_id: CronJobId }) => {
        pendingRef.current?.set(cron_job_id, null);
        lastRunAtMapRef.current.delete(cron_job_id);
        setJobsMap((prev) => {
          const newMap = new Map(prev);
          for (const [convId, convJobs] of newMap.entries()) {
            const filtered = convJobs.filter((j) => j.cron_job_id !== cron_job_id);
            if (filtered.length === 0) {
              newMap.delete(convId);
            } else if (filtered.length !== convJobs.length) {
              newMap.set(convId, filtered);
            }
          }
          return newMap;
        });
      },
    }),
    []
  );

  useCronJobSubscription(eventHandlers, fetchAllJobs);

  // Helper functions
  const hasJobsForConversation = useCallback(
    (conversation_id: ConversationId) => {
      return jobsMap.has(conversation_id) && jobsMap.get(conversation_id)!.length > 0;
    },
    [jobsMap]
  );

  const getJobsForConversation = useCallback(
    (conversation_id: ConversationId): ICronJob[] => {
      return jobsMap.get(conversation_id) || [];
    },
    [jobsMap]
  );

  const getJobStatus = useCallback(
    (conversation_id: ConversationId): 'none' | 'active' | 'paused' | 'error' | 'unread' => {
      const convJobs = jobsMap.get(conversation_id);
      if (!convJobs || convJobs.length === 0) {
        return 'none';
      }

      // Check if conversation has unread cron executions (highest priority for visual indicator)
      if (unreadConversations.has(conversation_id)) return 'unread';

      // Check if any job has error
      if (convJobs.some(isJobErrorLike)) return 'error';

      // Check if all jobs are paused
      if (convJobs.every((j) => !j.enabled)) return 'paused';

      return 'active';
    },
    [jobsMap, unreadConversations]
  );

  // Mark a conversation as read (clear unread status)
  const markAsRead = useCallback((conversation_id: ConversationId) => {
    activeConversationIdRef.current = conversation_id;
    setUnreadConversations((prev) => {
      if (!prev.has(conversation_id)) {
        return prev;
      }
      const newSet = new Set(prev);
      newSet.delete(conversation_id);
      return newSet;
    });
  }, []);

  // Update active conversation ref without triggering state update
  // Use this to sync the ref when route changes (e.g., URL navigation)
  const setActiveConversation = useCallback((conversation_id: ConversationId) => {
    activeConversationIdRef.current = conversation_id;
  }, []);

  // Check if a conversation has unread cron executions
  const hasUnread = useCallback(
    (conversation_id: ConversationId) => {
      return unreadConversations.has(conversation_id);
    },
    [unreadConversations]
  );

  return useMemo(
    () => ({
      jobsMap,
      loading,
      hasJobsForConversation,
      getJobsForConversation,
      getJobStatus,
      markAsRead,
      setActiveConversation,
      hasUnread,
      refetch: fetchAllJobs,
    }),
    [
      jobsMap,
      loading,
      hasJobsForConversation,
      getJobsForConversation,
      getJobStatus,
      markAsRead,
      setActiveConversation,
      hasUnread,
      fetchAllJobs,
    ]
  );
}

/**
 * Hook for fetching lightweight execution records for a specific cron job.
 * Each job is pruned server-side to its latest seven runs.
 */
export function useCronJobRuns(cron_job_id: CronJobId | undefined) {
  const [runs, setRuns] = useState<ICronJobRun[]>([]);
  const [loading, setLoading] = useState(false);
  const requestRef = useRef(0);

  const fetchRuns = useCallback(async () => {
    const request = ++requestRef.current;
    if (!cron_job_id) {
      setRuns([]);
      setLoading(false);
      return;
    }

    setLoading(true);
    try {
      const result = await ipcBridge.cron.listRuns.invoke({ cron_job_id: cron_job_id });
      if (request !== requestRef.current) return;
      setRuns(result || []);
    } catch (err) {
      if (request !== requestRef.current) return;
      console.error('[useCronJobRuns] Failed to fetch:', err);
      setRuns([]);
    } finally {
      if (request === requestRef.current) setLoading(false);
    }
  }, [cron_job_id]);

  // Initial fetch
  useEffect(() => {
    setRuns([]);
    void fetchRuns();
    return () => { requestRef.current += 1; };
  }, [fetchRuns]);

  // Refetch when this job executes. WebSocket delivery has no replay: also
  // reload the run history after any gap (reconnect, server lag resync).
  useEffect(() => {
    if (!cron_job_id) return;
    const unsubExecuted = ipcBridge.cron.onJobExecuted.on((data) => {
      if (data.cron_job_id === cron_job_id) {
        void fetchRuns();
      }
    });
    const unsubReconnected = ipcBridge.conversation.reconnected.on(() => void fetchRuns());
    return () => {
      unsubExecuted();
      unsubReconnected();
    };
  }, [cron_job_id, fetchRuns]);

  return { runs, loading };
}
