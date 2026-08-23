/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import type { IProvider, ModelTask } from "@/common/config/storage";
import type { ProviderId } from "@/common/types/ids";

import type { CreativeAsset, CreativeAssetPort } from "../../assets";
import type { CreativeModelCatalogSnapshot } from "../../models";
import type {
  CreateCreativeTaskInput,
  CreativeStandaloneWorkbenchKind,
  CreativeTask,
  CreativeTaskOwner,
  CreativeTaskPort,
  CreativeTaskReference,
} from "../../tasks";
import {
  CreativeWorkbenchRuntimeController,
  createAudioWorkbenchRuntimeProps,
  imageWorkbenchReferencesFromAssets,
  mapImageWorkbenchRuntimeResults,
  prepareAudioWorkbenchRun,
  prepareImageWorkbenchRun,
  prepareStandaloneHistoryRetry,
  prepareVideoWorkbenchRun,
  resolveExactWorkbenchModel,
  workbenchRuntimeRequestPresentation,
} from ".";
import type { CreativeWorkbenchRuntimeSnapshot } from ".";

const PROVIDER_ID = "0190f5fe-7c00-7a00-8000-0000000000a1" as ProviderId;
const PROJECT_ID = "01990a9e-8ea3-7cb2-9d55-a66b0d27e001";
const NODE_ID = "01990a9e-8ea3-7cb2-9d55-a66b0d27e002";
const ASSET_A = "01990a9e-8ea3-7cb2-9d55-a66b0d27e003";
const ASSET_B = "01990a9e-8ea3-7cb2-9d55-a66b0d27e004";

const capability = (task: ModelTask) => ({
  task,
  traits: [],
  protocol: `test.${task}`,
  connection_role: "default",
  allow_cross_origin_credentials: false,
  provider_params: {},
  created_at: 1,
  updated_at: 1,
});

function catalog(...tasks: ModelTask[]): CreativeModelCatalogSnapshot {
  const provider: IProvider = {
    id: PROVIDER_ID,
    name: "Provider A",
    enabled: true,
    platform: "custom",
    base_url: "https://example.invalid",
    auth_scheme: "bearer",
    has_credentials: true,
    models: tasks.map((task, index) => ({
      provider_id: PROVIDER_ID,
      model: `${task}-model`,
      enabled: true,
      sort_order: index,
      capabilities: [capability(task)],
      created_at: 1,
      updated_at: 1,
    })),
  };
  return { status: "ready", providers: [provider], error: null };
}

function stepfunImageCatalog(): CreativeModelCatalogSnapshot {
  const provider: IProvider = {
    id: PROVIDER_ID,
    name: "StepFun",
    enabled: true,
    platform: "stepfun",
    base_url: "https://api.stepfun.com/v1",
    auth_scheme: "bearer",
    has_credentials: true,
    models: [
      {
        provider_id: PROVIDER_ID,
        model: "step-image-edit-2",
        enabled: true,
        sort_order: 0,
        capabilities: [
          {
            ...capability("image_generation"),
            protocol: "stepfun.images",
          },
        ],
        created_at: 1,
        updated_at: 1,
      },
    ],
  };
  return { status: "ready", providers: [provider], error: null };
}

const assets: CreativeAssetPort = {
  list: async () => ({ items: [], total: 0 }),
  upload: async () => {
    throw new Error("not used");
  },
  update: async () => {
    throw new Error("not used");
  },
  remove: async () => undefined,
  url: (assetId) => `nomifun://asset/${assetId}`,
};

function task(
  input: CreateCreativeTaskInput,
  status: CreativeTask["status"],
  options: {
    assetId?: string;
    error?: string;
    errorKind?: string;
    taskId?: string;
  } = {},
): CreativeTask {
  return {
    ...input,
    taskId: options.taskId ?? input.idempotencyKey,
    status,
    error:
      status === "failed"
        ? {
            kind: options.errorKind ?? "provider",
            message: options.error ?? "provider failed",
            httpStatus: 500,
          }
        : null,
    resultAssetIds: status === "succeeded" ? [options.assetId ?? ASSET_A] : [],
    attempt: 1,
    submittedAt: 1_000,
    startedAt: status === "queued" ? null : 1_100,
    finishedAt:
      status === "succeeded" || status === "failed" || status === "canceled"
        ? 1_500
        : null,
    deletedAt: null,
  };
}

