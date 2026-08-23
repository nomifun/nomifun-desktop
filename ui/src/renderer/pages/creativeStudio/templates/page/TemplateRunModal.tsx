/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Input, InputNumber, Message, Modal, Select, Switch } from '@arco-design/web-react';
import { Copy, Play } from '@icon-park/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';

import {
  renderCreativePromptTemplate,
  validateTemplateInputsForDefinition,
  type CreativeTemplateDefinitionV1,
  type CreativeTemplateInputValue,
  type CreativeTemplateVariable,
} from '../domain';
import styles from './CreativeTemplateWorkspacePage.module.css';
import { draftPromptsStep, generationStep } from './templateViewModel';
import {
  formatTemplateValidationError,
  templateFallbackError,
} from '../templateI18n';

export interface CreativeTemplateRunRequest {
  template: CreativeTemplateDefinitionV1;
  inputs: CreativeTemplateInputValue[];
  referenceAssetIds: string[];
}

export interface CreativeTemplateRunnerPort {
  start(request: CreativeTemplateRunRequest): Promise<void>;
}

export interface TemplateRunModalProps {
  template: CreativeTemplateDefinitionV1 | null;
  runner?: CreativeTemplateRunnerPort;
  onClose: () => void;
  onPickAssets?: (
    variable: CreativeTemplateVariable,
    selectedAssetIds: readonly string[]
  ) => Promise<string[] | null>;
  onPickReferenceAssets?: (selectedAssetIds: readonly string[]) => Promise<string[] | null>;
  onUploadReferenceImages?: (
    files: readonly File[],
    selectedAssetIds: readonly string[]
  ) => Promise<string[]>;
}

function initialInput(variable: CreativeTemplateVariable): CreativeTemplateInputValue {
  switch (variable.type) {
    case 'text':
    case 'multiline-text':
      return {
        variableId: variable.id,
        type: variable.type,
        value: variable.defaultValue ?? '',
      };
    case 'number':
      return { variableId: variable.id, type: 'number', value: variable.defaultValue ?? 0 };
    case 'boolean':
      return { variableId: variable.id, type: 'boolean', value: variable.defaultValue };
    case 'choice':
      return {
        variableId: variable.id,
        type: 'choice',
        value: variable.defaultValue ?? variable.options[0] ?? '',
      };
    case 'image':
      return { variableId: variable.id, type: 'image', assetId: variable.defaultAssetId };
    case 'image-series':
      return {
        variableId: variable.id,
        type: 'image-series',
        assetIds: [...variable.defaultAssetIds],
      };
  }
}

function replaceInput(
  inputs: CreativeTemplateInputValue[],
  replacement: CreativeTemplateInputValue
): CreativeTemplateInputValue[] {
  return inputs.map((input) =>
    input.variableId === replacement.variableId ? replacement : input
  );
}

