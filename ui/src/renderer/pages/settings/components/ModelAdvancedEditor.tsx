/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Tooltip } from '@arco-design/web-react';
import { SettingTwo } from '@icon-park/react';
import type { ProviderId } from '@/common/types/ids';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ProviderModelCapabilityResponse } from '@/common/types/provider/providerModel';
import { ipcBridge } from '@/common';
import NomiModal from '@/renderer/components/base/NomiModal';
import ModelDefinitionEditor, { type ModelDefinitionEditorHandle } from './ModelDefinitionEditor';
import {
  capabilityDraftFromResponse,
  capabilityInputsFromDefinition,
  validateModelDefinition,
  type ModelDefinitionDraft,
  type ProviderModelCapabilityInput,
} from './providerModelAdvanced';
import useModelProtocolManifests from './useModelProtocolManifests';
import { useProviderConnections } from './useProviderConnections';
import ModelCallConfigModalFooter from './ModelCallConfigModalFooter';

export interface ModelAdvancedPatch {
  capabilities: ProviderModelCapabilityInput[];
}

/** Existing-model editor backed by the same full capability form as both add flows. */
const ModelAdvancedEditor: React.FC<{
  providerId: ProviderId;
  providerName: string;
  preset: string;
  providerBaseUrl: string;
  providerAuthScheme: string;
  model: string;
  capabilities: ProviderModelCapabilityResponse[];
  onSave: (patch: ModelAdvancedPatch) => Promise<void>;
}> = ({ providerId, providerName, preset, providerBaseUrl, providerAuthScheme, model, capabilities, onSave }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [focusedCallConfigTask, setFocusedCallConfigTask] = useState<ModelTask>();
  const modelEditorRef = useRef<ModelDefinitionEditorHandle>(null);
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
    setFocusedCallConfigTask(undefined);
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
        header={{
          title: focusedCallConfigTask
            ? `${t(`settings.modelTask.${focusedCallConfigTask}`, {
                defaultValue: focusedCallConfigTask,
              })} · ${t('settings.modelAdvanced.callConfigurationTitle', {
                defaultValue: '调用配置',
              })}`
            : t('settings.editModelCapabilities'),
          showClose: true,
        }}
        style={{
          width: focusedCallConfigTask ? 840 : 760,
          maxWidth: '94vw',
          maxHeight: focusedCallConfigTask ? '96vh' : '92vh',
        }}
        contentStyle={{
          background: 'var(--dialog-fill-0)',
          borderRadius: 16,
          padding: '20px 24px',
          overflow: 'auto',
          maxHeight: focusedCallConfigTask
            ? 'calc(96vh - 72px)'
            : 'calc(92vh - 160px)',
        }}
        footer={focusedCallConfigTask ? (
          <ModelCallConfigModalFooter
            task={focusedCallConfigTask}
            onCancel={() => modelEditorRef.current?.cancelCallConfig()}
            onApply={() => modelEditorRef.current?.applyCallConfig()}
          />
        ) :
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
            ref={modelEditorRef}
            value={definition}
            onChange={setDefinition}
            providerBaseUrl={providerBaseUrl}
            providerAuthScheme={providerAuthScheme}
            providerLabel={providerName}
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
            onCallConfigFocusChange={setFocusedCallConfigTask}
            callConfigFooterPlacement='modal'
          />
        </div>
      </NomiModal>
      <Tooltip content={t('settings.editModelCapabilities', { defaultValue: '编辑模态、协议与地址' })}>
        <Button
          size='mini'
          className='model-provider-action-btn !w-24px !h-24px !min-w-24px shrink-0 text-t-secondary hover:text-t-primary'
          icon={<SettingTwo theme='outline' size='14' />}
          aria-label={t('settings.editModelCapabilities', {
            defaultValue: '编辑模型调用配置',
          })}
          onClick={handleOpen}
        />
      </Tooltip>
    </>
  );
};

export default ModelAdvancedEditor;
