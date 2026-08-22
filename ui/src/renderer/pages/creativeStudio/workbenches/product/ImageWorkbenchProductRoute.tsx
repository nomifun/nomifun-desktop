/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isBackendHttpError } from '@/common/adapter/httpBridge';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  CreativeAssetPickerModal,
  creativeAssetClient,
  useCreativeAssets,
  type CreativeAsset,
} from '../../assets';
import type { CreativeProjectDetail } from '../../domain';
import { useNomiCreativeModelCatalog } from '../../models';
import {
  creativeTaskClient,
  creativeTaskReference,
  type CreativeTask,
} from '../../tasks';
import { creativeTaskHistoryClient } from '../../tasks/historyClient';
import {
  ImageWorkbench,
  type ImageWorkbenchAspectRatioOption,
  type ImageWorkbenchLayout,
  type ImageWorkbenchModelIdentity,
  type ImageWorkbenchSettings,
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
  imageWorkbenchModelOptions,
  imageWorkbenchReferencesFromAssets,
  prepareStandaloneHistoryRetry,
  useImageWorkbenchRuntime,
} from '../runtime';
import { standaloneWorkbenchOwner } from './ownership';
import {
  StandaloneWorkbenchPage,
  StandaloneHistoryGate,
  StandaloneHistoryRetireDialog,
  useStandaloneWorkbenchScope,
} from './shared';
import styles from './StandaloneWorkbenchProduct.module.css';

const EMPTY_SETTINGS: ImageWorkbenchSettings = {
  model: null,
  interfaceMode: 'images',
  quality: 'auto',
  width: 1024,
  height: 1024,
  aspectRatio: '1:1',
  count: 1,
};

const ownerScopedPendingNoop = async (): Promise<void> => undefined;

const UnownedImageWorkbench: React.FC<{
  historyLoading?: boolean;
  historyError?: string;
}> = ({
  historyLoading,
  historyError,
}) => {
  const catalog = useNomiCreativeModelCatalog();
  const [prompt, setPrompt] = useState('');
  const [layout, setLayout] = useState<ImageWorkbenchLayout>('side');
  const [settings, setSettings] = useState(EMPTY_SETTINGS);
  return (
    <ImageWorkbench
      layout={layout}
      prompt={prompt}
      references={[]}
      settings={settings}
      modelOptions={imageWorkbenchModelOptions(catalog, 'image_generation')}
      results={[]}
      selectedResultIds={[]}
      task={{ state: 'idle', pendingCount: 0 }}
      disabled
      historyLoading={historyLoading}
      historyError={historyError}
      onLayoutChange={setLayout}
      onPromptChange={setPrompt}
      onRemoveReference={() => undefined}
      onModelChange={(model) => setSettings((value) => ({ ...value, model }))}
      onInterfaceModeChange={(interfaceMode) =>
        setSettings((value) => ({ ...value, interfaceMode }))
      }
      onQualityChange={(quality) => setSettings((value) => ({ ...value, quality }))}
      onDimensionsChange={(dimensions) =>
        setSettings((value) => ({ ...value, ...dimensions }))
      }
      onAspectRatioChange={(option) =>
        setSettings((value) => ({
          ...value,
          aspectRatio: option.value,
          width: option.width,
          height: option.height,
        }))
      }
      onCountChange={(count) => setSettings((value) => ({ ...value, count }))}
      onGenerate={() => undefined}
      onResultSelectionChange={() => undefined}
    />
  );
};

const imageSettingsFromTask = (task: CreativeTask): ImageWorkbenchSettings => {
  if (task.task !== 'image_generation' && task.task !== 'image_edit') {
    throw new Error(`任务 ${task.taskId} 不是图片工作台任务。`);
  }
  const interfaceMode = task.parameters.interface_mode;
  const quality = task.parameters.quality;
  const aspectRatio = task.parameters.aspect;
  const count = task.parameters.count;
  const width = task.parameters.width;
  const height = task.parameters.height;
  if (
    (interfaceMode !== 'images' && interfaceMode !== 'responses') ||
    (quality !== 'auto' && quality !== 'high' && quality !== 'medium' && quality !== 'low') ||
    typeof aspectRatio !== 'string' ||
    !Number.isSafeInteger(count) ||
    (width !== undefined && !Number.isSafeInteger(width)) ||
    (height !== undefined && !Number.isSafeInteger(height)) ||
    (width === undefined) !== (height === undefined)
  ) {
    throw new Error(`任务 ${task.taskId} 的图片参数快照不完整，无法载入。`);
  }
  return {
    model: { providerId: task.providerId, model: task.model },
    interfaceMode,
    quality,
    aspectRatio,
    count: count as number,
    width: (width as number | undefined) ?? null,
    height: (height as number | undefined) ?? null,
  };
};