const TemplateInputControl: React.FC<{
  variable: CreativeTemplateVariable;
  input: CreativeTemplateInputValue;
  disabled: boolean;
  t: TFunction;
  onChange: (input: CreativeTemplateInputValue) => void;
  onPickAssets?: (
    variable: CreativeTemplateVariable,
    selectedAssetIds: readonly string[]
  ) => Promise<string[] | null>;
}> = ({ variable, input, disabled, t, onChange, onPickAssets }) => {
  if (
    (variable.type === 'text' || variable.type === 'multiline-text') &&
    (input.type === 'text' || input.type === 'multiline-text')
  ) {
    const control = {
      value: input.value,
      placeholder: variable.placeholder || variable.defaultValue || undefined,
      disabled,
      onChange: (value: string) => onChange({ ...input, value }),
    };
    return variable.type === 'multiline-text' ? (
      <Input.TextArea {...control} autoSize={{ minRows: 3, maxRows: 6 }} />
    ) : (
      <Input {...control} />
    );
  }
  if (variable.type === 'number' && input.type === 'number') {
    return (
      <InputNumber
        value={input.value}
        min={variable.minimum ?? undefined}
        max={variable.maximum ?? undefined}
        step={variable.step ?? undefined}
        disabled={disabled}
        onChange={(value) =>
          typeof value === 'number' && onChange({ ...input, value })
        }
      />
    );
  }
  if (variable.type === 'boolean' && input.type === 'boolean') {
    return (
      <Switch
        checked={input.value}
        disabled={disabled}
        onChange={(value) => onChange({ ...input, value })}
      />
    );
  }
  if (variable.type === 'choice' && input.type === 'choice') {
    return (
      <Select
        value={input.value || undefined}
        placeholder={
          variable.options.length > 0
            ? t('creativeStudio.templates.runModal.choicePlaceholder', {
                defaultValue: 'Select an option',
              })
            : t('creativeStudio.templates.runModal.choiceEmpty', {
                defaultValue: 'No options configured',
              })
        }
        options={variable.options.map((option) => ({ value: option, label: option }))}
        disabled={disabled || variable.options.length === 0}
        onChange={(value) => onChange({ ...input, value })}
      />
    );
  }
  if (variable.type === 'image' && input.type === 'image') {
    return (
      <div className={styles.referencePlaceholder}>
        <p>
          {input.assetId
            ? t('creativeStudio.templates.runModal.selectedAsset', {
                assetId: input.assetId,
                defaultValue: 'Selected asset {{assetId}}',
              })
            : t('creativeStudio.templates.runModal.noReference', {
                defaultValue: 'No reference image selected',
              })}
        </p>
        <Button
          size='small'
          disabled={disabled || !onPickAssets}
          title={
            onPickAssets
              ? undefined
              : t('creativeStudio.templates.runModal.assetPickerUnavailable', {
                  defaultValue: 'Asset picker is not connected',
                })
          }
          onClick={() =>
            void onPickAssets?.(variable, input.assetId ? [input.assetId] : [])
              .then((assetIds) => {
                if (assetIds) onChange({ ...input, assetId: assetIds[0] ?? null });
              })
              .catch((error) => Message.error(
                templateFallbackError(
                  error,
                  t,
                  'creativeStudio.templates.runModal.assetPickerOpenFailed',
                  'Failed to open asset picker'
                )
              ))
          }
        >
          {t('creativeStudio.templates.runModal.selectFromAssets', {
            defaultValue: 'Select from My assets',
          })}
        </Button>
      </div>
    );
  }
  if (variable.type === 'image-series' && input.type === 'image-series') {
    return (
      <div className={styles.referencePlaceholder}>
        <p>
          {input.assetIds.length > 0
            ? t('creativeStudio.templates.runModal.selectedImages', {
                count: input.assetIds.length,
                defaultValue: '{{count}} images selected',
              })
            : t('creativeStudio.templates.runModal.noReference', {
                defaultValue: 'No reference image selected',
              })}
        </p>
        <Button
          size='small'
          disabled={disabled || !onPickAssets}
          title={
            onPickAssets
              ? undefined
              : t('creativeStudio.templates.runModal.assetPickerUnavailable', {
                  defaultValue: 'Asset picker is not connected',
                })
          }
          onClick={() =>
            void onPickAssets?.(variable, input.assetIds)
              .then((assetIds) => {
                if (assetIds) onChange({ ...input, assetIds });
              })
              .catch((error) => Message.error(
                templateFallbackError(
                  error,
                  t,
                  'creativeStudio.templates.runModal.assetPickerOpenFailed',
                  'Failed to open asset picker'
                )
              ))
          }
        >
          {t('creativeStudio.templates.runModal.selectFromAssets', {
            defaultValue: 'Select from My assets',
          })}
        </Button>
      </div>
    );
  }
  return (
    <div className={styles.referencePlaceholder}>
      {t('creativeStudio.templates.runModal.contractMismatch', {
        defaultValue: 'Variable contract mismatch. Reopen the template.',
      })}
    </div>
  );
};