const ownerScope = (owner?: CreativeTaskOwner) =>
  owner ? { owner } : { nodeId: NODE_ID };

const standaloneOwner = (
  workbenchKind: CreativeStandaloneWorkbenchKind,
): CreativeTaskOwner => ({
  kind: "standalone_workbench",
  workbenchKind,
});

function imagePlan(owner?: CreativeTaskOwner) {
  return prepareImageWorkbenchRun({
    catalog: catalog("image_generation"),
    ...(owner?.kind === "standalone_workbench" ? {} : { canvasId: PROJECT_ID }),
    ...ownerScope(owner),
    model: { providerId: PROVIDER_ID, model: "image_generation-model" },
    references: { bindings: [], assets: [] },
    operation: { task: "image_generation", capability: "t2i" },
    prompt: "A real prompt",
    interfaceMode: "images",
    quality: "high",
    width: 1024,
    height: 1024,
    aspectRatio: "1:1",
    count: 1,
  });
}

function videoPlan(taskCount = 2, owner?: CreativeTaskOwner) {
  return prepareVideoWorkbenchRun({
    catalog: catalog("video_generation"),
    ...(owner?.kind === "standalone_workbench" ? {} : { canvasId: PROJECT_ID }),
    ...ownerScope(owner),
    model: { providerId: PROVIDER_ID, model: "video_generation-model" },
    references: { bindings: [], assets: [] },
    operation: { task: "video_generation", capability: "t2v" },
    prompt: "A real video prompt",
    seconds: 6,
    width: 1920,
    height: 1080,
    taskCount,
  });
}

function audioPlan(owner?: CreativeTaskOwner) {
  return prepareAudioWorkbenchRun({
    catalog: catalog("speech_synthesis"),
    ...(owner?.kind === "standalone_workbench" ? {} : { canvasId: PROJECT_ID }),
    ...ownerScope(owner),
    model: {
      providerId: PROVIDER_ID,
      model: "speech_synthesis-model",
    },
    references: { bindings: [], assets: [] },
    value: {
      text: "A real narration",
      instructions: "",
      voice: "alloy",
      format: "mp3",
      speed: 1,
      model: {
        providerId: PROVIDER_ID,
        model: "speech_synthesis-model",
      },
    },
    fieldSupport: {
      voice: true,
      format: true,
      speed: false,
      instructions: false,
      references: false,
    },
  });
}

function captureError(action: () => unknown): Error | null {
  try {
    action();
    return null;
  } catch (error) {
    return error instanceof Error ? error : new Error(String(error));
  }
}

