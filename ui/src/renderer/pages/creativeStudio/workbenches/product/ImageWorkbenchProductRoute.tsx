/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  CreativeAssetPickerModal,
  creativeAssetClient,
  useCreativeAssets,
  type CreativeAsset,
} from '../../assets';
import type { CreativeProjectDetail } from '../../domain';
import { useNomiCreativeModelCatalog } from '../../models';
import { creativeTaskClient } from '../../tasks';
import {
  ImageWorkbench,
  type ImageWorkbenchAspectRatioOption,
  type ImageWorkbenchLayout,
  type ImageWorkbenchModelIdentity,
  type ImageWorkbenchSettings,
} from '../image';
import {
  createImageWorkbenchRuntimeProps,
  exactWorkbenchModelOptions,
  imageWorkbenchModelOptions,
  imageWorkbenchReferencesFromAssets,
  useImageWorkbenchRuntime,
} from '../runtime';
import {
  ensureStandaloneWorkbenchNode,
  findStandaloneWorkbenchNode,
} from './ownership';
import {
  StandaloneWorkbenchPage,
  useStandaloneWorkbenchScope,
} from './shared';
import { useStandalonePersistence } from './useStandalonePersistence';
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

const UnownedImageWorkbench: React.FC = () => {
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
      onLayoutChange={setLayout}
      onPromptChange={setPrompt}
      onRemoveReference={() => undefined}
      onModelChange={(model) => setSettings((value) => ({ ...value, model }))}
      onInterfaceModeChange={(interfaceMode) => setSettings((value) => ({ ...value, interfaceMode }))}
      onQualityChange={(quality) => setSettings((value) => ({ ...value, quality }))}
      onDimensionsChange={(dimensions) => setSettings((value) => ({ ...value, ...dimensions }))}
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
      onDeleteResult={() => undefined}
      onDeleteSelected={() => undefined}
    />
  );
};