const TemplateRunModal: React.FC<TemplateRunModalProps> = ({
  template,
  runner,
  onClose,
  onPickAssets,
  onPickReferenceAssets,
  onUploadReferenceImages,
}) => {
  const { t } = useTranslation();
  const [inputs, setInputs] = useState<CreativeTemplateInputValue[]>([]);
  const [referenceAssetIds, setReferenceAssetIds] = useState<string[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const referenceInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setInputs(template?.variables.map(initialInput) ?? []);
    setReferenceAssetIds([]);
    setSubmitting(false);
  }, [template]);

  const validation = useMemo(
    () =>
      template
        ? validateTemplateInputsForDefinition(template, inputs)
        : ({
              ok: false,
              error: {
                code: 'invalid-value',
                path: '$.template',
                message: 'template is unavailable',
            },
          } as const),
    [inputs, template]
  );
  const prompt = useMemo(() => {
    if (!template) return { ok: false as const, value: '' };
    const promptTemplate = template.templates[0];
    if (!promptTemplate) return { ok: false as const, value: '' };
    const result = renderCreativePromptTemplate(
      template,
      promptTemplate.id,
      inputs
    );
    return result.ok
      ? { ok: true as const, value: result.value }
      : { ok: false as const, value: result.error.message };
  }, [inputs, template]);
  const promptPreview = useMemo(() => {
    if (!template) return '';
    const values = new Map(inputs.map((input) => [input.variableId, input]));
    const variables = new Map(template.variables.map((variable) => [variable.id, variable]));
    return (template.templates[0]?.segments ?? [])
      .map((segment) => {
        if (segment.kind === 'text') return segment.text;
        const variable = variables.get(segment.variableId);
        const input = values.get(segment.variableId);
        if (
          input &&
          (input.type === 'text' || input.type === 'multiline-text' || input.type === 'choice') &&
          input.value.trim()
        ) {
          return input.value;
        }
        if (input?.type === 'number') return String(input.value);
        if (input?.type === 'boolean') return input.value ? 'true' : 'false';
        return variable ? `{{${variable.key}}}` : '{{missing_variable}}';
      })
      .join('');
  }, [inputs, template]);

  if (!template) return null;
  const generate = generationStep(template);
  const model = generate.generation.model;
  const planningModel = draftPromptsStep(template)?.planning.model ?? null;
  const requiresPlanningModel = template.output.kind === 'multi-image-series';
  const canSubmit = validation.ok
    && prompt.ok
    && model !== null
    && (!requiresPlanningModel || planningModel !== null)
    && runner !== undefined;

  const submit = async () => {
    if (!canSubmit || !runner) return;
    setSubmitting(true);
    try {
      await runner.start({
        template,
        inputs,
        referenceAssetIds,
      });
      Message.success(
        t('creativeStudio.templates.runModal.submitted', {
          defaultValue: 'Template run submitted',
        })
      );
      onClose();
    } catch (error) {
      Message.error(
        templateFallbackError(
          error,
          t,
          'creativeStudio.templates.runModal.submitFailed',
          'Failed to submit template run'
        )
      );
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Modal
      visible
      alignCenter={false}
      className={styles.runModal}
      title={
        template.metadata.name ||
        t('creativeStudio.templates.runModal.titleFallback', {
          defaultValue: 'Run template',
        })
      }
      footer={null}
      autoFocus={false}
      unmountOnExit
      getPopupContainer={() =>
        document.getElementById('creative-studio-portal-root') ?? document.body
      }
      onCancel={onClose}
    >
      <div className={styles.runGrid} data-template-runner>
        <div className={styles.runColumn}>
          <section className={styles.runSection}>
            <h3>
              {t('creativeStudio.templates.runModal.inputs', {
                defaultValue: 'Variable inputs',
              })}
            </h3>
            <div className={styles.inputList}>
              {template.variables.map((variable) => {
                const input = inputs.find((candidate) => candidate.variableId === variable.id);
                if (!input) return null;
                return (
                  <label key={variable.id} className={styles.runField}>
                    <span>
                      {variable.label || variable.key}
                      {variable.required ? <span className={styles.required}>*</span> : null}
                    </span>
                    <TemplateInputControl
                      variable={variable}
                      input={input}
                      disabled={submitting}
                      t={t}
                      onChange={(replacement) =>
                        setInputs((current) => replaceInput(current, replacement))
                      }
                      onPickAssets={onPickAssets}
                    />
                  </label>
                );
              })}
            </div>
          </section>

          <section className={styles.runSection}>
            <div className={styles.sectionHeadingRow}>
              <h3>
                {t('creativeStudio.templates.runModal.references', {
                  defaultValue: 'Reference images',
                })}
              </h3>
              <div className={styles.referenceActions}>
                <Button
                  size='small'
                  disabled={submitting || !onPickReferenceAssets}
                  title={
                    onPickReferenceAssets
                      ? undefined
                      : t('creativeStudio.templates.runModal.assetPickerUnavailable', {
                          defaultValue: 'Asset picker is not connected',
                        })
                  }
                  onClick={() =>
                    void onPickReferenceAssets?.(referenceAssetIds)
                      .then((assetIds) => {
                        if (assetIds) setReferenceAssetIds(assetIds);
                      })
                      .catch((error) => Message.error(
                        templateFallbackError(
                          error,
                          t,
                          'creativeStudio.templates.runModal.assetPickerOpenFailed',
                          'Failed to open asset picker'
                        )
                      ))
                  }
                >
                  {t('creativeStudio.templates.runModal.myAssets', {
                    defaultValue: 'My assets',
                  })}
                </Button>
                <Button
                  size='small'
                  disabled={submitting || !onUploadReferenceImages}
                  title={
                    onUploadReferenceImages
                      ? undefined
                      : t('creativeStudio.templates.runModal.uploadGatewayUnavailable', {
                          defaultValue: 'Image upload gateway is not connected',
                        })
                  }
                  onClick={() => referenceInputRef.current?.click()}
                >
                  {t('creativeStudio.templates.runModal.upload', {
                    defaultValue: 'Upload',
                  })}
                </Button>
              </div>
            </div>
            <input
              ref={referenceInputRef}
              hidden
              type='file'
              accept='image/*'
              multiple
              onChange={(event) => {
                const files = [...(event.currentTarget.files ?? [])];
                event.currentTarget.value = '';
                if (files.length === 0 || !onUploadReferenceImages) return;
                void onUploadReferenceImages(files, referenceAssetIds)
                  .then(setReferenceAssetIds)
                  .catch((error) => Message.error(
                    templateFallbackError(
                      error,
                      t,
                      'creativeStudio.templates.runModal.uploadFailed',
                      'Failed to upload reference images'
                    )
                  ));
              }}
            />
            <div className={styles.referencePlaceholder}>
              {referenceAssetIds.length > 0
                ? t('creativeStudio.templates.runModal.addedReferences', {
                    count: referenceAssetIds.length,
                    defaultValue: '{{count}} reference images added',
                  })
                : t('creativeStudio.templates.runModal.noAddedReferences', {
                    defaultValue: 'No reference images added',
                  })}
            </div>
          </section>

          {!runner ? (
            <div className={styles.runnerUnavailable} role='status'>
              {t('creativeStudio.templates.runModal.gatewayUnavailable', {
                defaultValue:
                  'The run gateway is connecting to the NomiFun task system. No generated results are simulated yet.',
              })}
            </div>
          ) : !model ? (
            <div className={styles.runnerUnavailable} role='status'>
              {t('creativeStudio.templates.runModal.modelRequired', {
                defaultValue:
                  'Edit the template and select an enabled model that supports this task first.',
              })}
            </div>
          ) : requiresPlanningModel && !planningModel ? (
            <div className={styles.runnerUnavailable} role='status'>
              {t('creativeStudio.templates.runModal.planningModelRequired', {
                defaultValue:
                  'Select an enabled chat model for multi-image prompt planning first.',
              })}
            </div>
          ) : !validation.ok ? (
            <div className={styles.runnerUnavailable} role='status'>
              {formatTemplateValidationError(validation.error, t)}
            </div>
          ) : null}

          <Button
            long
            size='large'
            type='primary'
            loading={submitting}
            disabled={!canSubmit}
            icon={<Play theme='outline' size={16} fill='currentColor' />}
            onClick={() => void submit()}
          >
            {template.output.kind === 'multi-image-series'
              ? t('creativeStudio.templates.runModal.generatePrompts', {
                  defaultValue: 'Generate prompts',
                })
              : t('creativeStudio.templates.runModal.startTask', {
                  defaultValue: 'Start task',
                })}
          </Button>
        </div>

        <div className={styles.runColumn}>
          <section className={styles.runSection}>
            <div className={styles.sectionHeadingRow}>
              <h3>
                {t('creativeStudio.templates.runModal.promptPreview', {
                  defaultValue: 'Generated prompt preview',
                })}
              </h3>
              <Button
                size='small'
                icon={<Copy theme='outline' size={14} fill='currentColor' />}
                disabled={!promptPreview}
                onClick={() => promptPreview && void navigator.clipboard.writeText(promptPreview)}
              >
                {t('creativeStudio.templates.runModal.copy', { defaultValue: 'Copy' })}
              </Button>
            </div>
            <div className={styles.promptResult}>
              {promptPreview ||
                t('creativeStudio.templates.runModal.promptPlaceholder', {
                  defaultValue: 'Fill in the variables to preview the final prompt here',
                })}
            </div>
          </section>

          <div className={styles.infoGrid}>
            <div className={styles.infoPill}>
              <p>{t('creativeStudio.templates.runModal.model', { defaultValue: 'Model' })}</p>
              <strong>
                {model?.model ??
                  t('creativeStudio.templates.runModal.notSelected', {
                    defaultValue: 'Not selected',
                  })}
              </strong>
            </div>
            <div className={styles.infoPill}>
              <p>{t('creativeStudio.templates.runModal.task', { defaultValue: 'Task' })}</p>
              <strong>
                {model?.task === 'image_edit'
                  ? t('creativeStudio.templates.runModal.imageEdit', {
                      defaultValue: 'Image editing',
                    })
                  : t('creativeStudio.templates.runModal.imageGeneration', {
                      defaultValue: 'Image generation',
                    })}
              </strong>
            </div>
            <div className={styles.infoPill}>
              <p>{t('creativeStudio.templates.runModal.size', { defaultValue: 'Size' })}</p>
              <strong>
                {generate.generation.width} × {generate.generation.height}
              </strong>
            </div>
            <div className={styles.infoPill}>
              <p>
                {template.output.kind === 'multi-image-series'
                  ? t('creativeStudio.templates.runModal.draftCount', {
                      defaultValue: 'Drafts',
                    })
                  : t('creativeStudio.templates.runModal.count', {
                      defaultValue: 'Quantity',
                    })}
              </p>
              <strong>
                {template.output.kind === 'multi-image-series'
                  ? template.output.targetCount
                  : generate.generation.imagesPerPrompt}{' '}
                {t('creativeStudio.templates.runModal.imagesUnit', {
                  defaultValue: 'images',
                })}
              </strong>
            </div>
          </div>
        </div>
      </div>
    </Modal>
  );
};

export default TemplateRunModal;
