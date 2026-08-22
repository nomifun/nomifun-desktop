/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpPost } from "@/common/adapter/httpBridge";
import {
  CANONICAL_UUID_V7,
  parseConversationId,
  parseMessageId,
  parseProviderId,
  type MessageId,
} from "@/common/types/ids";

import { serializeCreativeStudioAgentHistory } from "../adapters";
import type { CreativeStudioAgentMessage } from "../types";
import {
  CreativeStudioAgentSessionResolutionError,
  type CreativeStudioAgentSessionPersistencePort,
  type CreativeStudioAgentSessionPersistenceRequest,
} from "./types";

interface WireRequest {
  project_id: string;
  session_id: string;
  model: { provider_id: string; model: string };
  pending_turn_idempotency_key: string | null;
}

interface CreativeStudioAgentSessionHttpTransport {
  resolve(request: WireRequest): Promise<unknown>;
}

const endpoint = httpPost<unknown, WireRequest>(
  "/api/creative-studio/agent-sessions/resolve",
);

const defaultTransport: CreativeStudioAgentSessionHttpTransport = {
  resolve: (request) => endpoint.invoke(request),
};

const record = (value: unknown, label: string): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new CreativeStudioAgentSessionResolutionError(
      "PORT_CONTRACT_VIOLATION",
      `${label} must be an object`,
    );
  }
  return value as Record<string, unknown>;
};

const string = (value: unknown, label: string): string => {
  if (typeof value !== "string") {
    throw new CreativeStudioAgentSessionResolutionError(
      "PORT_CONTRACT_VIOLATION",
      `${label} must be a string`,
    );
  }
  return value;
};

const exactKeys = (
  source: Record<string, unknown>,
  expected: readonly string[],
  label: string,
): void => {
  const expectedSet = new Set(expected);
  const unknown = Object.keys(source).filter((key) => !expectedSet.has(key));
  const missing = expected.filter((key) => !Object.hasOwn(source, key));
  if (unknown.length > 0 || missing.length > 0) {
    throw new CreativeStudioAgentSessionResolutionError(
      "PORT_CONTRACT_VIOLATION",
      `${label} has an invalid field set`,
    );
  }
};

const parseHistory = (
  value: unknown,
): readonly CreativeStudioAgentMessage[] => {
  if (!Array.isArray(value)) {
    throw new CreativeStudioAgentSessionResolutionError(
      "PORT_CONTRACT_VIOLATION",
      "Creative Studio session response history must be an array",
    );
  }
  return value.map((item, index) => {
    const source = record(item, `history[${index}]`);
    exactKeys(source, ["id", "role", "status", "text"], `history[${index}]`);
    const id = string(source.id, `history[${index}].id`);
    if (!CANONICAL_UUID_V7.test(id)) {
      throw new CreativeStudioAgentSessionResolutionError(
        "PORT_CONTRACT_VIOLATION",
        `history[${index}].id is not a canonical UUIDv7`,
      );
    }
    const role = string(source.role, `history[${index}].role`);
    const status = string(source.status, `history[${index}].status`);
    const text = string(source.text, `history[${index}].text`);
    if (status !== "complete" || (role !== "user" && role !== "assistant")) {
      throw new CreativeStudioAgentSessionResolutionError(
        "PORT_CONTRACT_VIOLATION",
        "The backend returned a non-durable Creative Studio history projection",
      );
    }
    return { id, role, status: "complete", text } as CreativeStudioAgentMessage;
  });
};

const parseAppliedProposalMessageIds = (
  value: unknown,
  history: readonly CreativeStudioAgentMessage[],
): readonly MessageId[] => {
  if (!Array.isArray(value)) {
    throw new CreativeStudioAgentSessionResolutionError(
      "PORT_CONTRACT_VIOLATION",
      "Creative Studio applied proposal IDs must be an array",
    );
  }
  const assistantIds = new Set(
    history
      .filter((message) => message.role === "assistant")
      .map((message) => message.id),
  );
  const seen = new Set<string>();
  return value.map((item, index) => {
    const id = string(item, `applied_proposal_message_ids[${index}]`);
    if (!CANONICAL_UUID_V7.test(id) || seen.has(id) || !assistantIds.has(id)) {
      throw new CreativeStudioAgentSessionResolutionError(
        "PORT_CONTRACT_VIOLATION",
        "Applied proposal IDs must be unique completed assistant messages from this history",
      );
    }
    seen.add(id);
    return parseMessageId(id);
  });
};

