/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeModelSelectionRef } from '../models';
import type { CreativeStudioAgentMessage } from './types';

export interface CreativeStudioAgentTurnRequest {
  projectId: string;
  sessionId: string;
  /** Durable UUIDv7 persisted in the project before this turn is submitted. */
  idempotencyKey: string;
  /** Human-readable text persisted for the Creative Studio transcript. */
  prompt: string;
  /** Exact durable planning envelope submitted to the NomiFun model. */
  modelInput: string;
  /** Ordered durable skill injection snapshot for this exact turn. */
  skillIds: readonly string[];
  model: CreativeModelSelectionRef;
  history: readonly CreativeStudioAgentMessage[];
  signal: AbortSignal;
}
export type CreativeStudioAgentTurnEvent =
  | { type: 'activity'; label: string }
  | { type: 'assistant-delta'; delta: string }
  | { type: 'history-reconciled'; history: readonly CreativeStudioAgentMessage[] }
  | { type: 'completed'; assistantMessageId?: string }
  | { type: 'failed'; message: string; code?: string; retryable?: boolean };

/**
 * Adapter point for the existing NomiFun conversation/agent runtime. The port
 * intentionally knows neither HTTP nor IPC and never fabricates a reply.
 */
export interface CreativeStudioAgentChatPort {
  runTurn(
    request: CreativeStudioAgentTurnRequest
  ):
    | AsyncIterable<CreativeStudioAgentTurnEvent>
    | Promise<AsyncIterable<CreativeStudioAgentTurnEvent>>;
}

export type CreativeStudioAgentTurnStatus =
  | { state: 'running' }
  | { state: 'completed' }
  | { state: 'stopped' }
  | { state: 'failed'; error: Error };

export interface CreativeStudioAgentTurnObserver {
  onEvent?(event: CreativeStudioAgentTurnEvent): void;
  onStatusChange?(status: CreativeStudioAgentTurnStatus): void;
}

export type CreativeStudioAgentTurnOutcome = Exclude<CreativeStudioAgentTurnStatus, { state: 'running' }>;

export class CreativeStudioAgentBusyError extends Error {
  constructor() {
    super('Creative Studio Agent already has an active turn');
    this.name = 'CreativeStudioAgentBusyError';
  }
}

export class CreativeStudioAgentProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CreativeStudioAgentProtocolError';
  }
}

export class CreativeStudioAgentRemoteError extends Error {
  readonly code?: string;
  readonly retryable: boolean;

  constructor(event: Extract<CreativeStudioAgentTurnEvent, { type: 'failed' }>) {
    super(event.message);
    this.name = 'CreativeStudioAgentRemoteError';
    this.code = event.code;
    this.retryable = event.retryable ?? false;
  }
}

const isAbortError = (error: unknown, signal: AbortSignal): boolean =>
  signal.aborted || (error instanceof Error && error.name === 'AbortError');

/**
 * One-turn coordinator shared by future React bindings and the NomiFun chat
 * adapter. A successful outcome requires an explicit `completed` event; an
 * exhausted or failed stream can never be presented as a successful reply.
 */
export class CreativeStudioAgentChatController {
  private activeController: AbortController | null = null;

  constructor(private readonly port: CreativeStudioAgentChatPort) {}

  get isRunning(): boolean {
    return this.activeController !== null;
  }

  stop(): void {
    this.activeController?.abort();
  }

  async runTurn(
    input: Omit<CreativeStudioAgentTurnRequest, 'signal'>,
    observer: CreativeStudioAgentTurnObserver = {}
  ): Promise<CreativeStudioAgentTurnOutcome> {
    if (this.activeController) throw new CreativeStudioAgentBusyError();

    const controller = new AbortController();
    this.activeController = controller;
    observer.onStatusChange?.({ state: 'running' });

    try {
      const stream = await this.port.runTurn({ ...input, signal: controller.signal });
      let completed = false;

      for await (const event of stream) {
        if (controller.signal.aborted) {
          const abortError = new Error('Creative Studio Agent turn stopped');
          abortError.name = 'AbortError';
          throw abortError;
        }
        observer.onEvent?.(event);
        if (event.type === 'failed') throw new CreativeStudioAgentRemoteError(event);
        if (event.type === 'completed') completed = true;
      }

      if (!completed) {
        throw new CreativeStudioAgentProtocolError(
          'Creative Studio Agent stream ended without a completed event'
        );
      }

      const outcome = { state: 'completed' } as const;
      observer.onStatusChange?.(outcome);
      return outcome;
    } catch (error) {
      if (isAbortError(error, controller.signal)) {
        const outcome = { state: 'stopped' } as const;
        observer.onStatusChange?.(outcome);
        return outcome;
      }
      const normalized = error instanceof Error ? error : new Error(String(error));
      const outcome = { state: 'failed', error: normalized } as const;
      observer.onStatusChange?.(outcome);
      return outcome;
    } finally {
      if (this.activeController === controller) this.activeController = null;
    }
  }
}
