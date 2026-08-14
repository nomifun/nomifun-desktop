import type { ConversationId, MessageId } from '@/common/types/ids';
import { MAX_NOMI_PENDING_POST_PROCESSES } from './nomiMessageBuffer';

export type NomiTerminalPostProcessRequest = {
  conversationId: ConversationId;
  terminalId: MessageId;
  targetMessageId?: MessageId;
  turnId?: MessageId;
  /**
   * Legacy terminals without `final_text_msg_id` may need to bind a late
   * fragment by the owning turn. Once the backend supplies an explicit target,
   * this remains false so a request cannot capture another segment.
   */
  allowTurnFallback: boolean;
  generation: number;
  turnStartGeneration: number;
};

export type NomiInFlightPostProcess = NomiTerminalPostProcessRequest & {
  targetMessageId: MessageId;
  bufferVersion: number;
};

/**
 * A completed local fallback is still observable for one exact target.  It is
 * not a pending job anymore: reconnects must not keep retrying it, while a
 * fragment that advances the same target's buffer version must be able to
 * wake a fresh attempt.
 */
export type NomiPostProcessObservation = NomiInFlightPostProcess;

export type NomiPostProcessState = {
  inFlight: Map<string, NomiInFlightPostProcess>;
  pending: Map<string, NomiTerminalPostProcessRequest>;
  waitingForInFlight: Set<string>;
  observed: Map<string, NomiPostProcessObservation>;
  processed: Map<string, number>;
};

/**
 * Only an explicit boolean `true` is a backend authority claim.
 *
 * `false` is intentionally not equivalent to authority: it denotes a
 * terminal that did not complete the backend projection and must remain on the
 * legacy/association path (when otherwise eligible). An omitted marker is the
 * pre-contract legacy wire shape.
 */
export const isNomiBackendFinalTextAuthoritative = (
  value: boolean | undefined
): value is true => value === true;

export const createNomiPostProcessState = (): NomiPostProcessState => ({
  inFlight: new Map(),
  pending: new Map(),
  waitingForInFlight: new Set(),
  observed: new Map(),
  processed: new Map(),
});

const rememberBounded = <T>(
  map: Map<string, T>,
  key: string,
  value: T,
  maxSize = MAX_NOMI_PENDING_POST_PROCESSES
): void => {
  map.delete(key);
  map.set(key, value);
  while (map.size > maxSize) {
    const oldest = map.keys().next().value;
    if (oldest === undefined) break;
    map.delete(oldest);
  }
};

export const rememberNomiPostProcessPending = (
  state: NomiPostProcessState,
  request: NomiTerminalPostProcessRequest
): void => {
  rememberBounded(state.pending, request.terminalId, request);
  for (const terminalId of state.waitingForInFlight) {
    if (!state.pending.has(terminalId)) state.waitingForInFlight.delete(terminalId);
  }
};

export const forgetNomiPostProcessPending = (
  state: NomiPostProcessState,
  terminalId: string
): void => {
  state.pending.delete(terminalId);
  state.waitingForInFlight.delete(terminalId);
};

export const markNomiPostProcessWaitingForInFlight = (
  state: NomiPostProcessState,
  terminalId: string
): void => {
  if (!state.pending.has(terminalId)) return;
  state.waitingForInFlight.delete(terminalId);
  state.waitingForInFlight.add(terminalId);
  while (state.waitingForInFlight.size > MAX_NOMI_PENDING_POST_PROCESSES) {
    const oldest = state.waitingForInFlight.values().next().value;
    if (oldest === undefined) break;
    state.waitingForInFlight.delete(oldest);
  }
};

export const getNomiPostProcessInFlightWaiters = (
  state: NomiPostProcessState
): NomiTerminalPostProcessRequest[] => {
  const waiters: NomiTerminalPostProcessRequest[] = [];
  for (const terminalId of state.waitingForInFlight) {
    const request = state.pending.get(terminalId);
    if (!request) {
      state.waitingForInFlight.delete(terminalId);
      continue;
    }
    waiters.push(request);
  }
  return waiters;
};

export const rememberNomiPostProcessObservation = (
  state: NomiPostProcessState,
  observation: NomiPostProcessObservation
): void => {
  rememberBounded(state.observed, observation.terminalId, observation);
};

