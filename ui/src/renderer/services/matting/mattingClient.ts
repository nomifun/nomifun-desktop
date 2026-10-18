/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ensureMattingModel, type ModelProgress } from './modelCache';
import { ML_TIMEOUT_MS, type MatteRequest, type MatteResponse } from './mattingProtocol';

const MAX_SOURCE_EDGE = 4096;

export async function createBitmapClamped(file: File, maxEdge: number): Promise<ImageBitmap> {
  const probe = await createImageBitmap(file);
  if (probe.width <= maxEdge && probe.height <= maxEdge) return probe;
  try {
    const scale = maxEdge / Math.max(probe.width, probe.height);
    return await createImageBitmap(probe, {
      resizeWidth: Math.max(1, Math.round(probe.width * scale)),
      resizeHeight: Math.max(1, Math.round(probe.height * scale)),
      resizeQuality: 'high',
    });
  } finally {
    probe.close();
  }
}

interface MattingWorker {
  onmessage: ((event: MessageEvent<MatteResponse>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  postMessage(message: MatteRequest, transfer: Transferable[]): void;
  terminate(): void;
}

type ClientDependencies = {
  createWorker: () => MattingWorker;
  acquireModel: (onProgress?: ModelProgress, signal?: AbortSignal) => Promise<Blob>;
  decode: (file: File, maxEdge: number) => Promise<ImageBitmap>;
  timeoutMs: number;
};

type Job = {
  request: { id: number; file: File; mode: 'auto' | 'heuristic'; tolerance: number };
  receive: (message: MatteResponse) => void;
  cancellation: AbortController;
  timer?: ReturnType<typeof setTimeout>;
  recycleWorker?: boolean;
};

type WithoutId<T> = T extends { id: number } ? Omit<T, 'id'> : never;

/** Owns cancellable preparation and a single reusable inference worker. */
export class MattingClient {
  private worker?: MattingWorker;
  private active?: Job;
  private readonly dependencies: ClientDependencies;

  constructor(dependencies: Partial<ClientDependencies> = {}) {
    this.dependencies = {
      createWorker: () => new Worker(new URL('./matting.worker.ts', import.meta.url), { type: 'module' }),
      acquireModel: ensureMattingModel,
      decode: createBitmapClamped,
      timeoutMs: ML_TIMEOUT_MS,
      ...dependencies,
    };
  }

  run(request: Job['request'], receive: Job['receive']): void {
    this.cancel();
    const job: Job = { request, receive, cancellation: new AbortController() };
    this.active = job;
    void this.start(job);
  }

  cancel(): void {
    if (!this.active) return;
    this.clearDeadline(this.active);
    this.active.cancellation.abort();
    this.active = undefined;
    // Termination is the only way to stop synchronous WASM. A new job must
    // never compete with an abandoned inference for the same worker/session.
    this.dropWorker();
  }

  dispose(): void {
    this.cancel();
    this.dropWorker();
  }

  private dropWorker(): void {
    this.worker?.terminate();
    this.worker = undefined;
  }

  private clearDeadline(job: Job): void {
    if (job.timer !== undefined) clearTimeout(job.timer);
    job.timer = undefined;
  }

  private emit(job: Job, message: WithoutId<MatteResponse> | MatteResponse): void {
    if (this.active === job) job.receive({ ...message, id: job.request.id } as MatteResponse);
  }

  private complete(job: Job, message: WithoutId<MatteResponse>): void {
    if (this.active !== job) return;
    this.clearDeadline(job);
    if (job.recycleWorker) this.dropWorker();
    this.active = undefined;
    // A terminal callback may synchronously start another job. Settle the old
    // lifecycle before publishing, so it cannot clear or terminate its successor.
    job.receive({ ...message, id: job.request.id } as MatteResponse);
  }

  private async start(job: Job): Promise<void> {
    let mode = job.request.mode;
    let model: Blob | undefined;
    if (mode === 'auto') {
      this.emit(job, { type: 'progress', phase: 'download' });
      try {
        model = await this.dependencies.acquireModel((loaded, total) => {
          this.emit(job, { type: 'progress', phase: 'download', loaded, total });
        }, job.cancellation.signal);
      } catch (error) {
        if (this.active !== job) return;
        mode = 'heuristic';
        this.emit(job, { type: 'fallback', reason: 'model_unavailable', detail: String(error) });
      }
    }
    await this.dispatch(job, mode, model);
  }

  private async dispatch(job: Job, mode: 'auto' | 'heuristic', model?: Blob): Promise<void> {
    if (this.active !== job) return;
    this.emit(job, { type: 'progress', phase: 'process' });
    let bitmap: ImageBitmap | undefined;
    try {
      bitmap = await this.dependencies.decode(job.request.file, MAX_SOURCE_EDGE);
      if (this.active !== job) return;
      if (!this.worker) {
        const worker = this.dependencies.createWorker();
        this.worker = worker;
        worker.onmessage = event => {
          if (this.worker === worker) this.handleMessage(event.data);
        };
        worker.onerror = event => {
          if (this.worker !== worker) return;
          const current = this.active;
          if (!current) return;
          this.clearDeadline(current);
          this.dropWorker();
          this.complete(current, { type: 'error', message: event.message || 'Matting worker failed' });
        };
      }
      if (mode === 'auto') {
        job.timer = setTimeout(() => {
          if (this.active !== job) return;
          this.clearDeadline(job);
          this.dropWorker();
          this.emit(job, { type: 'fallback', reason: 'inference_timeout' });
          // The transferred ImageBitmap belongs to the terminated worker.
          // Decode a fresh one for the recoverable quick-cutout attempt.
          void this.dispatch(job, 'heuristic');
        }, this.dependencies.timeoutMs);
      }
      this.worker.postMessage({
        id: job.request.id, type: 'matte', bitmap, mode, model, tolerance: job.request.tolerance,
      }, [bitmap]);
      bitmap = undefined;
    } catch (error) {
      if (this.active !== job) return;
      this.clearDeadline(job);
      this.dropWorker();
      this.complete(job, { type: 'error', message: String(error) });
    } finally {
      bitmap?.close();
    }
  }

  private handleMessage(message: MatteResponse): void {
    const job = this.active;
    if (!job || message.id !== job.request.id) return;
    if (message.type !== 'progress' || message.phase === 'process') this.clearDeadline(job);
    if (message.type === 'fallback' && message.reason === 'inference_failed') job.recycleWorker = true;
    if (message.type === 'done' || message.type === 'error') {
      this.complete(job, message);
    } else {
      this.emit(job, message);
    }
  }
}