describe("Creative workbench controller", () => {
  test("never emits idle between a create response and its authoritative task state", async () => {
    const lifecycle: string[] = [];
    const persistedReferences: CreativeTaskReference[] = [];
    const port: CreativeTaskPort = {
      create: async (input) => {
        lifecycle.push("create");
        return task(input, "queued");
      },
      get: async (reference) =>
        task(
          {
            ...reference,
            idempotencyKey: reference.taskId,
            parameters: { prompt: "A real prompt" },
            inputs: [],
          },
          "succeeded",
        ),
      cancel: async () => {
        throw new Error("not used");
      },
    };
    const controller = new CreativeWorkbenchRuntimeController(port, assets, {
      poll: { intervalMs: 0 },
      onPendingTask: (reference) => {
        persistedReferences.push(reference);
        lifecycle.push("pending");
      },
      onSettledTask: () => {
        lifecycle.push("settled");
      },
    });
    const states: string[] = [];
    controller.subscribe((snapshot) => states.push(snapshot.state));

    await controller.run(imagePlan());

    const submitting = states.indexOf("submitting");
    expect(submitting).toBeGreaterThanOrEqual(0);
    expect(states.slice(submitting + 1).includes("idle")).toBe(false);
    expect(controller.snapshot().state).toBe("succeeded");
    expect(lifecycle).toEqual(["pending", "create", "settled"]);
    expect(persistedReferences[0]?.taskId).toBe(
      controller.snapshot().entries[0]?.task.taskId,
    );
    const exposed = controller.snapshot();
    exposed.entries[0]?.task.resultAssetIds.push(ASSET_B);
    expect(
      controller.snapshot().entries[0]?.task.resultAssetIds.includes(ASSET_B),
    ).toBe(false);
  });

  test("surfaces a current AbortError and resumes polling the same exact task", async () => {
    let gets = 0;
    let created: CreativeTask | null = null;
    const port: CreativeTaskPort = {
      create: async (input) => {
        created = task(input, "running");
        return created;
      },
      get: async (reference) => {
        gets += 1;
        if (gets === 1) {
          const error = new Error("transport aborted without controller abort");
          error.name = "AbortError";
          throw error;
        }
        if (!created) throw new Error("missing task");
        return {
          ...created,
          ...reference,
          status: "succeeded",
          resultAssetIds: [ASSET_A],
          finishedAt: 2_000,
        };
      },
      cancel: async () => {
        throw new Error("not used");
      },
    };
    const controller = new CreativeWorkbenchRuntimeController(port, assets, {
      poll: { intervalMs: 0 },
    });

    const failedPoll = await controller.run(imagePlan());
    expect(failedPoll.state).toBe("request_error");
    expect(
      failedPoll.entries[0]?.requestError?.message.includes(
        "transport aborted",
      ),
    ).toBe(true);
    const taskId = failedPoll.entries[0]?.task.taskId;
    if (!taskId) throw new Error("expected real task id");

    const recovered = await controller.retry(taskId);

    expect(recovered.state).toBe("succeeded");
    expect(recovered.entries).toHaveLength(1);
    expect(recovered.entries[0]?.task.taskId).toBe(taskId);
    expect(gets).toBe(2);
  });

  test("retries an initial owner-history recovery after a transient GET failure", async () => {
    const plan = imagePlan();
    const reference: CreativeTaskReference = {
      taskId: plan.input.idempotencyKey,
      owner: { ...plan.input.owner },
      providerId: plan.input.providerId,
      model: plan.input.model,
      task: plan.input.task,
      capability: plan.input.capability,
    };
    let gets = 0;
    const port: CreativeTaskPort = {
      create: async () => {
        throw new Error("not used");
      },
      get: async () => {
        gets += 1;
        if (gets === 1) throw new Error("temporary recovery transport failure");
        return task(plan.input, "succeeded", {
          taskId: reference.taskId,
          assetId: ASSET_A,
        });
      },
      cancel: async () => {
        throw new Error("not used");
      },
    };
    const controller = new CreativeWorkbenchRuntimeController(port, assets);
    const request = { reference, outputKind: "image" as const, retryInput: null };

    const failed = await controller.resume([request]);
    expect(failed.state).toBe("request_error");
    expect(failed.entries).toHaveLength(0);

    const recovered = await controller.resume([request]);
    expect(recovered.state).toBe("succeeded");
    expect(recovered.entries[0]?.task.taskId).toBe(reference.taskId);
    expect(gets).toBe(2);
  });

  test("retries one failed video task without clearing successful siblings", async () => {
    const inputs: CreateCreativeTaskInput[] = [];
    const port: CreativeTaskPort = {
      create: async (input) => {
        inputs.push(structuredClone(input));
        if (inputs.length === 1)
          return task(input, "succeeded", { assetId: ASSET_A });
        if (inputs.length === 2) return task(input, "failed");
        return task(input, "succeeded", { assetId: ASSET_B });
      },
      get: async () => {
        throw new Error("not used");
      },
      cancel: async () => {
        throw new Error("not used");
      },
    };
    const controller = new CreativeWorkbenchRuntimeController(port, assets);

    const first = await controller.run(videoPlan());
    const failed = first.entries.find(
      (entry) => entry.task.status === "failed",
    );
    expect(
      first.entries.some((entry) =>
        entry.outputs.some((output) => output.assetId === ASSET_A),
      ),
    ).toBe(true);
    if (!failed) throw new Error("expected failed task");

    const retried = await controller.retry(failed.task.taskId);

    expect(retried.entries).toHaveLength(3);
    expect(
      retried.entries.some((entry) =>
        entry.outputs.some((output) => output.assetId === ASSET_A),
      ),
    ).toBe(true);
    expect(
      retried.entries.some((entry) =>
        entry.outputs.some((output) => output.assetId === ASSET_B),
      ),
    ).toBe(true);
    expect(
      new Set(inputs.slice(0, 2).map((input) => input.idempotencyKey)).size,
    ).toBe(2);
    expect(inputs[2]?.idempotencyKey).not.toBe(inputs[1]?.idempotencyKey);
  });

  test("dismisses only terminal presentation entries", async () => {
    const terminalPort: CreativeTaskPort = {
      create: async (input) => task(input, "succeeded", { assetId: ASSET_A }),
      get: async () => {
        throw new Error("not used");
      },
      cancel: async () => {
        throw new Error("not used");
      },
    };
    const terminal = new CreativeWorkbenchRuntimeController(terminalPort, assets);
    const completed = await terminal.run(videoPlan(2));
    const firstId = completed.entries[0]?.task.taskId;
    if (!firstId) throw new Error("expected terminal task");
    expect(terminal.dismiss([firstId]).entries).toHaveLength(1);

    const livePort: CreativeTaskPort = {
      create: async (input) => task(input, "queued"),
      get: async () => {
        throw new Error("temporary transport failure");
      },
      cancel: async () => {
        throw new Error("not used");
      },
    };
    const live = new CreativeWorkbenchRuntimeController(livePort, assets);
    const pending = await live.run(imagePlan());
    const liveId = pending.entries[0]?.task.taskId;
    if (!liveId) throw new Error("expected live task");
    const error = captureError(() => live.dismiss([liveId]));
    expect(error?.message.includes("Cannot dismiss live task")).toBe(true);
  });

  test("retains a pre-task submission failure and retries the same idempotency key", async () => {
    const keys: string[] = [];
    let attempts = 0;
    const port: CreativeTaskPort = {
      create: async (input) => {
        attempts += 1;
        keys.push(input.idempotencyKey);
        if (attempts === 1) throw new Error("response lost");
        return task(input, "succeeded");
      },
      get: async () => {
        throw new Error("not used");
      },
      cancel: async () => {
        throw new Error("not used");
      },
    };
    const controller = new CreativeWorkbenchRuntimeController(port, assets);

    const failed = await controller.run(imagePlan());
    expect(failed.state).toBe("request_error");
    expect(failed.entries).toHaveLength(0);
    expect(failed.submissionFailures).toHaveLength(1);
    expect(workbenchRuntimeRequestPresentation(failed)).toEqual({
      state: "request_error",
      message: "response lost",
      retryableSubmissionOrders: [0],
    });
    let duplicateRunError: unknown;
    try {
      await controller.run(imagePlan());
    } catch (error) {
      duplicateRunError = error;
    }
    expect(duplicateRunError instanceof Error).toBe(true);
    expect(attempts).toBe(1);

    const retried = await controller.retrySubmission(0);

    expect(retried.state).toBe("succeeded");
    expect(retried.submissionFailures).toHaveLength(0);
    expect(keys).toHaveLength(2);
    expect(keys[1]).toBe(keys[0]);
  });

  test("cancels a recovering task before its first GET and fences a stale running response", async () => {
    const plan = imagePlan();
    const reference: CreativeTaskReference = {
      taskId: plan.input.idempotencyKey,
      owner: { ...plan.input.owner },
      providerId: plan.input.providerId,
      model: plan.input.model,
      task: plan.input.task,
      capability: plan.input.capability,
    };
    let resolveFirstGet: ((value: CreativeTask) => void) | undefined;
    let gets = 0;
    const running = task(plan.input, "running");
    const canceled = {
      ...running,
      status: "canceled" as const,
      finishedAt: 2_000,
    };
    const port: CreativeTaskPort = {
      create: async () => {
        throw new Error("not used");
      },
      get: async () => {
        gets += 1;
        if (gets === 1) {
          return new Promise<CreativeTask>((resolve) => {
            resolveFirstGet = resolve;
          });
        }
        return canceled;
      },
      cancel: async () => canceled,
    };
    const controller = new CreativeWorkbenchRuntimeController(port, assets, {
      poll: { intervalMs: 0 },
    });

    const recovery = controller.resume([{ reference, outputKind: "image" }]);
    await Promise.resolve();
    const afterCancel = await controller.cancel(reference.taskId);
    expect(afterCancel.entries[0]?.task.status).toBe("canceled");

    resolveFirstGet?.(running);
    const final = await recovery;
    expect(final.entries[0]?.task.status).toBe("canceled");
  });

  test("lets the persistence owner remove one orphan without discarding another recovery", async () => {
    const missingPlan = imagePlan();
    const healthyPlan = imagePlan();
    const referenceFor = (
      input: CreateCreativeTaskInput,
    ): CreativeTaskReference => ({
      taskId: input.idempotencyKey,
      owner: { ...input.owner },
      providerId: input.providerId,
      model: input.model,
      task: input.task,
      capability: input.capability,
    });
    const missing = referenceFor(missingPlan.input);
    const healthy = referenceFor(healthyPlan.input);
    const removed: string[] = [];
    const port: CreativeTaskPort = {
      create: async () => {
        throw new Error("not used");
      },
      get: async (reference) => {
        if (reference.taskId === missing.taskId) throw new Error("404 orphan");
        return task(healthyPlan.input, "succeeded");
      },
      cancel: async () => {
        throw new Error("not used");
      },
    };
    const controller = new CreativeWorkbenchRuntimeController(port, assets, {
      onRecoveryFailure: (reference, error) => {
        if (!(error instanceof Error) || !error.message.includes("404"))
          return false;
        removed.push(reference.taskId);
        return true;
      },
    });

    const recovered = await controller.resume([
      { reference: missing, outputKind: "image" },
      { reference: healthy, outputKind: "image" },
    ]);

    expect(removed).toEqual([missing.taskId]);
    expect(recovered.requestError).toBeNull();
    expect(recovered.entries).toHaveLength(1);
    expect(recovered.entries[0]?.task.taskId).toBe(healthy.taskId);
    expect(recovered.entries[0]?.task.status).toBe("succeeded");
  });
});

