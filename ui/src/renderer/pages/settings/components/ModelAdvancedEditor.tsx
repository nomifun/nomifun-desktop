/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Input, Tooltip } from '@arco-design/web-react';
import { SettingTwo } from '@icon-park/react';
import type { ProviderId } from '@/common/types/ids';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ProviderModelCapabilityResponse } from '@/common/types/provider/providerModel';
import { ipcBridge } from '@/common';
import NomiModal from '@/renderer/components/base/NomiModal';
import ModelDefinitionEditor, { type ModelDefinitionEditorHandle } from './ModelDefinitionEditor';
import {
  capabilityDraftFromResponse,
  capabilityInputFromResponse,
  capabilityInputsFromDefinition,
  validateModelDefinition,
  type ModelDefinitionDraft,
  type ProviderModelCapabilityInput,
} from './providerModelAdvanced';
import useModelProtocolManifests from './useModelProtocolManifests';
import { useProviderConnections } from './useProviderConnections';
import ModelCallConfigModalFooter from './ModelCallConfigModalFooter';
import { mergeTaskCapabilityEdit, resolveScopedModelTextEdit } from './modelTaskScopedEdit';

export interface ModelAdvancedPatch {
  display_name: string | null;
  capabilities: ProviderModelCapabilityInput[];
  description?: string | null;
}

export interface ModelAdvancedEditorProps {
  providerId: ProviderId;
  providerName: string;
  preset: string;
  providerBaseUrl: string;
  providerAuthScheme: string;
  model: string;
  displayName?: string;
  /** Supplying this field enables the shared model description on the home form. */
  description?: string | null;
  capabilities: ProviderModelCapabilityResponse[];
  onSave: (patch: ModelAdvancedPatch) => Promise<void>;
  openRequest?: string;
  onOpenRequestHandled?: (request: string) => void;
  /** Restrict a scenario entry to its existing task without changing other tasks. */
  task?: ModelTask;
  hideTrigger?: boolean;
  onClose?: () => void;
}

