/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import {
  creativeAssetClient,
  CreativeAssetDeletedError,
  isCreativeAssetDeleted,
  useCreativeAssetPickerDialog,
} from '../../assets';
import { useNomiCreativeModelCatalog } from '../../models';
import { templateDraftPort } from '../agent';
import { useCreativeTemplateRuntime } from '../runtime';
import CreativeTemplateWorkspacePage from './CreativeTemplateWorkspacePage';
import type { CreativeTemplateRunnerPort } from './TemplateRunModal';
import type { StartCreativeTemplateRun } from '../runtime/types';

async function validateRunAssets(input: StartCreativeTemplateRun): Promise<void> {
  const ids = [...new Set([
    ...input.referenceAssetIds,
    ...input.inputs.flatMap((value) => value.type === 'image'
      ? value.assetId ? [value.assetId] : []
      : value.type === 'image-series' ? value.assetIds : []),
  ])];
  const assets = await Promise.all(ids.map((id) => creativeAssetClient.get(id)));
  const deleted = assets.find(isCreativeAssetDeleted);
  if (deleted) throw new CreativeAssetDeletedError(deleted.id);
}

const CreativeTemplateRoute: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { controller, snapshot } = useCreativeTemplateRuntime();
  const assetPicker = useCreativeAssetPickerDialog();
  const modelCatalog = useNomiCreativeModelCatalog();
  const runner = useMemo<CreativeTemplateRunnerPort>(() => ({
    async start(input) {
      await validateRunAssets(input);
      await controller.start(input);
    },
  }), [controller]);

  return (
    <>
      <CreativeTemplateWorkspacePage
        agentDraftPort={templateDraftPort}
        agentModelCatalog={modelCatalog}
        runner={runner}
        runCenter={{
          snapshot,
          assetUrl: (assetId) => creativeAssetClient.url(assetId),
          resume: (templateRunId) => controller.resume(templateRunId),
          cancel: (templateRunId) => controller.cancel(templateRunId),
          review: (templateRunId, drafts) => controller.review(templateRunId, drafts),
          retry: (run) => runner.start({
            template: run.templateSnapshot,
            inputs: run.request.inputs,
            referenceAssetIds: run.request.referenceAssetIds,
          }),
        }}
        onOpenModelSettings={() => void navigate('/models')}
        onPickAssets={(variable, selectedAssetIds) => assetPicker.pick({
          acceptedKinds: ['image'],
          initialSelectedIds: selectedAssetIds,
          selectionLimit: variable.type === 'image-series' ? variable.maxItems : 1,
          title: t(
            variable.type === 'image-series'
              ? 'creativeStudio.templates.picker.variableImages'
              : 'creativeStudio.templates.picker.variableImage',
            {
              defaultValue:
                variable.type === 'image-series'
                  ? 'Select variable images'
                  : 'Select variable reference image',
            }
          ),
        })}
        onPickReferenceAssets={(selectedAssetIds) => assetPicker.pick({
          acceptedKinds: ['image'],
          initialSelectedIds: selectedAssetIds,
          selectionLimit: 100,
          title: t('creativeStudio.templates.picker.templateReference', {
            defaultValue: 'Select template reference images',
          }),
        })}
        onUploadReferenceImages={async (files, selectedAssetIds) => {
          const assets = await Promise.all(
            files.map((file) => creativeAssetClient.upload(file, {
              title: file.name,
              tags: ['template-reference'],
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

export default CreativeTemplateRoute;
