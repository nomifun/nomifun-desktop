/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  CreativeAssetPickerModal,
  creativeAssetClient,
  useCreativeAssets,
  type CreativeAsset,
} from '../../assets';
import type { CreativeProjectDetail } from '../../domain';
import {
  CreativeModelSelect,
  useNomiCreativeModelCatalog,
  type CreativeModelSelectionRef,
} from '../../models';
import { creativeTaskClient } from '../../tasks';
import {
  VideoWorkbench,
  type VideoWorkbenchLayout,
} from '../video';
import {
  createVideoWorkbenchRuntimeProps,
  videoWorkbenchReferencesFromAssets,
  useVideoWorkbenchRuntime,
} from '../runtime';
import {
  ensureStandaloneWorkbenchNode,
  findStandaloneWorkbenchNode,
  STANDALONE_VIDEO_MAX_CONCURRENT_TASKS,
} from './ownership';
import {
  StandaloneWorkbenchPage,
  useStandaloneWorkbenchScope,
} from './shared';
import { useStandalonePersistence } from './useStandalonePersistence';
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
const DURATIONS = [
  { value: '5', label: '5 秒' },
  { value: '10', label: '10 秒' },
];

const UnownedVideoWorkbench: React.FC = () => {
  const catalog = useNomiCreativeModelCatalog();
  const [layout, setLayout] = useState<VideoWorkbenchLayout>('side');
  const [prompt, setPrompt] = useState('');
  const [model, setModel] = useState<CreativeModelSelectionRef | null>(null);
  return (
    <VideoWorkbench
      layout={layout}
      onLayoutChange={setLayout}
      prompt={prompt}
      onPromptChange={setPrompt}
      onGenerate={() => undefined}
      submitDisabled
      references={[]}
      onAddReferences={() => undefined}
      onRemoveReference={() => undefined}
      modelSlot={
        <CreativeModelSelect
          catalog={catalog}
          filter={{ capability: 'task', task: 'video_generation' }}
          value={model}
          onChange={setModel}
          disabled
        />
      }
      resolution='1080p'
      resolutionOptions={RESOLUTIONS}
      onResolutionChange={() => undefined}
      size='16:9'
      sizeOptions={ASPECTS}
      onSizeChange={() => undefined}
      duration='5'
      durationOptions={DURATIONS}
      onDurationChange={() => undefined}
      taskCount={1}
      onTaskCountChange={() => undefined}
      onOpenParameters={() => undefined}
      tasks={[]}
      selectedTaskIds={[]}
      onSelectedTaskIdsChange={() => undefined}
      onDeleteTasks={() => undefined}
    />
  );
};

