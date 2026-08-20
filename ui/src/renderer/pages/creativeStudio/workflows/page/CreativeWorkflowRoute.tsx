/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  creativeAssetClient,
  useCreativeAssetPickerDialog,
} from '../../assets';
import { useCreativeWorkflowRuntime } from '../runtime';
import CreativeWorkflowWorkspacePage from './CreativeWorkflowWorkspacePage';
import type { CreativeWorkflowRunnerPort } from './WorkflowRunModal';

const CreativeWorkflowRoute: React.FC = () => {
  const navigate = useNavigate();
  const { controller, snapshot } = useCreativeWorkflowRuntime();
  const assetPicker = useCreativeAssetPickerDialog();
  const runner = useMemo<CreativeWorkflowRunnerPort>(() => ({
    async start(input) {
      await controller.start(input);
    },
  }), [controller]);

  return (
    <>
      <CreativeWorkflowWorkspacePage
        runner={runner}
        runCenter={{
          snapshot,
          assetUrl: (assetId) => creativeAssetClient.url(assetId),
          resume: (runId) => controller.resume(runId),
          cancel: (runId) => controller.cancel(runId),
          review: (runId, drafts) => controller.review(runId, drafts),
          retry: (run) => controller.start({
            workflow: run.workflowSnapshot,
            inputs: run.request.inputs,
            referenceAssetIds: run.request.referenceAssetIds,
          }),
        }}
        onOpenModelSettings={() => void navigate('/models')}
        onPickAssets={(variable, selectedAssetIds) => assetPicker.pick({
          acceptedKinds: ['image'],
          initialSelectedIds: selectedAssetIds,
          selectionLimit: variable.type === 'image-series' ? variable.maxItems : 1,
          title: variable.type === 'image-series' ? '选择变量图片' : '选择变量参考图',
        })}
        onPickReferenceAssets={(selectedAssetIds) => assetPicker.pick({
          acceptedKinds: ['image'],
          initialSelectedIds: selectedAssetIds,
          selectionLimit: 100,
          title: '选择工作流参考图',
        })}
        onUploadReferenceImages={async (files, selectedAssetIds) => {
          const assets = await Promise.all(
            files.map((file) => creativeAssetClient.upload(file, {
              title: file.name,
              tags: ['workflow-reference'],
              inLibrary: true,
            }))
          );
          return [...new Set([...selectedAssetIds, ...assets.map((asset) => asset.id)])];
        }}
      />
      {assetPicker.dialog}
    </>
  );
};

export default CreativeWorkflowRoute;