export const tryRememberNomiInFlightPostProcess = (
  state: NomiPostProcessState,
  request: NomiInFlightPostProcess
): boolean => {
  // Running work must never be evicted to make room: its async completion
  // still owns exact version/generation cleanup. Leave overflow requests in
  // the bounded pending map until a real in-flight slot is released.
  if (
    state.inFlight.has(request.terminalId)
  ) {
    return false;
  }
  if (state.inFlight.size >= MAX_NOMI_PENDING_POST_PROCESSES) {
    markNomiPostProcessWaitingForInFlight(state, request.terminalId);
    return false;
  }
  state.waitingForInFlight.delete(request.terminalId);
  state.inFlight.set(request.terminalId, request);
  return true;
};

export const discardNomiPostProcessTerminal = (
  state: NomiPostProcessState,
  predicate: (request: NomiTerminalPostProcessRequest) => boolean
): void => {
  for (const [terminalId, request] of state.pending) {
    if (predicate(request)) state.pending.delete(terminalId);
  }
  for (const [terminalId, request] of state.inFlight) {
    if (predicate(request)) state.inFlight.delete(terminalId);
  }
  for (const [terminalId, request] of state.observed) {
    if (predicate(request)) state.observed.delete(terminalId);
  }
  for (const terminalId of state.waitingForInFlight) {
    if (!state.pending.has(terminalId)) state.waitingForInFlight.delete(terminalId);
  }
};

const requestMatchesBuffer = (
  request: NomiTerminalPostProcessRequest,
  messageId: string,
  turnId: string | undefined,
): boolean => {
  if (request.terminalId === messageId || request.targetMessageId === messageId) {
    return true;
  }

  // A legacy terminal may not carry the final text segment id.  Before a
  // buffer has ever been resolved, its owning turn is the only safe fallback
  // association.  Once a request has a concrete target, do not broaden that
  // association to every later segment in the turn.
  return (
    request.targetMessageId === undefined &&
    request.allowTurnFallback &&
    request.turnId !== undefined &&
    turnId !== undefined &&
    request.turnId === turnId
  );
};

export const isNomiPostProcessBufferAssociated = (
  state: NomiPostProcessState,
  conversationId: ConversationId,
  messageId: string | undefined,
  turnId?: string
): boolean => {
  if (!messageId) return false;

  for (const request of [...state.pending.values(), ...state.inFlight.values()]) {
    if (request.conversationId !== conversationId) continue;
    if (requestMatchesBuffer(request, messageId, turnId)) return true;
  }

  // Completed observations intentionally match only their exact terminal or
  // text target.  Keeping a successful observation out of `pending` prevents
  // reconnects from retrying an already-applied projection and prevents a
  // later unrelated text segment in the same turn from being captured by an
  // old fallback request.
  for (const observation of state.observed.values()) {
    if (
      observation.conversationId === conversationId &&
      requestMatchesBuffer(observation, messageId, turnId)
    ) {
      return true;
    }
  }

  return false;
};

export const takeNomiPostProcessObservationsForBuffer = (
  state: NomiPostProcessState,
  conversationId: ConversationId,
  messageId: string,
  turnId?: string
): NomiPostProcessObservation[] => {
  const matches: NomiPostProcessObservation[] = [];
  for (const [terminalId, observation] of state.observed) {
    if (
      observation.conversationId !== conversationId ||
      !requestMatchesBuffer(observation, messageId, turnId)
    ) {
      continue;
    }
    state.observed.delete(terminalId);
    matches.push(observation);
  }
  return matches;
};

export const promoteNomiPostProcessObservationsForBuffer = (
  state: NomiPostProcessState,
  conversationId: ConversationId,
  messageId: string,
  currentVersion: number,
  turnId?: string
): void => {
  for (const [terminalId, observation] of state.observed) {
    if (
      observation.conversationId !== conversationId ||
      !requestMatchesBuffer(observation, messageId, turnId) ||
      observation.bufferVersion === currentVersion
    ) {
      continue;
    }
    state.observed.delete(terminalId);
    rememberNomiPostProcessPending(state, {
      conversationId: observation.conversationId,
      terminalId: observation.terminalId,
      targetMessageId: observation.targetMessageId,
      turnId: observation.turnId,
      generation: observation.generation,
      turnStartGeneration: observation.turnStartGeneration,
      allowTurnFallback: observation.allowTurnFallback,
    });
    state.processed.delete(messageId);
  }
};

