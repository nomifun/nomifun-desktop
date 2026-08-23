/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  serializeCreativeStudioAgentHistory,
  type NomiCreativeStudioAgentSessionResolution,
  type NomiCreativeStudioAgentSessionResolutionInput,
  type NomiCreativeStudioAgentSessionResolver,
} from '../adapters';
import type { CreativeModelSelectionRef } from '../../models';
import {
  CreativeStudioAgentSessionResolutionError,
  type CreativeStudioAgentSessionPersistencePort,
  type CreativeStudioAgentSessionPersistenceRequest,
} from './types';

const nonBlank = (value: string): boolean => value.trim().length > 0;

const sameModel = (
  left: CreativeModelSelectionRef,
  right: CreativeModelSelectionRef
): boolean => left.providerId === right.providerId && left.model === right.model;

const abortError = (): Error => {
  const error = new Error('Creative Studio Agent session resolution was aborted');
  error.name = 'AbortError';
  return error;
};

const assertInput = (input: NomiCreativeStudioAgentSessionResolutionInput): void => {
  if (
    !nonBlank(input.canvasId) ||
    !nonBlank(input.sessionId) ||
    !nonBlank(input.model.providerId) ||
    !nonBlank(input.model.model) ||
    (input.pendingTurnIdempotencyKey !== null &&
      !nonBlank(input.pendingTurnIdempotencyKey))
  ) {
    throw new CreativeStudioAgentSessionResolutionError(
      'INVALID_INPUT',
      'Creative Studio Agent Canvas, session, provider and model identifiers must be non-empty'
    );
  }
};

const assertResolution = (
  input: CreativeStudioAgentSessionPersistenceRequest,
  resolution: NomiCreativeStudioAgentSessionResolution
): void => {
  const { binding, history } = resolution;
  if (binding.ownership !== 'creative-studio-exclusive') {
    throw new CreativeStudioAgentSessionResolutionError(
      'PORT_CONTRACT_VIOLATION',
      'Session persistence returned a conversation without exclusive Creative Studio ownership'
    );
  }
  if (binding.canvasId !== input.canvasId || binding.sessionId !== input.sessionId) {
    throw new CreativeStudioAgentSessionResolutionError(
      'PORT_CONTRACT_VIOLATION',
      'Session persistence returned a binding for a different Creative Studio Canvas or session'
    );
  }
  if (!nonBlank(binding.conversationId)) {
    throw new CreativeStudioAgentSessionResolutionError(
      'PORT_CONTRACT_VIOLATION',
      'Session persistence returned an empty Nomi conversation identifier'
    );
  }
  if (!sameModel(binding.model, input.model)) {
    throw new CreativeStudioAgentSessionResolutionError(
      'PORT_CONTRACT_VIOLATION',
      'Session persistence returned a conversation with a different selected model'
    );
  }
  if (typeof resolution.created !== 'boolean') {
    throw new CreativeStudioAgentSessionResolutionError(
      'PORT_CONTRACT_VIOLATION',
      'Session persistence returned an invalid created marker'
    );
  }
  if (
    history.some(
      (message) =>
        message.status !== 'complete' ||
        (message.role !== 'user' && message.role !== 'assistant')
    )
  ) {
    throw new CreativeStudioAgentSessionResolutionError(
      'PORT_CONTRACT_VIOLATION',
      'Session persistence returned non-durable Agent history'
    );
  }
  const assistantIds = new Set(
    history
      .filter((message) => message.role === 'assistant')
      .map((message) => message.id)
  );
  if (
    new Set(resolution.appliedProposalMessageIds).size !==
      resolution.appliedProposalMessageIds.length ||
    resolution.appliedProposalMessageIds.some((messageId) => !assistantIds.has(messageId))
  ) {
    throw new CreativeStudioAgentSessionResolutionError(
      'PORT_CONTRACT_VIOLATION',
      'Applied proposals must be unique completed assistant messages from this history'
    );
  }
  const canonicalHistoryKey = serializeCreativeStudioAgentHistory(history);
  if (binding.historyKey !== canonicalHistoryKey) {
    throw new CreativeStudioAgentSessionResolutionError(
      'PORT_CONTRACT_VIOLATION',
      'Session persistence history does not match its binding proof'
    );
  }
};

const waitForCaller = <T>(operation: Promise<T>, signal: AbortSignal): Promise<T> => {
  if (signal.aborted) return Promise.reject(abortError());

  return new Promise<T>((resolve, reject) => {
    let settled = false;
    const finish = (callback: () => void) => {
      if (settled) return;
      settled = true;
      signal.removeEventListener('abort', onAbort);
      callback();
    };
    const onAbort = () => finish(() => reject(abortError()));
    signal.addEventListener('abort', onAbort, { once: true });
    void operation.then(
      (value) => finish(() => resolve(value)),
      (error: unknown) => finish(() => reject(error))
    );
  });
};

const operationKey = (request: CreativeStudioAgentSessionPersistenceRequest): string =>
  JSON.stringify([
    request.canvasId,
    request.sessionId,
    request.model.providerId,
    request.model.model,
    request.pendingTurnIdempotencyKey,
  ]);

/**
 * Coordinates durable resolution without becoming a second persistence store.
 * Only simultaneous requests for the same Canvas/session are coalesced; a
 * remount or a new controller always asks the injected durable port again.
 */
export class CreativeStudioAgentSessionController {
  private readonly inFlight = new Map<
    string,
    Promise<NomiCreativeStudioAgentSessionResolution>
  >();

  constructor(private readonly port: CreativeStudioAgentSessionPersistencePort) {}

  async resolve(
    input: NomiCreativeStudioAgentSessionResolutionInput
  ): Promise<NomiCreativeStudioAgentSessionResolution> {
    assertInput(input);
    if (input.signal.aborted) throw abortError();

    const request: CreativeStudioAgentSessionPersistenceRequest = {
      canvasId: input.canvasId,
      sessionId: input.sessionId,
      model: { ...input.model },
      pendingTurnIdempotencyKey: input.pendingTurnIdempotencyKey,
    };
    const key = operationKey(request);

    let operation = this.inFlight.get(key);
    if (!operation) {
      let tracked: Promise<NomiCreativeStudioAgentSessionResolution>;
      tracked = Promise.resolve()
        .then(() => this.port.resolveOrCreateExclusive(request))
        .finally(() => {
          if (this.inFlight.get(key) === tracked) this.inFlight.delete(key);
        });
      operation = tracked;
      this.inFlight.set(key, tracked);
    }

    const resolution = await waitForCaller(operation, input.signal);
    // Validate for every waiter. A request with different model/history cannot
    // inherit the first caller's proof merely because both target one session.
    assertResolution(request, resolution);
    return resolution;
  }
}

export const createCreativeStudioAgentSessionResolver = (
  port: CreativeStudioAgentSessionPersistencePort
): NomiCreativeStudioAgentSessionResolver => {
  const controller = new CreativeStudioAgentSessionController(port);
  return (input) => controller.resolve(input);
};
