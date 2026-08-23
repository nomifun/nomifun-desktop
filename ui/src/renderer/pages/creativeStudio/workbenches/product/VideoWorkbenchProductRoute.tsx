/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { TFunction } from 'i18next';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
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
  combineStandaloneHistoryTasks,
  hydrateStandaloneTaskReferences,
  standaloneHistoryResumeRequests,
  standaloneHistoryRuntimeSnapshot,
  useStandaloneWorkbenchHistory,
  type StandaloneWorkbenchHistoryState,
} from '../history';
import {
  createDefaultVideoWorkbenchDraft,
  createVideoWorkbenchDraft,
  hydrateStandaloneWorkbenchDraftReferences,
  isExactWorkbenchDraftModelAvailable,
  readStandaloneWorkbenchDraft,
  writeStandaloneWorkbenchDraft,
} from '../drafts';
import {
  createVideoWorkbenchRuntimeProps,
  exactWorkbenchModelOptions,
  prepareStandaloneHistoryRetry,
  videoWorkbenchReferencesFromAssets,
  useVideoWorkbenchRuntime,
} from '../runtime';
import { VideoWorkbench, type VideoWorkbenchLayout } from '../video';
import {
  standaloneWorkbenchOwner,
  STANDALONE_VIDEO_MAX_CONCURRENT_TASKS,
} from './ownership';
import {
  creativeWorkbenchErrorMessage,
  StandaloneWorkbenchPage,
  StandaloneHistoryGate,
  StandaloneHistoryRetireDialog,
} from './shared';
import styles from './StandaloneWorkbenchProduct.module.css';

const RESOLUTIONS = [
  { value: '720p', label: '720P' },
  { value: '1080p', label: '1080P' },
];
const ASPECTS = [
  { value: '16:9', label: '16:9' },
  { value: '9:16', label: '9:16' },
  { value: '1:1', label: '1:1' },
];
const ownerScopedPendingNoop = async (): Promise<void> => undefined;

const videoDimensions = (
  resolution: string,
  aspect: string,
  t: TFunction
): { width: number; height: number } => {
  const shortEdge = resolution === '720p' ? 720 : resolution === '1080p' ? 1080 : null;
  if (shortEdge === null) {
    throw new Error(
      t('creativeStudio.product.video.errors.unsupportedResolution', {
        resolution,
        defaultValue: 'Unsupported video resolution: {{resolution}}',
      })
    );
  }
  if (aspect === '16:9') return { width: Math.round((shortEdge * 16) / 9), height: shortEdge };
  if (aspect === '9:16') return { width: shortEdge, height: Math.round((shortEdge * 16) / 9) };
  if (aspect === '1:1') return { width: shortEdge, height: shortEdge };
  throw new Error(
    t('creativeStudio.product.video.errors.unsupportedAspect', {
      aspect,
      defaultValue: 'Unsupported video aspect ratio: {{aspect}}',
    })
  );
};

const videoControlsFromTask = (
  task: CreativeTask,
  t: TFunction
): { prompt: string; resolution: string; aspect: string; duration: string } => {
  if (task.task !== 'video_generation') {
    throw new Error(
      t('creativeStudio.product.video.errors.notVideoTask', {
        taskId: task.taskId,
        defaultValue: 'Task {{taskId}} is not a video-generation task.',
      })
    );
  }
  const prompt = task.parameters.prompt;
  const width = task.parameters.width;
  const height = task.parameters.height;
  const seconds = task.parameters.seconds;
  if (
    typeof prompt !== 'string' ||
    !Number.isSafeInteger(width) ||
    !Number.isSafeInteger(height) ||
    (seconds !== 5 && seconds !== 10)
  ) {
    throw new Error(
      t('creativeStudio.product.video.errors.incompleteSnapshot', {
        taskId: task.taskId,
        defaultValue: 'Task {{taskId}} has an incomplete video parameter snapshot.',
      })
    );
  }
  const match = RESOLUTIONS.flatMap((resolution) =>
    ASPECTS.map((aspect) => ({
      resolution: resolution.value,
      aspect: aspect.value,
      dimensions: videoDimensions(resolution.value, aspect.value, t),
    }))
  ).find((candidate) => candidate.dimensions.width === width && candidate.dimensions.height === height);
  if (!match) {
    throw new Error(
      t('creativeStudio.product.video.errors.unsupportedSnapshotSize', {
        taskId: task.taskId,
        defaultValue: 'Task {{taskId}} uses a video size that is not available in this workbench.',
      })
    );
  }
  return { prompt, resolution: match.resolution, aspect: match.aspect, duration: String(seconds) };
};