export const isNomiPostProcessRequestCurrent = (
  request: NomiTerminalPostProcessRequest,
  scope: {
    mounted: boolean;
    conversationId: ConversationId;
    generation: number;
    turnStartGeneration: number;
    rootTurnId?: MessageId | null;
    lastSettledTurnId?: MessageId | null;
    cancelledTurnIds?: ReadonlySet<MessageId>;
    backendTerminalIds?: ReadonlySet<string>;
    backendTerminalTurnIds?: ReadonlySet<string>;
  }
): boolean => {
  if (
    !scope.mounted ||
    scope.conversationId !== request.conversationId ||
    scope.generation !== request.generation ||
    scope.turnStartGeneration !== request.turnStartGeneration
  ) {
    return false;
  }
  if (request.turnId && scope.cancelledTurnIds?.has(request.turnId)) return false;
  if (
    scope.backendTerminalIds?.has(request.terminalId) ||
    (request.turnId !== undefined && scope.backendTerminalTurnIds?.has(request.turnId))
  ) {
    return false;
  }

  // A terminal may be observed just before the authoritative runtime settles
  // the turn, while its final text fragment arrives just after the settle
  // callback clears the active root. Keep the exact owning turn valid across
  // that narrow boundary, but never let a request from a foreign turn borrow
  // the currently active generation. Turnless legacy requests remain fenced
  // by their exact ids and generation because they have no owner to compare.
  if (request.turnId !== undefined) {
    const owningTurn = scope.rootTurnId ?? scope.lastSettledTurnId;
    return owningTurn !== undefined && owningTurn !== null && request.turnId === owningTurn;
  }
  return true;
};

export const isNomiInFlightPostProcessCurrent = (
  state: NomiPostProcessState,
  request: NomiInFlightPostProcess,
  scope: Parameters<typeof isNomiPostProcessRequestCurrent>[1],
  currentBuffer: { version?: number; turnId?: string }
): boolean =>
  isNomiPostProcessRequestCurrent(request, scope) &&
  state.inFlight.get(request.terminalId) === request &&
  currentBuffer.version === request.bufferVersion &&
  (!request.turnId || !currentBuffer.turnId || currentBuffer.turnId === request.turnId);

export const shouldHandleNomiTerminalPostProcess = (
  message: {
    type: string;
    data: unknown;
    msgId: string;
    turnId?: string;
    finalTextMsgId?: string;
    finalTextAuthoritative?: boolean;
  },
  scope: {
    rootTurnId?: string | null;
    lastSettledTurnId?: string | null;
    hasBuffer: (messageId: string) => boolean;
    isAssociated: (messageId: string | undefined, turnId?: string) => boolean;
  }
): boolean => {
  // The marker is a terminal-only contract. Treat a malformed ordinary frame
  // carrying it as projection-only rather than allowing it to suppress or
  // discard a local fallback request.
  if (isNomiBackendFinalTextAuthoritative(message.finalTextAuthoritative)) {
    return message.type === 'finish' || message.type === 'error';
  }
  // An explicit false is a negative authority claim, not permission to
  // suppress the compatibility projection. It may follow the legacy finish
  // association rules, but an Error can never become a completed assistant
  // post-process job.
  if (message.finalTextAuthoritative === false && message.type !== 'finish') return false;
  if (message.type !== 'finish') return false;
  if (
    message.data &&
    typeof message.data === 'object' &&
    !Array.isArray(message.data) &&
    (message.data as Record<string, unknown>).stop_reason === 'cancelled'
  ) {
    return false;
  }

  if (message.turnId) {
    return (
      message.turnId === scope.rootTurnId ||
      message.turnId === scope.lastSettledTurnId ||
      scope.isAssociated(message.msgId, message.turnId) ||
      scope.isAssociated(message.finalTextMsgId, message.turnId)
    );
  }

  // A turnless legacy terminal cannot be attached to whichever turn happens
  // to be active now. It needs an exact buffered/observed wire id, or an
  // explicit final-text target that can safely wait for a late fragment.
  return (
    message.finalTextMsgId !== undefined ||
    scope.hasBuffer(message.msgId) ||
    scope.isAssociated(message.msgId)
  );
};
