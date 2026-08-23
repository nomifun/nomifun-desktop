/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { parseProviderId } from '@/common/types/ids';

import type {
  CreativeChatModelReference,
  CreativeChatSessionReference,
} from '../../../domain';
import type { CreativeModelSelectionRef } from '../../../models';
import type { CreativeStudioAgentMessage } from '../../../agent';

export type CreativeCanvasAgentHistoryAuthority = 'current' | 'completed-pending-turn';

const sameModel = (
  left: CreativeChatModelReference,
  right: CreativeChatModelReference
): boolean => left.providerId === right.providerId && left.model === right.model;

const validatePlanningModelInput = (value: unknown): string => {
  if (
    typeof value !== 'string' ||
    value.length === 0 ||
    value.length > 262_144 ||
    value !== value.trim()
  ) {
    throw new TypeError(
      'Creative Studio Agent model input must be trimmed, non-empty, and at most 262144 characters'
    );
  }
  return value;
};

const copyPlanningSkillIds = (value: unknown): string[] => {
  if (!Array.isArray(value) || value.length > 8) {
    throw new TypeError('Creative Studio Agent skill ids must be an array of at most 8 items');
  }
  const skillIds = value.map((skillId, index) => {
    if (
      typeof skillId !== 'string' ||
      skillId.length === 0 ||
      skillId.length > 128 ||
      skillId !== skillId.trim() ||
      !/^[A-Za-z0-9._-]+$/.test(skillId)
    ) {
      throw new TypeError(
        `Creative Studio Agent skill id ${index} must be a trimmed 1-128 character ASCII id`
      );
    }
    return skillId;
  });
  if (new Set(skillIds).size !== skillIds.length) {
    throw new TypeError('Creative Studio Agent skill ids must be unique');
  }
  return skillIds;
};

/** Re-establish the branded NomiFun provider ID at the product/runtime boundary. */
export function creativeCanvasAgentModelSelection(
  model: CreativeChatModelReference | null
): CreativeModelSelectionRef | null {
  return model
    ? {
        providerId: parseProviderId(model.providerId),
        model: model.model,
      }
    : null;
}

export function createCreativeCanvasAgentSession(
  id: string,
  now: number
): CreativeChatSessionReference {
  return {
    id,
    title: '新对话',
    messageIds: [],
    model: null,
    pendingTurn: null,
    createdAt: now,
    updatedAt: now,
  };
}

export function creativeCanvasAgentSessionWithPendingTurn(input: {
  session: CreativeChatSessionReference;
  model: CreativeModelSelectionRef;
  idempotencyKey: string;
  prompt: string;
  modelInput?: string;
  skillIds?: readonly string[];
  now: number;
}): CreativeChatSessionReference {
  const prompt = input.prompt.trim();
  if (!prompt) throw new Error('Creative Studio Agent prompt must be non-empty');
  const modelInput = validatePlanningModelInput(
    input.modelInput === undefined ? prompt : input.modelInput
  );
  const skillIds = copyPlanningSkillIds(
    input.skillIds === undefined ? [] : input.skillIds
  );
  if (input.session.pendingTurn) {
    throw new Error('Creative Studio Agent session already has a pending turn');
  }
  if (input.session.model && !sameModel(input.session.model, input.model)) {
    throw new Error('Creative Studio Agent session model is immutable after its first turn');
  }
  const title =
    input.session.messageIds.length === 0 && input.session.title === '新对话'
      ? prompt.slice(0, 28)
      : input.session.title;
  return {
    ...input.session,
    title,
    model: { ...input.model },
    pendingTurn: {
      idempotencyKey: input.idempotencyKey,
      prompt,
      modelInput,
      skillIds,
      createdAt: input.now,
    },
    updatedAt: input.now,
  };
}

export function classifyCreativeCanvasAgentHistory(
  session: CreativeChatSessionReference,
  history: readonly CreativeStudioAgentMessage[]
): CreativeCanvasAgentHistoryAuthority {
  const ids = history.map((message) => {
    if (
      message.status !== 'complete' ||
      (message.role !== 'user' && message.role !== 'assistant')
    ) {
      throw new Error('Creative Studio Agent authority returned a non-durable message');
    }
    return message.id;
  });
  if (new Set(ids).size !== ids.length) {
    throw new Error('Creative Studio Agent authority returned duplicate message ids');
  }
  if (
    ids.length < session.messageIds.length ||
    session.messageIds.some((messageId, index) => ids[index] !== messageId)
  ) {
    throw new Error('Creative Studio Agent authority does not preserve Canvas message references');
  }
  const recoveredCount = ids.length - session.messageIds.length;
  if (recoveredCount === 0) return 'current';
  if (
    recoveredCount === 2 &&
    session.pendingTurn &&
    history.at(-2)?.role === 'user' &&
    history.at(-1)?.role === 'assistant'
  ) {
    return 'completed-pending-turn';
  }
  throw new Error('Creative Studio Agent authority returned an invalid pending-turn projection');
}

export function creativeCanvasAgentSessionWithAuthoritativeHistory(
  session: CreativeChatSessionReference,
  history: readonly CreativeStudioAgentMessage[],
  now: number
): CreativeChatSessionReference {
  if (classifyCreativeCanvasAgentHistory(session, history) !== 'completed-pending-turn') {
    throw new Error('Creative Studio Agent completion has no new durable message pair');
  }
  return {
    ...session,
    messageIds: history.map((message) => message.id),
    pendingTurn: null,
    updatedAt: now,
  };
}

export function creativeCanvasAgentSessionWithoutPendingTurn(
  session: CreativeChatSessionReference,
  now: number
): CreativeChatSessionReference {
  return session.pendingTurn
    ? { ...session, pendingTurn: null, updatedAt: now }
    : session;
}

export function replaceCreativeCanvasAgentSession(
  sessions: readonly CreativeChatSessionReference[],
  session: CreativeChatSessionReference
): CreativeChatSessionReference[] {
  const index = sessions.findIndex((candidate) => candidate.id === session.id);
  if (index < 0) return [...sessions, session];
  return sessions.map((candidate, candidateIndex) =>
    candidateIndex === index ? session : candidate
  );
}
