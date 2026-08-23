/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "vitest";

import { parseProviderId } from "@/common/types/ids";
import { serializeCreativeStudioAgentHistory } from "../adapters";
import type { CreativeStudioAgentMessage } from "../types";
import {
  CreativeStudioAgentSessionResolutionError,
  createNomiCreativeStudioAgentSessionHttpPort,
  type CreativeStudioAgentSessionHttpTransport,
  type CreativeStudioAgentSessionPersistenceRequest,
} from ".";

const canvasId = "0190f5fe-7c00-7a00-8000-000000000701";
const sessionId = "0190f5fe-7c00-7a00-8000-000000000702";
const providerId = parseProviderId("0190f5fe-7c00-7a00-8000-000000000703");
const conversationId = "0190f5fe-7c00-7a00-8000-000000000704";
const userMessageId = "0190f5fe-7c00-7a00-8000-000000000705";
const priorAssistantMessageId = "0190f5fe-7c00-7a00-8000-000000000706";
const pendingKey = "0190f5fe-7c00-7a00-8000-000000000707";
const history: readonly CreativeStudioAgentMessage[] = [
  { id: userMessageId, role: "user", status: "complete", text: "制作海报" },
  {
    id: priorAssistantMessageId,
    role: "assistant",
    status: "complete",
    text: "开始制作",
  },
];
const request: CreativeStudioAgentSessionPersistenceRequest = {
  canvasId,
  sessionId,
  model: { providerId, model: "nomi-chat" },
  pendingTurnIdempotencyKey: null,
};