describe("Creative workbench planning and presentation boundaries", () => {
  test("does not fall back from image_edit to image_generation", () => {
    const error = captureError(() =>
      resolveExactWorkbenchModel(
        catalog("image_generation"),
        { providerId: PROVIDER_ID, model: "image_generation-model" },
        "image_edit",
      ),
    );
    expect(error?.message.includes("not enabled for image_edit")).toBe(true);
  });

  test("projects only committed image asset ids and real asset URLs", () => {
    const plan = imagePlan();
    const succeeded = task(plan.input, "succeeded");
    const snapshot: CreativeWorkbenchRuntimeSnapshot = {
      state: "succeeded",
      entries: [
        {
          order: 0,
          task: succeeded,
          outputs: [
            {
              assetId: ASSET_A,
              kind: "image",
              url: `nomifun://asset/${ASSET_A}`,
            },
          ],
          requestError: null,
          retryInput: plan.input,
          outputKind: "image",
        },
      ],
      submissionFailures: [],
      submittingCount: 0,
      recoveringCount: 0,
      requestError: null,
    };

    expect(mapImageWorkbenchRuntimeResults(snapshot)[0]).toMatchObject({
      id: succeeded.taskId,
      taskId: succeeded.taskId,
      status: "succeeded",
      outputs: [
        {
          assetId: ASSET_A,
          imageUrl: `nomifun://asset/${ASSET_A}`,
        },
      ],
    });
  });

  test("does not offer a retry for deterministic image parameter failures", () => {
    const plan = imagePlan();
    const failed = task(plan.input, "failed", {
      errorKind: "invalid_params",
      error: "StepFun model step-image-edit-2 does not support size 1536x1024",
    });
    const snapshot: CreativeWorkbenchRuntimeSnapshot = {
      state: "failed",
      entries: [
        {
          order: 0,
          task: failed,
          outputs: [],
          requestError: null,
          retryInput: plan.input,
          outputKind: "image",
        },
      ],
      submissionFailures: [],
      submittingCount: 0,
      recoveringCount: 0,
      requestError: null,
    };

    expect(mapImageWorkbenchRuntimeResults(snapshot)[0]?.retryable).toBe(false);
  });

  test("rejects non-image objects instead of inventing image reference previews", () => {
    const audio: CreativeAsset = {
      id: ASSET_A,
      kind: "audio",
      title: "Voice reference",
      collection: null,
      tags: [],
      mimeType: "audio/mpeg",
      width: null,
      height: null,
      bytes: 123,
      inLibrary: true,
      textContent: null,
      origin: null,
      originalUrl: `nomifun://asset/${ASSET_A}`,
      thumbnailUrl: null,
      createdAt: 1,
      updatedAt: 1,
    };
    const error = captureError(() =>
      imageWorkbenchReferencesFromAssets([audio]),
    );
    expect(error?.message.includes("is audio")).toBe(true);
  });

  test("builds an exact standalone owner without inventing a config node", () => {
    const base = {
      catalog: catalog("image_generation"),
      model: {
        providerId: PROVIDER_ID,
        model: "image_generation-model",
      },
      references: { bindings: [], assets: [] },
      operation: { task: "image_generation" as const, capability: "t2i" as const },
      prompt: "A standalone prompt",
      interfaceMode: "images" as const,
      quality: "high" as const,
      width: 1024,
      height: 1024,
      aspectRatio: "1:1",
      count: 1,
    };
    const plan = prepareImageWorkbenchRun({
      ...base,
      owner: {
        kind: "standalone_workbench",
        workbenchKind: "image",
      },
    });
    expect(plan.input.owner).toEqual({
      kind: "standalone_workbench",
      workbenchKind: "image",
    });
    const ambiguous = captureError(() =>
      prepareImageWorkbenchRun({
        ...base,
        canvasId: PROJECT_ID,
        nodeId: NODE_ID,
        owner: {
          kind: "standalone_workbench",
          workbenchKind: "image",
        },
      })
    );
    expect(ambiguous?.message.includes("either owner or nodeId")).toBe(true);

    for (const [expectedKind, action] of [
      ["image", () => imagePlan(standaloneOwner("video"))],
      ["video", () => videoPlan(1, standaloneOwner("audio"))],
      ["audio", () => audioPlan(standaloneOwner("image"))],
    ] as const) {
      const mismatch = captureError(action);
      expect(mismatch?.message.includes(`exact ${expectedKind} standalone owner`)).toBe(
        true,
      );
    }

    const failed = task(plan.input, "failed");
    const retry = prepareStandaloneHistoryRetry({
      catalog: base.catalog,
      task: failed,
      references: base.references,
    });
    expect(retry.input.owner).toEqual(plan.input.owner);
    expect(retry.input.parameters).toEqual(plan.input.parameters);
    expect(retry.input.inputs).toEqual(plan.input.inputs);
    expect(retry.input.idempotencyKey).not.toBe(plan.input.idempotencyKey);
  });

  test("projects StepFun dimensions into the provider-native size contract", () => {
    const base = {
      catalog: stepfunImageCatalog(),
      owner: standaloneOwner("image"),
      model: {
        providerId: PROVIDER_ID,
        model: "step-image-edit-2",
      },
      references: { bindings: [], assets: [] },
      operation: { task: "image_generation" as const, capability: "t2i" as const },
      prompt: "A wide landscape",
      interfaceMode: "images" as const,
      quality: "auto" as const,
      width: 1360,
      height: 768,
      aspectRatio: "16:9",
      count: 1,
    };
    const plan = prepareImageWorkbenchRun(base);
    expect(plan.input.parameters).toMatchObject({
      width: 1360,
      height: 768,
      size: "768x1360",
    });

    const unsupported = captureError(() =>
      prepareImageWorkbenchRun({
        ...base,
        width: 1536,
        height: 1024,
        aspectRatio: "3:2",
      })
    );
    expect(unsupported?.message.includes("does not support the requested dimensions")).toBe(true);
  });

  test("keeps audio cancellation enabled for an authoritative queued task", async () => {
    const plan = audioPlan();
    const queued = task(plan.input, "queued");
    const snapshot: CreativeWorkbenchRuntimeSnapshot = {
      state: "queued",
      entries: [
        {
          order: 0,
          task: queued,
          outputs: [],
          requestError: null,
          retryInput: plan.input,
          outputKind: "audio",
        },
      ],
      submissionFailures: [],
      submittingCount: 0,
      recoveringCount: 0,
      requestError: null,
    };
    let canceled = 0;
    const errors: unknown[] = [];
    const props = createAudioWorkbenchRuntimeProps({
      base: {
        value: {
          text: "A real narration",
          instructions: "",
          voice: "alloy",
          format: "mp3",
          speed: 1,
          model: {
            providerId: PROVIDER_ID,
            model: "speech_synthesis-model",
          },
        },
        modelSlot: null,
        voiceOptions: [],
        formatOptions: [],
        references: [],
        onValueChange: () => undefined,
        onRemoveReference: () => undefined,
        onPlaybackChange: () => undefined,
        onDownloadResult: () => undefined,
        onInsertResult: () => undefined,
      },
      runtime: snapshot,
      onGenerate: () => undefined,
      onCancel: () => {
        canceled += 1;
      },
      onActionError: (error) => errors.push(error),
    });

    expect(props.disabled).toBe(false);
    props.onCancel?.();
    await Promise.resolve();
    expect(canceled).toBe(1);
    expect(errors).toHaveLength(0);
  });
});