/** Existing-model editor backed by the same capability form as both add flows. */
const ModelAdvancedEditor: React.FC<ModelAdvancedEditorProps> = ({
  providerId,
  providerName,
  preset,
  providerBaseUrl,
  providerAuthScheme,
  model,
  displayName,
  description,
  capabilities,
  onSave,
  openRequest,
  onOpenRequestHandled,
  task,
  hideTrigger = false,
  onClose,
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [scopeError, setScopeError] = useState<'unavailable' | 'changed' | 'metadata_changed'>();
  const [descriptionDraft, setDescriptionDraft] = useState(description ?? '');
  const [focusedCallConfigTask, setFocusedCallConfigTask] = useState<ModelTask>();
  const modelEditorRef = useRef<ModelDefinitionEditorHandle>(null);
  const handledOpenRequestRef = useRef<string | undefined>(undefined);
  const taskBaselineRef = useRef<ProviderModelCapabilityInput | undefined>(undefined);
  const sharedFieldsBaselineRef = useRef({ displayName, description });
  const [definition, setDefinition] = useState<ModelDefinitionDraft>(() => ({
    model,
    displayName,
    capabilities: capabilities.map(capabilityDraftFromResponse),
  }));
  const selectedTasks = useMemo(
    () => task ? [task] : definition.capabilities.map((capability) => capability.task),
    [definition.capabilities, task]
  );
  const manifests = useModelProtocolManifests({
    preset,
    tasks: selectedTasks,
    baseUrlHint: providerBaseUrl,
  });
  const connectionState = useProviderConnections(providerId, open);
  const validation = useMemo(
    () =>
      validateModelDefinition(
        task
          ? { ...definition, capabilities: definition.capabilities.filter((capability) => capability.task === task) }
          : definition,
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
      task,
    ]
  );

  const resetDraft = useCallback(() => {
    setDefinition({ model, displayName, capabilities: capabilities.map(capabilityDraftFromResponse) });
  }, [capabilities, displayName, model]);

  const handleOpen = useCallback(() => {
    resetDraft();
    const capability = task ? capabilities.find((candidate) => candidate.task === task) : undefined;
    taskBaselineRef.current = capability ? capabilityInputFromResponse(capability) : undefined;
    sharedFieldsBaselineRef.current = { displayName, description };
    setDescriptionDraft(description ?? '');
    setScopeError(undefined);
    setFocusedCallConfigTask(undefined);
    setOpen(true);
  }, [capabilities, description, displayName, resetDraft, task]);

  const handleClose = () => {
    if (saving) return;
    setOpen(false);
    onClose?.();
  };

  useEffect(() => {
    if (!openRequest || handledOpenRequestRef.current === openRequest) return;
    handledOpenRequestRef.current = openRequest;
    handleOpen();
    onOpenRequestHandled?.(openRequest);
  }, [handleOpen, onOpenRequestHandled, openRequest]);

  const handleSave = async () => {
    if (!validation.valid) return;
    const scopedSave = task
      ? mergeTaskCapabilityEdit(definition, capabilities, task, taskBaselineRef.current)
      : undefined;
    if (scopedSave?.error) {
      if (scopedSave.error !== 'invalid') setScopeError(scopedSave.error);
      return;
    }
    const nextCapabilities = scopedSave?.capabilities ?? capabilityInputsFromDefinition(definition);
    if (!nextCapabilities) return;
    const nextDisplayName = task
      ? resolveScopedModelTextEdit(definition.displayName, sharedFieldsBaselineRef.current.displayName, displayName)
      : { value: definition.displayName?.trim() || null, conflict: false };
    const nextDescription = task
      ? resolveScopedModelTextEdit(descriptionDraft, sharedFieldsBaselineRef.current.description, description)
      : { value: descriptionDraft.trim() || null, conflict: false };
    if (nextDisplayName.conflict || (description !== undefined && nextDescription.conflict)) {
      setScopeError('metadata_changed');
      return;
    }
    setSaving(true);
    try {
      await onSave({
        display_name: nextDisplayName.value ?? null,
        capabilities: nextCapabilities,
        ...(description === undefined ? {} : { description: nextDescription.value ?? null }),
      });
      setOpen(false);
      onClose?.();
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
        onCancel={handleClose}
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
            : task
              ? `${t(`settings.modelTask.${task}`, { defaultValue: task })} · ${t('settings.modelAdvanced.titleForTask', { defaultValue: '编辑模型' })}`
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
              onClick={handleClose}
            >
              {t('common.cancel')}
            </Button>
            <Button
              type='primary'
              loading={saving}
              disabled={!validation.valid || Boolean(scopeError)}
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
          {scopeError && (
            <div role='alert' className='text-12px text-danger-6 mb-12px'>
              {scopeError === 'metadata_changed'
                ? t('settings.modelAdvanced.sharedFieldsChanged', {
                    defaultValue: '模型别名或描述已被更新，请关闭后重新打开再编辑。',
                  })
                : scopeError === 'changed'
                  ? t('settings.modelAdvanced.scopedTaskChanged', {
                      defaultValue: '此用途配置已被更新，请关闭后重新打开再编辑。',
                    })
                  : t('settings.modelAdvanced.scopedTaskUnavailable', {
                      defaultValue: '当前用途配置已变化，请关闭后重新打开。',
                    })}
            </div>
          )}
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
            capabilityTask={task}
            connections={connectionState.connections}
            onCreateConnection={async (connection) => {
              await ipcBridge.providerConnection.save.invoke({ provider_id: providerId, connection });
              await connectionState.mutate();
            }}
            onCallConfigFocusChange={setFocusedCallConfigTask}
            callConfigFooterPlacement='modal'
          />
          {description !== undefined && !focusedCallConfigTask && (
            <div className='mt-12px space-y-6px' data-model-description-editor>
              <label htmlFor={`model-description-${providerId}`} className='block text-12px text-t-secondary'>
                {t('settings.modelDescriptionTitle')}
              </label>
              <Input.TextArea
                id={`model-description-${providerId}`}
                value={descriptionDraft}
                rows={3}
                placeholder={t('settings.modelDescriptionPlaceholder')}
                onChange={setDescriptionDraft}
                disabled={saving}
              />
              <div className='text-11px text-t-tertiary'>
                {t('settings.modelAdvanced.descriptionSharedHint', {
                  defaultValue: '模型描述在各调用用途中共用。',
                })}
              </div>
            </div>
          )}
        </div>
      </NomiModal>
      {!hideTrigger && (
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
      )}
    </>
  );
};

export default ModelAdvancedEditor;