const OwnedVideoWorkbench: React.FC<{
  detail: CreativeProjectDetail;
  refresh(): Promise<CreativeProjectDetail | undefined>;
}> = ({ detail, refresh }) => {
  const navigate = useNavigate();
  const catalog = useNomiCreativeModelCatalog();
  const assets = useCreativeAssets({ pageSize: 200, query: { sort: 'updated_desc' } });
  const initialNode = useMemo(() => findStandaloneWorkbenchNode(detail.document, 'video'), []);
  const [layout, setLayout] = useState<VideoWorkbenchLayout>('side');
  const [prompt, setPrompt] = useState(initialNode?.data.prompt ?? '');
  const [model, setModel] = useState<CreativeModelSelectionRef | null>(
    initialNode?.data.providerId && initialNode.data.model
      ? { providerId: initialNode.data.providerId as CreativeModelSelectionRef['providerId'], model: initialNode.data.model }
      : null
  );
  const [resolution, setResolution] = useState('1080p');
  const [aspect, setAspect] = useState('16:9');
  const [duration, setDuration] = useState('5');
  const [referenceIds, setReferenceIds] = useState<string[]>(initialNode?.data.inputAssetIds ?? []);
  const [selectedTaskIds, setSelectedTaskIds] = useState<readonly string[]>([]);
  const [hiddenTaskIds, setHiddenTaskIds] = useState<readonly string[]>([]);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const references = useMemo(
    () => referenceIds.flatMap((id) => assets.assets.find((asset) => asset.id === id) ?? []),
    [assets.assets, referenceIds]
  );
  const persistence = useStandalonePersistence({ kind: 'video', detail, refresh });
  const runtime = useVideoWorkbenchRuntime({
    scopeKey: `${detail.project.projectId}:standalone-video`,
    tasks: creativeTaskClient,
    assets: creativeAssetClient,
    initialResumeRequests: persistence.initialResumeRequests,
    onPendingTask: persistence.onPendingTask,
    onSettledTask: persistence.onSettledTask,
    onRecoveryFailure: persistence.onRecoveryFailure,
    onRuntimeError: (reason) => setError(reason instanceof Error ? reason.message : String(reason)),
  });

  const generate = async (): Promise<void> => {
    setError(null);
    setHiddenTaskIds([]);
    if (!model || catalog.status !== 'ready') {
      setError('没有可用且明确选择的真实视频模型，未发起生成。');
      return;
    }
    const kinds = new Set(references.map((asset) => asset.kind));
    if (kinds.size > 1 || [...kinds].some((kind) => kind !== 'image' && kind !== 'video')) {
      setError('一次视频任务只能使用同一种兼容参考素材。');
      return;
    }
    try {
      const capability = references[0]?.kind === 'image' ? 'i2v' : references[0]?.kind === 'video' ? 'v2v' : 't2v';
      const node = await ensureStandaloneWorkbenchNode(detail.project.projectId, 'video', {
        task: 'video_generation',
        capability,
        prompt,
        providerId: model.providerId,
        model: model.model,
        parameters: { resolution, aspect, seconds: Number(duration) },
        inputAssetIds: references.map((asset) => asset.id),
      });
      await refresh();
      await runtime.generate({
        catalog,
        projectId: detail.project.projectId,
        nodeId: node.id,
        model,
        references: {
          assets: references,
          bindings: references.map((asset) => ({
            assetId: asset.id,
            kind: asset.kind,
            role: asset.kind === 'video' ? ('video' as const) : ('reference' as const),
          })),
        },
        operation: { task: 'video_generation', capability },
        prompt,
        resolution,
        aspectRatio: aspect,
        seconds: Number(duration),
        width: null,
        height: null,
        taskCount: STANDALONE_VIDEO_MAX_CONCURRENT_TASKS,
      });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  const modelSlot = (
    <CreativeModelSelect
      catalog={catalog}
      filter={{ capability: 'task', task: 'video_generation' }}
      value={model}
      onChange={setModel}
      disabled={runtime.submittingCount > 0 || runtime.recoveringCount > 0}
    />
  );
  const baseProps = {
    layout,
    onLayoutChange: setLayout,
    prompt,
    onPromptChange: setPrompt,
    submitDisabled:
      persistence.resumeError !== null || catalog.status !== 'ready' || model === null,
    references: videoWorkbenchReferencesFromAssets(references),
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
    durationOptions: DURATIONS,
    onDurationChange: setDuration,
    taskCount: STANDALONE_VIDEO_MAX_CONCURRENT_TASKS,
    onTaskCountChange: (count: number) => {
      if (count !== STANDALONE_VIDEO_MAX_CONCURRENT_TASKS) {
        setError('当前 canonical config node 只能拥有一个 pending task；多视频并发需 standalone owner-union 后再开放。');
      }
    },
    onOpenParameters: () =>
      setError('高级参数只会在具有明确 NomiFun 协议契约后开放，当前未发送隐藏参数。'),
    onOpenPromptLibrary: () => navigate('/workshop/prompts'),
    selectedTaskIds,
    onSelectedTaskIdsChange: setSelectedTaskIds,
    onDeleteTasks: (ids: readonly string[]) => {
      const selected = runtime.entries.filter((entry) => ids.includes(entry.task.taskId));
      if (selected.some((entry) => entry.task.status !== 'succeeded')) {
        setError('后端没有删除任务历史的能力；未伪装删除失败或运行中的任务。');
        return;
      }
      void Promise.all(
        selected.flatMap((entry) => entry.outputs.map((output) => creativeAssetClient.remove(output.assetId)))
      )
        .then(async () => {
          setHiddenTaskIds((current) => [...new Set([...current, ...ids])]);
          setSelectedTaskIds([]);
          await assets.reload();
        })
        .catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
    },
    onNewSession: () => {
      runtime.reset();
      setHiddenTaskIds([]);
    },
    onCopyPrompt: (value: string) => void navigator.clipboard.writeText(value),
    onDownloadTask: (taskId: string) => {
      const output = runtime.entries.find((entry) => entry.task.taskId === taskId)?.outputs[0];
      if (!output?.url) return;
      const link = document.createElement('a');
      link.href = output.url;
      link.download = `${output.assetId}.mp4`;
      link.click();
    },
  };
  const wiredProps = createVideoWorkbenchRuntimeProps({
    base: baseProps,
    runtime,
    catalog,
    onGenerate: generate,
    onRetryTask: runtime.retry,
    onActionError: (reason) => setError(reason instanceof Error ? reason.message : String(reason)),
  });
  const props = {
    ...wiredProps,
    tasks: wiredProps.tasks.filter((task) => !hiddenTaskIds.includes(task.id)),
  };

  return (
    <>
      {persistence.resumeError || error ? (
        <div className={styles.runtimeNotice} role='alert'>{persistence.resumeError?.message ?? error}</div>
      ) : null}
      <VideoWorkbench {...props} />
      <CreativeAssetPickerModal
        open={pickerOpen}
        assets={assets.assets}
        acceptedKinds={['image', 'video']}
        selectedIds={referenceIds}
        loading={assets.loading}
        loadingMore={assets.loadingMore}
        hasMore={assets.hasMore}
        error={assets.error ?? assets.mutationError}
        uploading={assets.mutating}
        onToggle={(asset: CreativeAsset) =>
          setReferenceIds((ids) => {
            if (ids.includes(asset.id)) return ids.filter((id) => id !== asset.id);
            const existing = ids.flatMap((id) => assets.assets.find((item) => item.id === id) ?? []);
            return existing.some((item) => item.kind !== asset.kind) ? [asset.id] : [...ids, asset.id];
          })
        }
        onLoadMore={() => void assets.loadMore()}
        onRetry={() => void assets.reload()}
        onUploadFiles={(files) => {
          void Promise.all(
            files.map((file) => assets.upload(file, {
              title: file.name,
              tags: ['workbench-reference'],
              inLibrary: true,
            }))
          )
            .then((uploaded) => {
              const firstKind = uploaded[0]?.kind;
              if (!firstKind) return;
              if (uploaded.some((asset) => asset.kind !== firstKind)) {
                setError('一次视频任务只能使用同一种兼容参考素材。');
              }
              const compatible = uploaded.filter((asset) => asset.kind === firstKind);
              setReferenceIds((ids) => {
                const existing = ids.flatMap(
                  (id) => assets.assets.find((asset) => asset.id === id) ?? []
                );
                return existing.some((asset) => asset.kind !== firstKind)
                  ? compatible.map((asset) => asset.id)
                  : [...new Set([...ids, ...compatible.map((asset) => asset.id)])];
              });
            })
            .catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
        }}
        onCancel={() => setPickerOpen(false)}
      />
    </>
  );
};

/** Router-ready, prop-free standalone video product. */
const VideoWorkbenchProductRoute: React.FC = () => {
  const scope = useStandaloneWorkbenchScope();
  return (
    <StandaloneWorkbenchPage scope={scope} error={null}>
      {scope.state === 'ready' && scope.detail ? (
        <OwnedVideoWorkbench
          key={scope.detail.project.projectId}
          detail={scope.detail}
          refresh={scope.refreshProject}
        />
      ) : (
        <UnownedVideoWorkbench />
      )}
    </StandaloneWorkbenchPage>
  );
};

export default VideoWorkbenchProductRoute;
