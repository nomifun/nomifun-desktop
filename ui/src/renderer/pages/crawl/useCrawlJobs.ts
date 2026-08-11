/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ICrawlJob, ICreateCrawlJobParams } from '@/common/adapter/ipcBridge';
import type { CrawlJobId } from '@/common/types/ids';
import { useCallback, useEffect, useRef, useState } from 'react';

/** A job is actively consuming its queue and should keep polling. */
export const isJobActive = (job: ICrawlJob): boolean => job.status === 'running';

export interface CrawlJobsResult {
  jobs: ICrawlJob[];
  loading: boolean;
  error?: string;
  reload: () => Promise<void>;
  createJob: (params: ICreateCrawlJobParams) => Promise<ICrawlJob>;
  startJob: (jobId: CrawlJobId) => Promise<void>;
  cancelJob: (jobId: CrawlJobId) => Promise<void>;
  deleteJob: (jobId: CrawlJobId) => Promise<void>;
  retryFailed: (jobId: CrawlJobId) => Promise<number>;
}

export function useCrawlJobs(): CrawlJobsResult {
  const [jobs, setJobs] = useState<ICrawlJob[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | undefined>();
  const mounted = useRef(true);

  const reload = useCallback(async () => {
    try {
      const next = await ipcBridge.crawl.listJobs.invoke();
      if (!mounted.current) return;
      setJobs(next ?? []);
      setError(undefined);
    } catch (e) {
      if (!mounted.current) return;
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (mounted.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void reload();
    return () => {
      mounted.current = false;
    };
  }, [reload]);

  // Progress arrives over WS, but a job list also has to survive a missed
  // frame — the server never replays. Poll only while something is running.
  useEffect(() => {
    const offProgress = ipcBridge.crawlEvents.progress.on((event) => {
      setJobs((prev) =>
        prev.map((job) =>
          job.job_id === event.job_id ? { ...job, progress: event.progress } : job
        )
      );
    });
    const offFinished = ipcBridge.crawlEvents.finished.on((event) => {
      setJobs((prev) =>
        prev.map((job) =>
          job.job_id === event.job_id
            ? { ...job, status: event.status, progress: event.progress }
            : job
        )
      );
    });
    return () => {
      offProgress?.();
      offFinished?.();
    };
  }, []);

  useEffect(() => {
    if (!jobs.some(isJobActive)) return;
    const timer = window.setInterval(() => void reload(), 5000);
    return () => window.clearInterval(timer);
  }, [jobs, reload]);

  const createJob = useCallback(
    async (params: ICreateCrawlJobParams) => {
      const job = await ipcBridge.crawl.createJob.invoke(params);
      await reload();
      return job;
    },
    [reload]
  );

  const startJob = useCallback(
    async (jobId: CrawlJobId) => {
      await ipcBridge.crawl.startJob.invoke({ job_id: jobId });
      await reload();
    },
    [reload]
  );

  const cancelJob = useCallback(
    async (jobId: CrawlJobId) => {
      await ipcBridge.crawl.cancelJob.invoke({ job_id: jobId });
      await reload();
    },
    [reload]
  );

  const deleteJob = useCallback(
    async (jobId: CrawlJobId) => {
      await ipcBridge.crawl.deleteJob.invoke({ job_id: jobId });
      await reload();
    },
    [reload]
  );

  const retryFailed = useCallback(
    async (jobId: CrawlJobId) => {
      const count = await ipcBridge.crawl.retryFailed.invoke({ job_id: jobId });
      await reload();
      return count ?? 0;
    },
    [reload]
  );

  return { jobs, loading, error, reload, createJob, startJob, cancelJob, deleteJob, retryFailed };
}