const OwnedVideoWorkbenchReady: React.FC<{
  history: StandaloneWorkbenchHistoryState;
}> = ({ history }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const catalog = useNomiCreativeModelCatalog();
  const assets = useCreativeAssets({ pageSize: 200, query: { sort: 'updated_desc' } });
  const [initialDraft] = useState(
    () => readStandaloneWorkbenchDraft('video') ?? createDefaultVideoWorkbenchDraft()
  );
  const [layout, setLayout] = useState<VideoWorkbenchLayout>(initialDraft.layout);
  const [prompt, setPrompt] = useState(initialDraft.prompt);
  const [model, setModel] = useState<CreativeModelSelectionRef | null>(
    initialDraft.model ? { ...initialDraft.model } : null
  );
  const [resolution, setResolution] = useState<string>(initialDraft.parameters.resolution);
  const [aspect, setAspect] = useState<string>(initialDraft.parameters.aspect);
  const [duration, setDuration] = useState<string>(initialDraft.parameters.duration);
  const [taskCount, setTaskCount] = useState<number>(initialDraft.parameters.taskCount);
  const [referenceIds, setReferenceIds] = useState<string[]>([
    ...initialDraft.referenceAssetIds,
  ]);
  const [hydratedReferences, setHydratedReferences] = useState<CreativeAsset[]>([]);
  const [draftReferencesRestoring, setDraftReferencesRestoring] = useState(
    initialDraft.referenceAssetIds.length > 0
  );
  const [selectedTaskIds, setSelectedTaskIds] = useState<string[]>([]);
  const [retireTaskIds, setRetireTaskIds] = useState<string[]>([]);
  const [retiredTaskIds, setRetiredTaskIds] = useState<string[]>([]);
  const [retiring, setRetiring] = useState(false);
  const [retireError, setRetireError] = useState<string | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const formatError = useCallback(
    (reason: unknown) => creativeWorkbenchErrorMessage(reason, t),
    [t]
  );
  const durationOptions = useMemo(
    () => [
      {
        value: '5',
        label: t('creativeStudio.product.video.durationFive', { defaultValue: '5 秒' }),
      },
      {
        value: '10',
        label: t('creativeStudio.product.video.durationTen', { defaultValue: '10 秒' }),
      },
    ],
    [t]
  );
  const loadGenerationRef = useRef(0);
  const historyScope = useMemo(() => ({ workbenchKind: 'video' as const }), []);
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
  const runtime = useVideoWorkbenchRuntime({
    scopeKey: 'standalone-video',
    tasks: creativeTaskClient,
    assets: creativeAssetClient,
    initialResumeRequests,
    onPendingTask: ownerScopedPendingNoop,
    onSettledTask,
    onRecoveryFailure,
    onRuntimeError: (reason) => setError(formatError(reason)),
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
  const busy = presentationRuntime.entries.some(
    (entry) => entry.task.status === 'queued' || entry.task.status === 'running'
  ) || presentationRuntime.submittingCount > 0 || presentationRuntime.recoveringCount > 0;

  useEffect(() => {
    const restoredReferenceIds = initialDraft.referenceAssetIds;
    if (restoredReferenceIds.length === 0) return;
    const restoredReferenceSet = new Set(restoredReferenceIds);
    let canceled = false;
    void hydrateStandaloneWorkbenchDraftReferences(
      'video',
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
      createVideoWorkbenchDraft({
        layout,
        prompt,
        model,
        resolution,
        aspect,
        duration,
        taskCount,
        referenceAssetIds: referenceIds,
      })
    );
  }, [aspect, duration, layout, model, prompt, referenceIds, resolution, taskCount]);

  useEffect(() => {
    if (!model || catalog.status !== 'ready') return;
    const stillAvailable = isExactWorkbenchDraftModelAvailable(
      model,
      exactWorkbenchModelOptions(catalog, 'video_generation')
    );
    if (!stillAvailable) setModel(null);
  }, [catalog, model]);

  useEffect(
    () => () => {
      loadGenerationRef.current += 1;
    },
    []
  );

  const taskById = useCallback(
    (taskId: string): CreativeTask => {
      const task = presentationRuntime.entries.find((entry) => entry.task.taskId === taskId)?.task;
      if (!task) {
        throw new Error(
          t('creativeStudio.product.errors.taskNotFound', {
            defaultValue: '找不到任务 {{taskId}}。',
            taskId,
          })
        );
      }
      return task;
    },
    [presentationRuntime.entries, t]
  );

  const generate = async (): Promise<void> => {
    setError(null);
    if (draftReferencesRestoring) {
      setError(
        t('creativeStudio.product.errors.referencesRestoring', {
          defaultValue: '参考素材草稿仍在恢复，未发起生成。',
        })
      );
      return;
    }
    if (!model || catalog.status !== 'ready') {
      setError(
        t('creativeStudio.product.video.errors.modelRequired', {
          defaultValue: '没有可用且明确选择的真实视频模型，未发起生成。',
        })
      );
      return;
    }
    if (
      references.length !== referenceIds.length ||
      references.length > 1 ||
      references.some((asset) => asset.kind !== 'image')
    ) {
      setError(
        t('creativeStudio.product.video.errors.referenceLimit', {
          defaultValue: '当前视频生成只支持一张可读取的真实图片参考；V2V 与多图引用尚未开放。',
        })
      );
      return;
    }
    try {
      const capability = references.length === 1 ? 'i2v' : 't2v';
      const dimensions = videoDimensions(resolution, aspect, t);
      await runtime.generate({
        catalog,
        owner: standaloneWorkbenchOwner('video'),
        model,
        references: {
          assets: references,
          bindings: references.map((asset) => ({
            assetId: asset.id,
            kind: 'image' as const,
            role: 'reference' as const,
          })),
        },
        operation: { task: 'video_generation', capability },
        prompt,
        seconds: Number(duration),
        width: dimensions.width,
        height: dimensions.height,
        taskCount,
      });
    } catch (reason) {
      setError(formatError(reason));
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
      setError(formatError(reason));
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
        setError(formatError(reason));
      }
    }
  };

  const loadTask = async (taskId: string): Promise<void> => {
    const generation = loadGenerationRef.current + 1;
    loadGenerationRef.current = generation;
    setError(null);
    try {
      const task = taskById(taskId);
      const controls = videoControlsFromTask(task, t);
      const nextReferences = await hydrateStandaloneTaskReferences(task, creativeAssetClient);
      if (nextReferences.assets.length > 1 || nextReferences.assets.some((asset) => asset.kind !== 'image')) {
        throw new Error(
          t('creativeStudio.product.video.errors.unsupportedHistoryReference', {
            defaultValue: '该历史任务使用当前视频工作台未开放的参考类型。',
          })
        );
      }
      if (loadGenerationRef.current !== generation) return;
      setPrompt(controls.prompt);
      setModel({ providerId: task.providerId as CreativeModelSelectionRef['providerId'], model: task.model });
      setResolution(controls.resolution);
      setAspect(controls.aspect);
      setDuration(controls.duration);
      setTaskCount(1);
      setHydratedReferences([...nextReferences.assets]);
      setReferenceIds(nextReferences.bindings.map((binding) => binding.assetId));
    } catch (reason) {
      if (loadGenerationRef.current !== generation) return;
      setError(formatError(reason));
    }
  };

  const requestRetirement = (taskIds: readonly string[]): void => {
    setError(null);
    setRetireError(null);
    const unique = [...new Set(taskIds)];
    try {
      if (unique.length === 0 || unique.length > 100) {
        throw new Error(
          t('creativeStudio.product.history.invalidSelection', {
            defaultValue: '每次必须选择 1-100 条终态历史。',
          })
        );
      }
      for (const taskId of unique) {
        const task = taskById(taskId);
        if (task.status === 'queued' || task.status === 'running') {
          throw new Error(
            t('creativeStudio.product.history.runningTask', {
              defaultValue: '运行中的任务必须先取消，不能直接从历史移除。',
            })
          );
        }
      }
      setRetireTaskIds(unique);
    } catch (reason) {
      setError(formatError(reason));
    }
  };

  const confirmRetirement = async (): Promise<void> => {
    if (retireTaskIds.length === 0 || retiring) return;
    setRetiring(true);
    setError(null);
    setRetireError(null);
    try {
      const result = await creativeTaskHistoryClient.retireStandalone({
        workbenchKind: 'video',
        taskIds: retireTaskIds,
      });
      runtime.dismiss(result.retiredTaskIds);
      setRetiredTaskIds((current) => [...new Set([...current, ...result.retiredTaskIds])]);
      setSelectedTaskIds([]);
      setRetireTaskIds([]);
      await history.reload();
    } catch (reason) {
      setRetireError(formatError(reason));
    } finally {
      setRetiring(false);
    }
  };

  const modelSlot = (
    <CreativeModelSelect
      catalog={catalog}
      filter={{ capability: 'task', task: 'video_generation' }}
      value={model}
      onChange={setModel}
      disabled={busy}
    />
  );
  const baseProps = {
    layout,
    onLayoutChange: setLayout,
    prompt,
    onPromptChange: setPrompt,
    submitDisabled:
      draftReferencesRestoring ||
      catalog.status !== 'ready' ||
      model === null ||
      history.refreshing,
    references: videoWorkbenchReferencesFromAssets(references),
    addReferenceLabel: t('creativeStudio.product.video.addImageReference', {
      defaultValue: '添加图片参考',
    }),
    onAddReferences: () => setPickerOpen(true),
    onRemoveReference: (referenceId: string) =>
      setReferenceIds((ids) => ids.filter((id) => id !== referenceId)),
    onMoveReference: (referenceId: string, direction: -1 | 1) =>
      setReferenceIds((ids) => {
        const index = ids.indexOf(referenceId);
        const target = index + direction;
        if (index < 0 || target < 0 || target >= ids.length) return ids;
        const next = [...ids];
        [next[index], next[target]] = [next[target] as string, next[index] as string];
        return next;
      }),
    modelSlot,
    resolution,
    resolutionOptions: RESOLUTIONS,
    onResolutionChange: setResolution,
    size: aspect,
    sizeOptions: ASPECTS,
    onSizeChange: setAspect,
    duration,
    durationOptions,
    onDurationChange: setDuration,
    taskCount,
    onTaskCountChange: (count: number) => {
      if (!Number.isSafeInteger(count) || count < 1 || count > STANDALONE_VIDEO_MAX_CONCURRENT_TASKS) {
        setError(
          t('creativeStudio.product.video.errors.concurrentCount', {
            defaultValue: '视频并发任务数必须在 1-{{max}} 之间。',
            max: STANDALONE_VIDEO_MAX_CONCURRENT_TASKS,
          })
        );
        return;
      }
      setTaskCount(count);
    },
    onOpenParameters: () =>
      setError(
        t('creativeStudio.product.video.parametersUnavailable', {
          defaultValue: '高级参数只会在具有明确 NomiFun 协议契约后开放，当前未发送隐藏参数。',
        })
      ),
    onOpenPromptLibrary: () => navigate('/workshop/prompts'),
    selectedTaskIds,
    onSelectedTaskIdsChange: (ids: readonly string[]) => setSelectedTaskIds([...ids]),
    onDeleteTasks: requestRetirement,
    onNewSession: () => {
      loadGenerationRef.current += 1;
      setPrompt('');
      setReferenceIds([]);
      setHydratedReferences([]);
      setTaskCount(1);
      setSelectedTaskIds([]);
      setError(null);
    },
    onLoadTask: (taskId: string) => void loadTask(taskId),
    onCancelTask: (taskId: string) => void cancelTask(taskId),
    onInspectTask: (taskId: string) => {
      try {
        const task = taskById(taskId);
        setError(
          task.error
            ? `${task.error.kind}: ${task.error.message}`
            : t('creativeStudio.product.video.errors.noDetails', {
                defaultValue: '任务没有错误详情。',
              })
        );
      } catch (reason) {
        setError(formatError(reason));
      }
    },
    onCopyPrompt: (value: string) => {
      void navigator.clipboard.writeText(value).catch((reason) =>
        setError(formatError(reason))
      );
    },
    onDownloadTask: (taskId: string) => {
      try {
        const task = taskById(taskId);
        const assetId = task.resultAssetIds[0];
        if (!assetId || task.resultAssetIds.length !== 1) {
          setError(
            t('creativeStudio.product.video.errors.noDownloadResult', {
              defaultValue: '视频任务没有唯一可下载结果。',
            })
          );
          return;
        }
        const link = document.createElement('a');
        link.href = creativeAssetClient.url(assetId, 'original');
        link.download = `${assetId}.mp4`;
        link.click();
      } catch (reason) {
        setError(formatError(reason));
      }
    },
    historyLoadingMore: history.loadingMore,
    historyHasMore: history.nextCursor !== null,
    onLoadMoreTasks: () => void history.loadMore(),
  };
  const props = createVideoWorkbenchRuntimeProps({
    base: baseProps,
    runtime: presentationRuntime,
    catalog,
    onGenerate: generate,
    onRetryTask: retryTask,
    onActionError: (reason) => setError(formatError(reason)),
  });

  return (
    <>
      {history.error || error ? (
        <div className={styles.runtimeNotice} role='alert'>
          {history.error ? formatError(history.error) : error}
        </div>
      ) : null}
      {presentationRuntime.submissionFailures.map((failure) => (
        <div className={styles.runtimeNotice} role='status' key={failure.order}>
          <span>
            {t('creativeStudio.product.submission.uncertain', {
              defaultValue: '任务提交结果尚未确认，可使用原幂等请求安全重试。',
            })}
          </span>
          <button
            type='button'
            onClick={() => {
              void runtime.retrySubmission(failure.order).catch((reason) =>
                setError(formatError(reason))
              );
            }}
          >
            {t('creativeStudio.product.submission.confirmStatus', {
              defaultValue: '确认任务状态',
            })}
          </button>
        </div>
      ))}
      {presentationRuntime.requestError && history.activeTasks.length > 0 ? (
        <div className={styles.runtimeNotice} role='status'>
          <span>{formatError(presentationRuntime.requestError)}</span>
          <button
            type='button'
            onClick={() => {
              void runtime.resume(initialResumeRequests).catch((reason) =>
                setError(formatError(reason))
              );
            }}
          >
            {t('creativeStudio.product.submission.retrySync', {
              defaultValue: '重试任务同步',
            })}
          </button>
        </div>
      ) : null}
      <VideoWorkbench {...props} />
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
          setReferenceIds((ids) => (ids.includes(asset.id) ? [] : [asset.id]))
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
            .then((uploaded) => {
              const first = uploaded[0];
              if (!first) return;
              if (first.kind !== 'image') {
                setError(
                  t('creativeStudio.product.video.errors.imageReferenceOnly', {
                    defaultValue: '当前视频生成只支持图片参考。',
                  })
                );
                return;
              }
              setReferenceIds([first.id]);
            })
            .catch((reason) => setError(formatError(reason)));
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

const OwnedVideoWorkbench: React.FC = () => {
  const { t } = useTranslation();
  const historyScope = useMemo(() => ({ workbenchKind: 'video' as const }), []);
  const history = useStandaloneWorkbenchHistory(historyScope);
  if (history.status !== 'ready') {
    return (
      <StandaloneHistoryGate
        label={t('creativeStudio.product.video.historyLabel', { defaultValue: '视频' })}
        error={history.error}
        onRetry={() => void history.reload()}
      />
    );
  }
  return <OwnedVideoWorkbenchReady history={history} />;
};

/** Router-ready, prop-free standalone video product. */
const VideoWorkbenchProductRoute: React.FC = () => {
  return (
    <StandaloneWorkbenchPage error={null}>
      <OwnedVideoWorkbench />
    </StandaloneWorkbenchPage>
  );
};

export default VideoWorkbenchProductRoute;
