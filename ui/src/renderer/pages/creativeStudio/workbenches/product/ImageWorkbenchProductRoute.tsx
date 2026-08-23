/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isBackendHttpError } from '@/common/adapter/httpBridge';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  CreativeAssetPickerModal,
  creativeAssetClient,
  useCreativeAssets,
  type CreativeAsset,
} from '../../assets';
import {
  CreativeModelSelect,
  useNomiCreativeModelCatalog,
  type CreativeModelSelectionRef,
} from '../../models';
import {
  creativeTaskClient,
  creativeTaskReference,
  type CreativeTask,
} from '../../tasks';
import { creativeTaskHistoryClient } from '../../tasks/historyClient';
import {
  createDefaultImageWorkbenchDraft,
  createImageWorkbenchDraft,
  hydrateStandaloneWorkbenchDraftReferences,
  imageWorkbenchSettingsFromDraft,
  isExactWorkbenchDraftModelAvailable,
  readStandaloneWorkbenchDraft,
  writeStandaloneWorkbenchDraft,
} from '../drafts';
import {
  ImageWorkbench,
  type ImageWorkbenchAspectRatioOption,
  type ImageWorkbenchLayout,
  type ImageWorkbenchModelIdentity,
  type ImageWorkbenchSettings,
  imageWorkbenchSizePolicyForModel,
} from '../image';
import {
  combineStandaloneHistoryTasks,
  hydrateStandaloneTaskReferences,
  standaloneHistoryResumeRequests,
  standaloneHistoryRuntimeSnapshot,
  useStandaloneWorkbenchHistory,
  type StandaloneWorkbenchHistoryState,
} from '../history';
import {
  createImageWorkbenchRuntimeProps,
  exactWorkbenchModelOptions,
  imageWorkbenchReferencesFromAssets,
  prepareStandaloneHistoryRetry,
  useImageWorkbenchRuntime,
} from '../runtime';
import { standaloneWorkbenchOwner } from './ownership';
import {
  StandaloneWorkbenchPage,
  StandaloneHistoryGate,
  StandaloneHistoryRetireDialog,
} from './shared';
import styles from './StandaloneWorkbenchProduct.module.css';

const ownerScopedPendingNoop = async (): Promise<void> => undefined;

const IMAGE_GENERATION_MODEL_COPY = {
  placeholder: '选择生图模型',
  loading: '正在加载生图模型…',
  noCompatibleModel: '没有已启用且具备“图像生成”能力的模型。',
  disabled: '生成任务进行中，暂不可切换模型。',
  error: '生图模型目录加载失败。',
  unavailable: '已选生图模型当前不可用，请重新选择。',
  configureModels: '配置生图模型',
};

const IMAGE_EDIT_MODEL_COPY = {
  placeholder: '选择图片编辑模型',
  loading: '正在加载图片编辑模型…',
  noCompatibleModel: '没有已启用且具备“图片编辑”能力的模型。',
  disabled: '生成任务进行中，暂不可切换模型。',
  error: '图片编辑模型目录加载失败。',
  unavailable: '已选图片编辑模型当前不可用，请重新选择。',
  configureModels: '配置图片编辑模型',
};

const creativeModelSelection = (
  model: ImageWorkbenchModelIdentity | null
): CreativeModelSelectionRef | null =>
  model
    ? {
        providerId: model.providerId as CreativeModelSelectionRef['providerId'],
        model: model.model,
      }
    : null;

