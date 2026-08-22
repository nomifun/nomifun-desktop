/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IProvider } from '@/common/config/storage';
import ModalHOC from '@/renderer/utils/ui/ModalHOC';
import NomiModal from '@/renderer/components/base/NomiModal';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import useModeModeList from '@renderer/hooks/agent/useModeModeList';
import ModelDefinitionEditor, { type ModelCatalogSuggestion } from './ModelDefinitionEditor';
import {
  capabilityInputsFromDefinition,
  describeValidationErrors,
  normalizeModelId,
  validateModelDefinition,
  type ModelDefinitionDraft,
} from './providerModelAdvanced';
import useModelProtocolManifests from './useModelProtocolManifests';
import { useProviderConnections } from './useProviderConnections';

const EMPTY_DEFINITION: ModelDefinitionDraft = { model: '', capabilities: [] };

const AddModelModal = ModalHOC<{ data?: IProvider; onSubmit: (provider: IProvider) => void }>(
  ({ modalProps, data, onSubmit, modalCtrl }) => {
    const { t } = useTranslation();
    const [message, messageHolder] = useArcoMessage();
    const [definition, setDefinition] = useState<ModelDefinitionDraft>(EMPTY_DEFINITION);
    const [saving, setSaving] = useState(false);
    const tasks = useMemo(
      () => definition.capabilities.map((capability) => capability.task),
      [definition.capabilities]
    );
    const manifests = useModelProtocolManifests({
      preset: data?.platform,
      tasks,
      baseUrlHint: data?.base_url,
      modelHint: definition.model,
    });
    const connectionState = useProviderConnections(data?.id, modalProps.visible && Boolean(data));
    const modelListState = useModeModeList({
      platform: data?.platform ?? '',
      providerId: data?.id,
    });
    const catalogSuggestions = useMemo<ModelCatalogSuggestion[]>(
      () =>
        (modelListState.data?.models ?? []).map((model) => ({
          value: model.value,
          label: model.label,
          tasks: model.tasks,
          traits: model.traits,
          ...(model.contextLimit === undefined ? {} : { contextLimit: model.contextLimit }),
        })),
      [modelListState.data?.models]
    );
    const existingModelIds = useMemo(() => data?.models.map((row) => row.model) ?? [], [data?.models]);
    const validation = useMemo(
      () =>
        validateModelDefinition(
          definition,
          manifests.manifests,
          data?.base_url ?? '',
          existingModelIds,
          manifests.loadingTasks,
          connectionState.connections.map((connection) => connection.role),
          data?.auth_scheme ?? '',
          Object.fromEntries(
            connectionState.connections.map((connection) => [connection.role, connection.auth_scheme])
          ),
          connectionState.connections
        ),
      [
        connectionState.connections,
        data?.base_url,
        data?.auth_scheme,
        definition,
        existingModelIds,
        manifests.loadingTasks,
        manifests.manifests,
      ]
    );

    useEffect(() => {
      if (modalProps.visible) {
        setDefinition(EMPTY_DEFINITION);
        setSaving(false);
      }
    }, [data?.id, modalProps.visible]);

    const handleConfirm = useCallback(async () => {
      if (!data || !validation.valid) {
        // Name the blockers. A bare "finish configuring each task" left a
        // new-api provider — which requires an explicit protocol per model —
        // with no way to discover what was missing.
        const detail = describeValidationErrors(validation.errors, (key, fallback) =>
          t(key, { defaultValue: fallback })
        );
        message.warning(
          detail ||
            t('settings.completeCapabilityConfiguration', {
              defaultValue: '请完成每个已选模态的协议、地址和参数配置。',
            })
        );
        return;
      }
      const capabilities = capabilityInputsFromDefinition(definition);
      if (!capabilities) {
        message.warning(
          t('settings.modelAdvanced.invalidParamsJson', { defaultValue: '供应商参数必须是 JSON 对象。' })
        );
        return;
      }

      setSaving(true);
      try {
        await ipcBridge.providerModel.save.invoke({
          provider_id: data.id,
          model: {
            model: normalizeModelId(definition.model),
            enabled: true,
            capabilities,
          },
        });
        onSubmit(data);
        modalCtrl.close();
      } catch (error) {
        console.error('provider model save failed', error);
        message.error(t('settings.saveModelConfigFailed', { defaultValue: '模型能力保存失败' }));
      } finally {
        setSaving(false);
      }
    }, [data, definition, message, modalCtrl, onSubmit, t, validation.errors, validation.valid]);

    return (
      <>
        {messageHolder}
        <NomiModal
          visible={modalProps.visible}
          onCancel={modalCtrl.close}
          unmountOnExit
          header={{ title: t('settings.addModel'), showClose: true }}
          style={{ width: 760, maxWidth: '94vw', maxHeight: '92vh' }}
          contentStyle={{
            background: 'var(--dialog-fill-0)',
            borderRadius: 16,
            padding: '20px 24px',
            overflow: 'auto',
          }}
          onOk={handleConfirm}
          confirmLoading={saving}
          okText={t('common.confirm')}
          cancelText={t('common.cancel')}
          okButtonProps={{ disabled: !validation.valid }}
        >
          <div className='pt-16px'>
            <ModelDefinitionEditor
              value={definition}
              onChange={setDefinition}
              providerBaseUrl={data?.base_url ?? ''}
              providerAuthScheme={data?.auth_scheme ?? ''}
              manifests={manifests.manifests}
              manifestLoadingTasks={manifests.loadingTasks}
              manifestErrorTasks={manifests.errorTasks}
              validationErrors={validation.errors}
              validationPending={connectionState.isLoading}
              existingModelIds={existingModelIds}
              catalogSuggestions={catalogSuggestions}
              catalogLoading={modelListState.isLoading}
              catalogError={
                modelListState.error instanceof Error
                  ? modelListState.error.message
                  : modelListState.error
                    ? String(modelListState.error)
                    : undefined
              }
              onRefreshCatalog={() => void modelListState.mutate()}
              connections={connectionState.connections}
              onCreateConnection={async (connection) => {
                if (!data) throw new Error('provider is required');
                await ipcBridge.providerConnection.save.invoke({ provider_id: data.id, connection });
                await connectionState.mutate();
              }}
            />
          </div>
        </NomiModal>
      </>
    );
  }
);

export default AddModelModal;