describe("Nomi Creative Studio Agent session HTTP port", () => {
  test("maps the strict snake-case wire contract into a branded binding", async () => {
    let wire: unknown;
    const transport: CreativeStudioAgentSessionHttpTransport = {
      async resolve(input) {
        wire = input;
        return {
          binding: {
            ownership: "creative-studio-exclusive",
            canvas_id: canvasId,
            session_id: sessionId,
            conversation_id: conversationId,
            model: { provider_id: providerId, model: "nomi-chat" },
            history_key: serializeCreativeStudioAgentHistory(history),
          },
          history,
          applied_proposal_message_ids: [priorAssistantMessageId],
          created: false,
        };
      },
    };

    const binding =
      await createNomiCreativeStudioAgentSessionHttpPort(
        transport,
      ).resolveOrCreateExclusive(request);

    expect(wire).toEqual({
      canvas_id: canvasId,
      session_id: sessionId,
      model: { provider_id: providerId, model: "nomi-chat" },
      pending_turn_idempotency_key: null,
    });
    expect(binding.binding.conversationId).toBe(conversationId);
    expect(binding.appliedProposalMessageIds).toEqual([priorAssistantMessageId]);
  });

  test("rejects a malformed backend conversation identity", async () => {
    const transport: CreativeStudioAgentSessionHttpTransport = {
      async resolve() {
        return {
          binding: {
            ownership: "creative-studio-exclusive",
            canvas_id: canvasId,
            session_id: sessionId,
            conversation_id: "not-a-conversation",
            model: { provider_id: providerId, model: "nomi-chat" },
            history_key: serializeCreativeStudioAgentHistory(history),
          },
          history,
          applied_proposal_message_ids: [],
          created: false,
        };
      },
    };
    const failure = await createNomiCreativeStudioAgentSessionHttpPort(
      transport,
    )
      .resolveOrCreateExclusive(request)
      .catch((error: unknown) => error);
    expect(failure instanceof Error).toBe(true);
  });

  test("returns complete server-authoritative history while a pending turn is fenced", async () => {
    const recoveredHistory: readonly CreativeStudioAgentMessage[] = [
      ...history,
      {
        id: "0190f5fe-7c00-7a00-8000-000000000708",
        role: "user",
        status: "complete",
        text: "继续完善",
      },
      {
        id: "0190f5fe-7c00-7a00-8000-000000000709",
        role: "assistant",
        status: "complete",
        text: "已经完成",
      },
    ];
    const pendingRequest = {
      ...request,
      pendingTurnIdempotencyKey: pendingKey,
    };
    const transport: CreativeStudioAgentSessionHttpTransport = {
      async resolve(input) {
        expect(input.pending_turn_idempotency_key).toBe(pendingKey);
        return {
          binding: {
            ownership: "creative-studio-exclusive",
            canvas_id: canvasId,
            session_id: sessionId,
            conversation_id: conversationId,
            model: { provider_id: providerId, model: "nomi-chat" },
            history_key: serializeCreativeStudioAgentHistory(recoveredHistory),
          },
          history: recoveredHistory,
          applied_proposal_message_ids: [],
          created: false,
        };
      },
    };

    const resolution =
      await createNomiCreativeStudioAgentSessionHttpPort(
        transport,
      ).resolveOrCreateExclusive(pendingRequest);

    expect(resolution.history).toEqual(recoveredHistory);
  });

  test("rejects an applied receipt that is not a unique assistant history message", async () => {
    for (const appliedIds of [
      [userMessageId],
      [priorAssistantMessageId, priorAssistantMessageId],
      ["0190f5fe-7c00-7a00-8000-000000000799"],
    ]) {
      const transport: CreativeStudioAgentSessionHttpTransport = {
        async resolve() {
          return {
            binding: {
              ownership: "creative-studio-exclusive",
              canvas_id: canvasId,
              session_id: sessionId,
              conversation_id: conversationId,
              model: { provider_id: providerId, model: "nomi-chat" },
              history_key: serializeCreativeStudioAgentHistory(history),
            },
            history,
            applied_proposal_message_ids: appliedIds,
            created: false,
          };
        },
      };
      const failure = await createNomiCreativeStudioAgentSessionHttpPort(transport)
        .resolveOrCreateExclusive(request)
        .catch((error: unknown) => error);
      expect(failure instanceof CreativeStudioAgentSessionResolutionError).toBe(true);
      expect((failure as CreativeStudioAgentSessionResolutionError).code).toBe(
        "PORT_CONTRACT_VIOLATION",
      );
    }
  });

  test("rejects non-UUID Canvas/session input before transport", async () => {
    let calls = 0;
    const transport: CreativeStudioAgentSessionHttpTransport = {
      async resolve() {
        calls += 1;
        return {};
      },
    };
    const error = await createNomiCreativeStudioAgentSessionHttpPort(transport)
      .resolveOrCreateExclusive({ ...request, canvasId: "canvas-a" })
      .catch((failure: unknown) => failure);
    expect(error instanceof CreativeStudioAgentSessionResolutionError).toBe(
      true,
    );
    expect((error as CreativeStudioAgentSessionResolutionError).code).toBe(
      "INVALID_INPUT",
    );
    expect(calls).toBe(0);
  });

  test("rejects unknown response fields and an invalid pending fence before use", async () => {
    let calls = 0;
    const transport: CreativeStudioAgentSessionHttpTransport = {
      async resolve() {
        calls += 1;
        return {
          binding: {
            ownership: "creative-studio-exclusive",
            canvas_id: canvasId,
            session_id: sessionId,
            conversation_id: conversationId,
            model: { provider_id: providerId, model: "nomi-chat" },
            history_key: serializeCreativeStudioAgentHistory(history),
          },
          history,
          applied_proposal_message_ids: [],
          created: false,
          legacy_alias: true,
        };
      },
    };
    const responseFailure = await createNomiCreativeStudioAgentSessionHttpPort(
      transport,
    )
      .resolveOrCreateExclusive(request)
      .catch((failure: unknown) => failure);
    expect(
      responseFailure instanceof CreativeStudioAgentSessionResolutionError,
    ).toBe(true);
    expect(calls).toBe(1);

    const inputFailure = await createNomiCreativeStudioAgentSessionHttpPort(
      transport,
    )
      .resolveOrCreateExclusive({
        ...request,
        pendingTurnIdempotencyKey: "not-a-pending-key",
      })
      .catch((failure: unknown) => failure);
    expect(
      inputFailure instanceof CreativeStudioAgentSessionResolutionError,
    ).toBe(true);
    expect(
      (inputFailure as CreativeStudioAgentSessionResolutionError).code,
    ).toBe("INVALID_INPUT");
    expect(calls).toBe(1);
  });
});
