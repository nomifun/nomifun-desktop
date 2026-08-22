/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Tooltip } from '@arco-design/web-react';
import { SettingTwo } from '@icon-park/react';
import type { ProviderId } from '@/common/types/ids';
import type { ProviderModelCapabilityResponse } from '@/common/types/provider/providerModel';
import { ipcBridge } from '@/common';
import NomiModal from '@/renderer/components/base/NomiModal';
import ModelDefinitionEditor from './ModelDefinitionEditor';
import {
  capabilityDraftFromResponse,
  capabilityInputsFromDefinition,
  validateModelDefinition,
  type ModelDefinitionDraft,
  type ProviderModelCapabilityInput,
} from './providerModelAdvanced';
import useModelProtocolManifests from './useModelProtocolManifests';
import { useProviderConnections } from './useProviderConnections';

export interface ModelAdvancedPatch {
  capabilities: ProviderModelCapabilityInput[];
}

/** Existing-model editor backed by the same full capability form as both add flows. */
const ModelAdvancedEditor: React.FC<{
  providerId: ProviderId;
  preset: string;
  providerBaseUrl: string;
  providerAuthScheme: string;
  model: string;
  capabilities: ProviderModelCapabilityResponse[];
  onSave: (patch: ModelAdvancedPatch) => Promise<void>;
}> = ({ providerId, preset, providerBaseUrl, providerAuthScheme, model, capabilities, onSave }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [definition, setDefinition] = useState<ModelDefinitionDraft>(() => ({
    model,
    capabilities: capabilities.map(capabilityDraftFromResponse),
  }));
  const selectedTasks = useMemo(
    () => definition.capabilities.map((capability) => capability.task),
    [definition.capabilities]
  );
  const manifests = useModelProtocolManifests({
    preset,
    tasks: selectedTasks,
    baseUrlHint: providerBaseUrl,
    modelHint: definition.model,
  });
  const connectionState = useProviderConnections(providerId, open);
  const validation = useMemo(
    () =>
      validateModelDefinition(
        definition,
        manifests.manifests,
        providerBaseUrl,
        [],
        manifests.loadingTasks,
        connectionState.connections.map((connection) => connection.role),
        providerAuthScheme,
        Object.fromEntries(
          connectionState.connections.map((connection) => [connection.role, connection.auth_scheme])
        ),
        connectionState.connections
      ),
    [
      connectionState.connections,
      definition,
      manifests.loadingTasks,
      manifests.manifests,
      providerBaseUrl,
      providerAuthScheme,
    ]
  );

  const resetDraft = () => {
    setDefinition({ model, capabilities: capabilities.map(capabilityDraftFromResponse) });
  };

  const handleOpen = () => {
    resetDraft();
    setOpen(true);
  };

  const handleSave = async () => {
    const nextCapabilities = capabilityInputsFromDefinition(definition);
    if (!validation.valid || !nextCapabilities) return;
    setSaving(true);
    try {
      await onSave({ capabilities: nextCapabilities });
      setOpen(false);
    } catch {
      // The parent owns the persistence toast. Keep the editor open for retry.
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <NomiModal
        visible={open}
        onCancel={() => {
          if (!saving) setOpen(false);
        }}
        unmountOnExit
        maskClosable={!saving}
        escToExit={!saving}
        header={{ title: t('settings.editModelCapabilities'), showClose: true }}
        style={{ width: 760, maxWidth: '94vw', maxHeight: '92vh' }}
        contentStyle={{
          background: 'var(--dialog-fill-0)',
          borderRadius: 16,
          padding: '20px 24px',
          overflow: 'auto',
          maxHeight: 'calc(92vh - 160px)',
        }}
        footer={
          <div className='flex justify-end gap-10px mt-10px'>
            <Button
              disabled={saving}
              className='px-20px min-w-80px'
              style={{ borderRadius: 8 }}
              onClick={() => setOpen(false)}
            >
              {t('common.cancel')}
            </Button>
            <Button
              type='primary'
              loading={saving}
              disabled={!validation.valid}
              className='px-20px min-w-80px'
              style={{ borderRadius: 8 }}
              onClick={() => void handleSave()}
            >
              {t('common.save')}
            </Button>
          </div>
        }
      >
        <div className='pt-16px'>
          <ModelDefinitionEditor
            value={definition}
            onChange={setDefinition}
            providerBaseUrl={providerBaseUrl}
            providerAuthScheme={providerAuthScheme}
            manifests={manifests.manifests}
            manifestLoadingTasks={manifests.loadingTasks}
            manifestErrorTasks={manifests.errorTasks}
            validationErrors={validation.errors}
            validationPending={connectionState.isLoading}
            modelReadOnly
            connections={connectionState.connections}
            onCreateConnection={async (connection) => {
              await ipcBridge.providerConnection.save.invoke({ provider_id: providerId, connection });
              await connectionState.mutate();
            }}
          />
        </div>
      </NomiModal>
      <Tooltip content={t('settings.editModelCapabilities', { defaultValue: '编辑模态、协议与地址' })}>
        <Button
          size='mini'
          className='model-provider-action-btn !w-24px !h-24px !min-w-24px shrink-0 text-t-secondary hover:text-t-primary'
          icon={<SettingTwo theme='outline' size='14' />}
          onClick={handleOpen}
        />
      </Tooltip>
    </>
  );
};

export default ModelAdvancedEditor;
