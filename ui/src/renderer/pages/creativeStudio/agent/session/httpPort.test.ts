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

const projectId = "0190f5fe-7c00-7a00-8000-000000000701";
const sessionId = "0190f5fe-7c00-7a00-8000-000000000702";
const providerId = parseProviderId("0190f5fe-7c00-7a00-8000-000000000703");
const conversationId = "0190f5fe-7c00-7a00-8000-000000000704";
const userMessageId = "0190f5fe-7c00-7a00-8000-000000000705";
const history: readonly CreativeStudioAgentMessage[] = [
  { id: userMessageId, role: "user", status: "complete", text: "制作海报" },
];
const request: CreativeStudioAgentSessionPersistenceRequest = {
  projectId,
  sessionId,
  model: { providerId, model: "nomi-chat" },
  history,
  historyKey: serializeCreativeStudioAgentHistory(history),
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
            project_id: projectId,
            session_id: sessionId,
            conversation_id: conversationId,
            model: { provider_id: providerId, model: "nomi-chat" },
            history_key: request.historyKey,
          },
          history,
          created: false,
        };
      },
    };

    const binding =
      await createNomiCreativeStudioAgentSessionHttpPort(
        transport,
      ).resolveOrCreateExclusive(request);

    expect(wire).toEqual({
      project_id: projectId,
      session_id: sessionId,
      model: { provider_id: providerId, model: "nomi-chat" },
      history,
      history_key: request.historyKey,
    });
    expect(binding.conversationId).toBe(conversationId);
  });

  test("rejects a malformed backend conversation identity", async () => {
    const transport: CreativeStudioAgentSessionHttpTransport = {
      async resolve() {
        return {
          binding: {
            ownership: "creative-studio-exclusive",
            project_id: projectId,
            session_id: sessionId,
            conversation_id: "not-a-conversation",
            model: { provider_id: providerId, model: "nomi-chat" },
            history_key: request.historyKey,
          },
          history,
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

  test("rejects non-UUID project/session input before transport", async () => {
    let calls = 0;
    const transport: CreativeStudioAgentSessionHttpTransport = {
      async resolve() {
        calls += 1;
        return {};
      },
    };
    const error = await createNomiCreativeStudioAgentSessionHttpPort(transport)
      .resolveOrCreateExclusive({ ...request, projectId: "project-a" })
      .catch((failure: unknown) => failure);
    expect(error instanceof CreativeStudioAgentSessionResolutionError).toBe(
      true,
    );
    expect((error as CreativeStudioAgentSessionResolutionError).code).toBe(
      "INVALID_INPUT",
    );
    expect(calls).toBe(0);
  });

  test("rejects unknown response fields and non-durable input before use", async () => {
    let calls = 0;
    const transport: CreativeStudioAgentSessionHttpTransport = {
      async resolve() {
        calls += 1;
        return {
          binding: {
            ownership: "creative-studio-exclusive",
            project_id: projectId,
            session_id: sessionId,
            conversation_id: conversationId,
            model: { provider_id: providerId, model: "nomi-chat" },
            history_key: request.historyKey,
          },
          history,
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
        history: [
          {
            id: userMessageId,
            role: "assistant",
            status: "running",
            text: "",
            activityLabel: "生成中",
          },
        ],
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