const OwnedImageWorkbench: React.FC<{
  detail: CreativeProjectDetail;
  refresh(): Promise<CreativeProjectDetail | undefined>;
}> = ({ detail, refresh }) => {
  const navigate = useNavigate();
  const catalog = useNomiCreativeModelCatalog();
  const assets = useCreativeAssets({ pageSize: 200, query: { sort: 'updated_desc' } });
  const initialNode = useMemo(() => findStandaloneWorkbenchNode(detail.document, 'image'), []);
  const [layout, setLayout] = useState<ImageWorkbenchLayout>('side');
  const [prompt, setPrompt] = useState(initialNode?.data.prompt ?? '');
  const [settings, setSettings] = useState<ImageWorkbenchSettings>(() => ({
    ...EMPTY_SETTINGS,
    model:
      initialNode?.data.providerId && initialNode.data.model
        ? { providerId: initialNode.data.providerId, model: initialNode.data.model }
        : null,
  }));
  const [referenceIds, setReferenceIds] = useState<string[]>(initialNode?.data.inputAssetIds ?? []);
  const [selectedResultIds, setSelectedResultIds] = useState<string[]>([]);
  const [hiddenResultIds, setHiddenResultIds] = useState<string[]>([]);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const references = useMemo(
    () => referenceIds.flatMap((id) => assets.assets.find((asset) => asset.id === id) ?? []),
    [assets.assets, referenceIds]
  );
  const modelTask = references.length ? 'image_edit' : 'image_generation';
  const persistence = useStandalonePersistence({ kind: 'image', detail, refresh });
  const runtime = useImageWorkbenchRuntime({
    scopeKey: `${detail.project.projectId}:standalone-image`,
    tasks: creativeTaskClient,
    assets: creativeAssetClient,
    initialResumeRequests: persistence.initialResumeRequests,
    onPendingTask: persistence.onPendingTask,
    onSettledTask: persistence.onSettledTask,
    onRecoveryFailure: persistence.onRecoveryFailure,
    onRuntimeError: (reason) => setError(reason instanceof Error ? reason.message : String(reason)),
  });

  useEffect(() => {
    if (!settings.model || catalog.status !== 'ready') return;
    const stillAvailable = exactWorkbenchModelOptions(catalog, modelTask).some(
      (option) =>
        option.providerId === settings.model?.providerId && option.model === settings.model.model
    );
    if (!stillAvailable) setSettings((value) => ({ ...value, model: null }));
  }, [catalog, modelTask, settings.model]);

  const generate = async (): Promise<void> => {
    setError(null);
    setHiddenResultIds([]);
    if (!settings.model || catalog.status !== 'ready') {
      setError('没有可用且明确选择的真实模型，未发起生成。');
      return;
    }
    try {
      const node = await ensureStandaloneWorkbenchNode(detail.project.projectId, 'image', {
        task: references.length ? 'image_edit' : 'image_generation',
        capability: references.length ? 'i2i' : 't2i',
        prompt,
        providerId: settings.model.providerId,
        model: settings.model.model,
        parameters: {
          interface_mode: settings.interfaceMode,
          quality: settings.quality,
          aspect: settings.aspectRatio,
          count: settings.count,
          ...(settings.width === null ? {} : { width: settings.width }),
          ...(settings.height === null ? {} : { height: settings.height }),
        },
        inputAssetIds: references.map((asset) => asset.id),
      });
      await refresh();
      await runtime.generate({
        catalog,
        projectId: detail.project.projectId,
        nodeId: node.id,
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

  const removeResults = async (ids: readonly string[]): Promise<void> => {
    const results = createImageWorkbenchRuntimeProps({
      base: baseProps,
      runtime,
      catalog,
      task: modelTask,
      onGenerate: generate,
      onActionError: (reason) => setError(reason instanceof Error ? reason.message : String(reason)),
    }).results;
    const selected = results.filter((result) => ids.includes(result.id));
    if (selected.some((result) => result.status !== 'succeeded')) {
      setError('后端没有删除任务历史的能力；未伪装删除失败或运行中的任务。');
      return;
    }
    try {
      await Promise.all(
        selected.map((result) =>
          result.status === 'succeeded' ? creativeAssetClient.remove(result.assetId) : Promise.resolve()
        )
      );
      setHiddenResultIds((current) => [...new Set([...current, ...ids])]);
      setSelectedResultIds([]);
      await assets.reload();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
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
    onDeleteResult: (resultId: string) => void removeResults([resultId]),
    onDeleteSelected: (resultIds: string[]) => void removeResults(resultIds),
  };
  const wiredProps = createImageWorkbenchRuntimeProps({
    base: baseProps,
    runtime,
    catalog,
    task: modelTask,
    disabled: persistence.resumeError !== null || catalog.status !== 'ready',
    onGenerate: generate,
    onRetryTask: runtime.retry,
    onActionError: (reason) => setError(reason instanceof Error ? reason.message : String(reason)),
  });
  const props = {
    ...wiredProps,
    results: wiredProps.results.filter((result) => !hiddenResultIds.includes(result.id)),
  };

  return (
    <>
      {persistence.resumeError || error ? (
        <div className={styles.runtimeNotice} role='alert'>{persistence.resumeError?.message ?? error}</div>
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
            files.map((file) => assets.upload(file, {
              title: file.name,
              tags: ['workbench-reference'],
              inLibrary: true,
            }))
          )
            .then((uploaded) => setReferenceIds((ids) => [
              ...new Set([...ids, ...uploaded.map((asset) => asset.id)]),
            ]))
            .catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
        }}
        onCancel={() => setPickerOpen(false)}
      />
    </>
  );
};

/** Router-ready, prop-free standalone image product. */
const ImageWorkbenchProductRoute: React.FC = () => {
  const scope = useStandaloneWorkbenchScope();
  const [error] = useState<string | null>(null);
  return (
    <StandaloneWorkbenchPage scope={scope} error={error}>
      {scope.state === 'ready' && scope.detail ? (
        <OwnedImageWorkbench
          key={scope.detail.project.projectId}
          detail={scope.detail}
          refresh={scope.refreshProject}
        />
      ) : (
        <UnownedImageWorkbench />
      )}
    </StandaloneWorkbenchPage>
  );
};

export default ImageWorkbenchProductRoute;