const OwnedImageWorkbenchReady: React.FC<{
  history: StandaloneWorkbenchHistoryState;
}> = ({ history }) => {
  const navigate = useNavigate();
  const catalog = useNomiCreativeModelCatalog();
  const assets = useCreativeAssets({ pageSize: 200, query: { sort: 'updated_desc' } });
  const [initialDraft] = useState(
    () => readStandaloneWorkbenchDraft('image') ?? createDefaultImageWorkbenchDraft()
  );
  const [layout, setLayout] = useState<ImageWorkbenchLayout>(initialDraft.layout);
  const [prompt, setPrompt] = useState(initialDraft.prompt);
  const [settings, setSettings] = useState<ImageWorkbenchSettings>(() =>
    imageWorkbenchSettingsFromDraft(initialDraft)
  );
  const [referenceIds, setReferenceIds] = useState<string[]>([
    ...initialDraft.referenceAssetIds,
  ]);
  const [hydratedReferences, setHydratedReferences] = useState<CreativeAsset[]>([]);
  const [draftReferencesRestoring, setDraftReferencesRestoring] = useState(
    initialDraft.referenceAssetIds.length > 0
  );
  const [selectedResultIds, setSelectedResultIds] = useState<string[]>([]);
  const [retireTaskIds, setRetireTaskIds] = useState<string[]>([]);
  const [retiredTaskIds, setRetiredTaskIds] = useState<string[]>([]);
  const [retiring, setRetiring] = useState(false);
  const [retireError, setRetireError] = useState<string | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const historyScope = useMemo(() => ({ workbenchKind: 'image' as const }), []);
  const durableTasks = useMemo(
    () =>
      combineStandaloneHistoryTasks(history.tasks, history.activeTasks).filter(
        (task) => !retiredTaskIds.includes(task.taskId)
      ),
    [history.activeTasks, history.tasks, retiredTaskIds]
  );
  const initialResumeRequests = useMemo(
    () => standaloneHistoryResumeRequests(historyScope, history.activeTasks),
    [history.activeTasks, historyScope]
  );
  const onSettledTask = useCallback(async () => {
    await history.reload();
  }, [history.reload]);
  const onRecoveryFailure = useCallback(
    async (_reference: unknown, reason: unknown): Promise<boolean> => {
      if (!isBackendHttpError(reason) || reason.status !== 404) return false;
      await history.reload();
      return true;
    },
    [history.reload]
  );
  const runtime = useImageWorkbenchRuntime({
    scopeKey: 'standalone-image',
    tasks: creativeTaskClient,
    assets: creativeAssetClient,
    initialResumeRequests,
    onPendingTask: ownerScopedPendingNoop,
    onSettledTask,
    onRecoveryFailure,
    onRuntimeError: (reason) => setError(reason instanceof Error ? reason.message : String(reason)),
  });
  const presentationRuntime = useMemo(
    () => standaloneHistoryRuntimeSnapshot(historyScope, durableTasks, runtime, creativeAssetClient),
    [durableTasks, historyScope, runtime]
  );
  const referenceById = useMemo(
    () => new Map([...assets.assets, ...hydratedReferences].map((asset) => [asset.id, asset])),
    [assets.assets, hydratedReferences]
  );
  const references = useMemo(
    () => referenceIds.flatMap((id) => referenceById.get(id) ?? []),
    [referenceById, referenceIds]
  );
  const modelTask = referenceIds.length ? 'image_edit' : 'image_generation';
  const modelOptionsForTask = useMemo(
    () => exactWorkbenchModelOptions(catalog, modelTask),
    [catalog, modelTask]
  );
  const selectedModelOption = useMemo(
    () =>
      settings.model
        ? modelOptionsForTask.find(
            (option) =>
              option.providerId === settings.model?.providerId &&
              option.model === settings.model?.model
          ) ?? null
        : null,
    [modelOptionsForTask, settings.model]
  );
  const sizePolicy = useMemo(
    () => imageWorkbenchSizePolicyForModel(selectedModelOption),
    [selectedModelOption]
  );
  const selectedSizeOption = useMemo(
    () =>
      sizePolicy.options.find(
        (option) =>
          !option.disabled &&
          option.value === settings.aspectRatio &&
          option.width === settings.width &&
          option.height === settings.height
      ) ?? null,
    [settings.aspectRatio, settings.height, settings.width, sizePolicy.options]
  );

  useEffect(() => {
    const restoredReferenceIds = initialDraft.referenceAssetIds;
    if (restoredReferenceIds.length === 0) return;
    const restoredReferenceSet = new Set(restoredReferenceIds);
    let canceled = false;
    void hydrateStandaloneWorkbenchDraftReferences(
      'image',
      restoredReferenceIds,
      creativeAssetClient
    )
      .then((hydrated) => {
        if (canceled) return;
        const retained = new Set(hydrated.retainedReferenceAssetIds);
        setHydratedReferences((current) => {
          const next = new Map(current.map((asset) => [asset.id, asset]));
          for (const assetId of restoredReferenceSet) next.delete(assetId);
          for (const asset of hydrated.assets) next.set(asset.id, asset);
          return [...next.values()];
        });
        setReferenceIds((current) =>
          current.filter(
            (assetId) => !restoredReferenceSet.has(assetId) || retained.has(assetId)
          )
        );
        setDraftReferencesRestoring(false);
      })
      .catch(() => {
        if (canceled) return;
        setReferenceIds((current) =>
          current.filter((assetId) => !restoredReferenceSet.has(assetId))
        );
        setDraftReferencesRestoring(false);
      });
    return () => {
      canceled = true;
    };
  }, [initialDraft.referenceAssetIds]);

  useEffect(() => {
    writeStandaloneWorkbenchDraft(
      createImageWorkbenchDraft({
        layout,
        prompt,
        settings,
        referenceAssetIds: referenceIds,
      })
    );
  }, [layout, prompt, referenceIds, settings]);

  useEffect(() => {
    if (
      draftReferencesRestoring ||
      !settings.model ||
      catalog.status !== 'ready'
    ) {
      return;
    }
    const stillAvailable = isExactWorkbenchDraftModelAvailable(
      settings.model,
      modelOptionsForTask
    );
    if (!stillAvailable) setSettings((value) => ({ ...value, model: null }));
  }, [catalog.status, draftReferencesRestoring, modelOptionsForTask, settings.model]);

  useEffect(() => {
    if (!selectedModelOption) return;
    const currentOption = sizePolicy.options.find(
      (option) => option.value === settings.aspectRatio
    );
    const fallbackOption =
      currentOption && !currentOption.disabled
        ? currentOption
        : sizePolicy.options.find((option) => !option.disabled);
    if (!fallbackOption) return;
    const dimensionsMatch =
      settings.width === fallbackOption.width && settings.height === fallbackOption.height;
    const shouldNormalizeDimensions =
      !sizePolicy.allowCustomDimensions && !dimensionsMatch;
    const nextCount = Math.min(settings.count, sizePolicy.maxCount);
    if (!shouldNormalizeDimensions && nextCount === settings.count) return;
    setSettings((value) => ({
      ...value,
      aspectRatio: fallbackOption.value,
      width: shouldNormalizeDimensions ? fallbackOption.width : value.width,
      height: shouldNormalizeDimensions ? fallbackOption.height : value.height,
      count: nextCount,
    }));
  }, [
    selectedModelOption,
    settings.aspectRatio,
    settings.count,
    settings.height,
    settings.width,
    sizePolicy,
  ]);

  const taskById = useCallback(
    (taskId: string): CreativeTask => {
      const task = presentationRuntime.entries.find((entry) => entry.task.taskId === taskId)?.task;
      if (!task) throw new Error(`找不到任务 ${taskId}。`);
      return task;
    },
    [presentationRuntime.entries]
  );

  const generate = async (): Promise<void> => {
    setError(null);
    if (draftReferencesRestoring) {
      setError('参考素材草稿仍在恢复，未发起生成。');
      return;
    }
    if (!settings.model || catalog.status !== 'ready') {
      setError('没有可用且明确选择的真实模型，未发起生成。');
      return;
    }
    if (references.length !== referenceIds.length) {
      setError('存在无法读取的图片参考，未把 I2I 请求降级成 T2I。');
      return;
    }
    if (settings.count > sizePolicy.maxCount) {
      setSettings((value) => ({ ...value, count: sizePolicy.maxCount }));
      setError(`当前模型最多支持生成 ${sizePolicy.maxCount} 张图片。`);
      return;
    }
    if (!sizePolicy.allowCustomDimensions && !selectedSizeOption) {
      setError('当前模型不支持该尺寸，请重新选择尺寸。');
      return;
    }
    try {
      await runtime.generate({
        catalog,
        owner: standaloneWorkbenchOwner('image'),
        model: settings.model,
        references: {
          assets: references,
          bindings: references.map((asset) => ({
            assetId: asset.id,
            kind: 'image' as const,
            role: 'reference' as const,
          })),
        },
        operation: references.length
          ? { task: 'image_edit', capability: 'i2i' }
          : { task: 'image_generation', capability: 't2i' },
        prompt,
        interfaceMode: settings.interfaceMode,
        quality: settings.quality,
        width: settings.width,
        height: settings.height,
        size: references.length ? null : selectedSizeOption?.requestSize ?? null,
        aspectRatio: settings.aspectRatio,
        count: settings.count,
      });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  const retryTask = async (taskId: string): Promise<void> => {
    setError(null);
    try {
      const task = taskById(taskId);
      const retryReferences = await hydrateStandaloneTaskReferences(task, creativeAssetClient);
      await runtime.run(
        prepareStandaloneHistoryRetry({ catalog, task, references: retryReferences })
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  const cancelTask = async (taskId: string): Promise<void> => {
    setError(null);
    try {
      await runtime.cancel(taskId);
    } catch {
      try {
        const task = taskById(taskId);
        await creativeTaskClient.cancel(creativeTaskReference(task));
        await history.reload();
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason));
      }
    }
  };

  const requestRetirement = (taskIds: readonly string[]): void => {
    setError(null);
    setRetireError(null);
    const unique = [...new Set(taskIds)];
    try {
      if (unique.length === 0 || unique.length > 100) {
        throw new Error('每次必须选择 1-100 条终态历史。');
      }
      for (const taskId of unique) {
        const task = taskById(taskId);
        if (task.status === 'queued' || task.status === 'running') {
          throw new Error('运行中的任务必须先取消，不能直接从历史移除。');
        }
      }
      setRetireTaskIds(unique);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  const confirmRetirement = async (): Promise<void> => {
    if (retireTaskIds.length === 0 || retiring) return;
    setRetiring(true);
    setError(null);
    setRetireError(null);
    try {
      const result = await creativeTaskHistoryClient.retireStandalone({
        workbenchKind: 'image',
        taskIds: retireTaskIds,
      });
      runtime.dismiss(result.retiredTaskIds);
      setRetiredTaskIds((current) => [...new Set([...current, ...result.retiredTaskIds])]);
      setSelectedResultIds([]);
      setRetireTaskIds([]);
      await history.reload();
    } catch (reason) {
      setRetireError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setRetiring(false);
    }
  };

  const baseProps = {
    layout,
    prompt,
    references: imageWorkbenchReferencesFromAssets(references),
    settings,
    aspectRatioOptions: sizePolicy.options,
    dimensionsDisabled: !sizePolicy.allowCustomDimensions,
    maxCount: sizePolicy.maxCount,
    selectedResultIds,
    onLayoutChange: setLayout,
    onPromptChange: setPrompt,
    onClearPrompt: () => setPrompt(''),
    onOpenPromptLibrary: () => navigate('/workshop/prompts'),
    onChooseReferences: () => setPickerOpen(true),
    onRemoveReference: (referenceId: string) =>
      setReferenceIds((ids) => ids.filter((id) => id !== referenceId)),
    onModelChange: (model: ImageWorkbenchModelIdentity | null) =>
      setSettings((value) => ({ ...value, model })),
    onInterfaceModeChange: (interfaceMode: ImageWorkbenchSettings['interfaceMode']) =>
      setSettings((value) => ({ ...value, interfaceMode })),
    onQualityChange: (quality: ImageWorkbenchSettings['quality']) =>
      setSettings((value) => ({ ...value, quality })),
    onDimensionsChange: (dimensions: { width: number | null; height: number | null }) => {
      if (!sizePolicy.allowCustomDimensions) return;
      setSettings((value) => ({ ...value, ...dimensions }));
    },
    onAspectRatioChange: (option: ImageWorkbenchAspectRatioOption) => {
      if (!sizePolicy.options.some((candidate) => candidate.value === option.value)) return;
      setSettings((value) => ({
        ...value,
        aspectRatio: option.value,
        width: option.width,
        height: option.height,
      }));
    },
    onCountChange: (count: number) =>
      setSettings((value) => ({
        ...value,
        count: Math.max(1, Math.min(sizePolicy.maxCount, Math.floor(count))),
      })),
    onResultSelectionChange: setSelectedResultIds,
    onDeleteResult: (taskId: string) => requestRetirement([taskId]),
    onDeleteSelected: requestRetirement,
    onCancelTask: (taskId: string) => void cancelTask(taskId),
    historyLoadingMore: history.loadingMore,
    historyHasMore: history.nextCursor !== null,
    onLoadMoreResults: () => void history.loadMore(),
  };
  const props = createImageWorkbenchRuntimeProps({
    base: baseProps,
    runtime: presentationRuntime,
    catalog,
    task: modelTask,
    disabled:
      draftReferencesRestoring || catalog.status !== 'ready' || history.refreshing,
    onGenerate: generate,
    onRetryTask: retryTask,
    onActionError: (reason) => setError(reason instanceof Error ? reason.message : String(reason)),
  });
  const modelSlot = (
    <CreativeModelSelect
      catalog={catalog}
      filter={{ capability: 'task', task: modelTask }}
      value={creativeModelSelection(settings.model)}
      onChange={(selection) =>
        setSettings((value) => ({
          ...value,
          model: { providerId: selection.providerId, model: selection.model },
        }))
      }
      disabled={props.disabled}
      label='模型'
      copy={modelTask === 'image_edit' ? IMAGE_EDIT_MODEL_COPY : IMAGE_GENERATION_MODEL_COPY}
      onOpenModelSettings={() =>
        void navigate(`/models?section=${modelTask === 'image_edit' ? 'image-edit' : 'image'}`)
      }
    />
  );

  return (
    <>
      {history.error || error ? (
        <div className={styles.runtimeNotice} role='alert'>
          {history.error?.message ?? error}
        </div>
      ) : null}
      {presentationRuntime.submissionFailures.map((failure) => (
        <div className={styles.runtimeNotice} role='status' key={failure.order}>
          <span>任务提交结果尚未确认，可使用原幂等请求安全重试。</span>
          <button
            type='button'
            onClick={() => {
              void runtime.retrySubmission(failure.order).catch((reason) =>
                setError(reason instanceof Error ? reason.message : String(reason))
              );
            }}
          >
            确认任务状态
          </button>
        </div>
      ))}
      {presentationRuntime.requestError && history.activeTasks.length > 0 ? (
        <div className={styles.runtimeNotice} role='status'>
          <span>{presentationRuntime.requestError.message}</span>
          <button
            type='button'
            onClick={() => {
              void runtime.resume(initialResumeRequests).catch((reason) =>
                setError(reason instanceof Error ? reason.message : String(reason))
              );
            }}
          >
            重试任务同步
          </button>
        </div>
      ) : null}
      <ImageWorkbench {...props} modelSlot={modelSlot} />
      <CreativeAssetPickerModal
        open={pickerOpen}
        assets={assets.assets}
        acceptedKinds={['image']}
        selectedIds={referenceIds}
        loading={assets.loading}
        loadingMore={assets.loadingMore}
        hasMore={assets.hasMore}
        error={assets.error ?? assets.mutationError}
        uploading={assets.mutating}
        onToggle={(asset: CreativeAsset) =>
          setReferenceIds((ids) =>
            ids.includes(asset.id) ? ids.filter((id) => id !== asset.id) : [...ids, asset.id]
          )
        }
        onLoadMore={() => void assets.loadMore()}
        onRetry={() => void assets.reload()}
        onUploadFiles={(files) => {
          void Promise.all(
            files.map((file) =>
              assets.upload(file, {
                title: file.name,
                tags: ['workbench-reference'],
                inLibrary: true,
              })
            )
          )
            .then((uploaded) =>
              setReferenceIds((ids) => [
                ...new Set([...ids, ...uploaded.map((asset) => asset.id)]),
              ])
            )
            .catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
        }}
        onCancel={() => setPickerOpen(false)}
      />
      <StandaloneHistoryRetireDialog
        open={retireTaskIds.length > 0}
        count={retireTaskIds.length}
        busy={retiring}
        error={retireError}
        onCancel={() => {
          setRetireTaskIds([]);
          setRetireError(null);
        }}
        onConfirm={() => void confirmRetirement()}
      />
    </>
  );
};

const OwnedImageWorkbench: React.FC = () => {
  const historyScope = useMemo(() => ({ workbenchKind: 'image' as const }), []);
  const history = useStandaloneWorkbenchHistory(historyScope);
  if (history.status !== 'ready') {
    return (
      <StandaloneHistoryGate
        label='生图'
        error={history.error}
        onRetry={() => void history.reload()}
      />
    );
  }
  return <OwnedImageWorkbenchReady history={history} />;
};

/** Router-ready, prop-free standalone image product. */
const ImageWorkbenchProductRoute: React.FC = () => {
  return (
    <StandaloneWorkbenchPage error={null}>
      <OwnedImageWorkbench />
    </StandaloneWorkbenchPage>
  );
};

export default ImageWorkbenchProductRoute;