const OwnedImageWorkbenchReady: React.FC<{
  detail: CreativeProjectDetail;
  history: StandaloneWorkbenchHistoryState;
}> = ({ detail, history }) => {
  const navigate = useNavigate();
  const catalog = useNomiCreativeModelCatalog();
  const assets = useCreativeAssets({ pageSize: 200, query: { sort: 'updated_desc' } });
  const [layout, setLayout] = useState<ImageWorkbenchLayout>('side');
  const [prompt, setPrompt] = useState('');
  const [settings, setSettings] = useState<ImageWorkbenchSettings>(EMPTY_SETTINGS);
  const [referenceIds, setReferenceIds] = useState<string[]>([]);
  const [hydratedReferences, setHydratedReferences] = useState<CreativeAsset[]>([]);
  const [selectedResultIds, setSelectedResultIds] = useState<string[]>([]);
  const [retireTaskIds, setRetireTaskIds] = useState<string[]>([]);
  const [retiredTaskIds, setRetiredTaskIds] = useState<string[]>([]);
  const [retiring, setRetiring] = useState(false);
  const [retireError, setRetireError] = useState<string | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const loadGenerationRef = useRef(0);
  const projectId = detail.project.projectId;
  const historyScope = useMemo(
    () => ({ projectId, workbenchKind: 'image' as const }),
    [projectId]
  );
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
    scopeKey: `${projectId}:standalone-image`,
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

  useEffect(() => {
    if (!settings.model || catalog.status !== 'ready') return;
    const stillAvailable = exactWorkbenchModelOptions(catalog, modelTask).some(
      (option) =>
        option.providerId === settings.model?.providerId && option.model === settings.model.model
    );
    if (!stillAvailable) setSettings((value) => ({ ...value, model: null }));
  }, [catalog, modelTask, settings.model]);

  useEffect(
    () => () => {
      loadGenerationRef.current += 1;
    },
    []
  );

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
    if (!settings.model || catalog.status !== 'ready') {
      setError('没有可用且明确选择的真实模型，未发起生成。');
      return;
    }
    if (references.length !== referenceIds.length) {
      setError('存在无法读取的图片参考，未把 I2I 请求降级成 T2I。');
      return;
    }
    try {
      await runtime.generate({
        catalog,
        projectId,
        owner: standaloneWorkbenchOwner(projectId, 'image'),
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

  const loadTask = async (taskId: string): Promise<void> => {
    const generation = loadGenerationRef.current + 1;
    loadGenerationRef.current = generation;
    setError(null);
    try {
      const task = taskById(taskId);
      const nextReferences = await hydrateStandaloneTaskReferences(task, creativeAssetClient);
      const nextSettings = imageSettingsFromTask(task);
      if (loadGenerationRef.current !== generation) return;
      setPrompt(typeof task.parameters.prompt === 'string' ? task.parameters.prompt : '');
      setSettings(nextSettings);
      setHydratedReferences([...nextReferences.assets]);
      setReferenceIds(nextReferences.bindings.map((binding) => binding.assetId));
    } catch (reason) {
      if (loadGenerationRef.current !== generation) return;
      setError(reason instanceof Error ? reason.message : String(reason));
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
        projectId,
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
    onDimensionsChange: (dimensions: { width: number | null; height: number | null }) =>
      setSettings((value) => ({ ...value, ...dimensions })),
    onAspectRatioChange: (option: ImageWorkbenchAspectRatioOption) =>
      setSettings((value) => ({
        ...value,
        aspectRatio: option.value,
        width: option.width,
        height: option.height,
      })),
    onCountChange: (count: number) => setSettings((value) => ({ ...value, count })),
    onResultSelectionChange: setSelectedResultIds,
    onDeleteResult: (taskId: string) => requestRetirement([taskId]),
    onDeleteSelected: requestRetirement,
    onLoadResult: (taskId: string) => void loadTask(taskId),
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
    disabled: catalog.status !== 'ready' || history.refreshing,
    onGenerate: generate,
    onRetryTask: retryTask,
    onActionError: (reason) => setError(reason instanceof Error ? reason.message : String(reason)),
  });

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
      <ImageWorkbench {...props} />
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

const OwnedImageWorkbench: React.FC<{ detail: CreativeProjectDetail }> = ({ detail }) => {
  const historyScope = useMemo(
    () => ({ projectId: detail.project.projectId, workbenchKind: 'image' as const }),
    [detail.project.projectId]
  );
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
  return <OwnedImageWorkbenchReady detail={detail} history={history} />;
};

/** Router-ready, prop-free standalone image product. */
const ImageWorkbenchProductRoute: React.FC = () => {
  const scope = useStandaloneWorkbenchScope();
  return (
    <StandaloneWorkbenchPage scope={scope} error={null}>
      {scope.state === 'ready' && scope.detail ? (
        <OwnedImageWorkbench key={scope.detail.project.projectId} detail={scope.detail} />
      ) : (
        <UnownedImageWorkbench />
      )}
    </StandaloneWorkbenchPage>
  );
};

export default ImageWorkbenchProductRoute;