const validateBoundaryId = (value: string, label: string): void => {
  if (!CANONICAL_UUID_V7.test(value)) {
    throw new CreativeStudioAgentSessionResolutionError(
      "INVALID_INPUT",
      `${label} must be a canonical UUIDv7`,
    );
  }
};

export function createNomiCreativeStudioAgentSessionHttpPort(
  transport: CreativeStudioAgentSessionHttpTransport = defaultTransport,
): CreativeStudioAgentSessionPersistencePort {
  return {
    async resolveOrCreateExclusive(
      request: CreativeStudioAgentSessionPersistenceRequest,
    ) {
      validateBoundaryId(request.projectId, "projectId");
      validateBoundaryId(request.sessionId, "sessionId");
      if (request.pendingTurnIdempotencyKey !== null) {
        validateBoundaryId(
          request.pendingTurnIdempotencyKey,
          "pendingTurnIdempotencyKey",
        );
      }
      parseProviderId(request.model.providerId);
      if (
        !request.model.model ||
        request.model.model.trim() !== request.model.model
      ) {
        throw new CreativeStudioAgentSessionResolutionError(
          "INVALID_INPUT",
          "model must be trimmed and non-empty",
        );
      }
      const response = record(
        await transport.resolve({
          project_id: request.projectId,
          session_id: request.sessionId,
          model: {
            provider_id: request.model.providerId,
            model: request.model.model,
          },
          pending_turn_idempotency_key: request.pendingTurnIdempotencyKey,
        }),
        "Creative Studio session response",
      );
      exactKeys(
        response,
        ["binding", "history", "applied_proposal_message_ids", "created"],
        "Creative Studio session response",
      );
      const binding = record(
        response.binding,
        "Creative Studio session binding",
      );
      exactKeys(
        binding,
        [
          "ownership",
          "project_id",
          "session_id",
          "conversation_id",
          "model",
          "history_key",
        ],
        "Creative Studio session binding",
      );
      const model = record(
        binding.model,
        "Creative Studio session binding model",
      );
      exactKeys(
        model,
        ["provider_id", "model"],
        "Creative Studio session binding model",
      );
      if (typeof response.created !== "boolean") {
        throw new CreativeStudioAgentSessionResolutionError(
          "PORT_CONTRACT_VIOLATION",
          "Creative Studio session response created must be a boolean",
        );
      }
      const history = parseHistory(response.history);
      const appliedProposalMessageIds = parseAppliedProposalMessageIds(
        response.applied_proposal_message_ids,
        history,
      );
      const historyKey = string(binding.history_key, "binding.history_key");
      const ownership = string(binding.ownership, "binding.ownership");
      const boundProjectId = string(binding.project_id, "binding.project_id");
      const boundSessionId = string(binding.session_id, "binding.session_id");
      const boundProviderId = parseProviderId(model.provider_id);
      const boundModel = string(model.model, "binding.model.model");
      if (serializeCreativeStudioAgentHistory(history) !== historyKey) {
        throw new CreativeStudioAgentSessionResolutionError(
          "PORT_CONTRACT_VIOLATION",
          "The backend history projection does not match its history_key",
        );
      }
      if (
        ownership !== "creative-studio-exclusive" ||
        boundProjectId !== request.projectId ||
        boundSessionId !== request.sessionId ||
        boundProviderId !== request.model.providerId ||
        boundModel !== request.model.model
      ) {
        throw new CreativeStudioAgentSessionResolutionError(
          "PORT_CONTRACT_VIOLATION",
          "The backend returned a Creative Studio binding outside the requested authority",
        );
      }

      return {
        binding: {
          ownership: "creative-studio-exclusive",
          projectId: boundProjectId,
          sessionId: boundSessionId,
          conversationId: parseConversationId(binding.conversation_id),
          model: {
            providerId: boundProviderId,
            model: boundModel,
          },
          historyKey,
        },
        history,
        appliedProposalMessageIds,
        created: response.created,
      };
    },
  };
}

export type { CreativeStudioAgentSessionHttpTransport, WireRequest };
