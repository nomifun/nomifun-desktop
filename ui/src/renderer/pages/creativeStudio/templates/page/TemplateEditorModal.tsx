/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Button,
  Checkbox,
  Input,
  Modal,
  Switch,
} from '@arco-design/web-react';
import { AlignTextLeft, CheckOne, Delete, Down, HashtagKey, List, Pic, Plus, Text } from '@icon-park/react';
import React, { useEffect, useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';

import NomiInput from '@renderer/components/base/NomiInput';
import NomiInputNumber from '@renderer/components/base/NomiInputNumber';
import NomiSelect from '@renderer/components/base/NomiSelect';

import { parseProviderId } from '@/common/types/ids';
import { OutputLimitInput } from '@/renderer/pages/settings/components/OutputLimitInput';
import NomiCreativeModelSelect from '../../models/NomiCreativeModelSelect';
import type { CreativeModelCatalogSnapshot } from '../../models';
import {
  imageGenerationFixedSizeOptions,
  imageGenerationSizePolicyForModel,
} from '@renderer/creation/parameters/image';
import ImageSizePicker from '@renderer/creation/parameters/ImageSizePicker';
import { exactGenerationModelOptions } from '@renderer/creation/modelSelection';
import {
  cloneTemplateDefinition,
  type CreativeTemplateDefinitionV1,
  type CreativeTemplateImageTask,
  type CreativeTemplateImageGenerationSettings,
  type CreativeTemplatePromptPlanningSettings,
  type CreativeTemplateVariable,
} from '../domain';
import styles from './TemplateWorkspaceDialogs.module.css';
import {
  convertTemplateVariable,
  createTemplateVariable,
  draftPromptsStep,
  generationStep,
  removeTemplateVariable,
  replaceCreativePromptTemplateText,
  replaceTemplateVariable,
  setTemplateReferenceVariable,
  switchTemplateMode,
  templateMode,
  creativePromptTemplateText,
  type TemplateEditorMode,
  type TemplateVariableType,
} from './templateViewModel';
import { createTemplateTranslationCopy } from '../templateI18n';

const variableTypeIcons = {
  text: Text,
  'multiline-text': AlignTextLeft,
  number: HashtagKey,
  boolean: CheckOne,
  choice: List,
  image: Pic,
  'image-series': Pic,
};

const buildVariableTypeOptions = (t: TFunction): Array<{
  value: TemplateVariableType;
  label: string;
}> => [
  {
    value: 'text',
    label: t('creativeStudio.templates.editor.variableType.text', {
      defaultValue: 'Single-line text',
    }),
  },
  {
    value: 'multiline-text',
    label: t('creativeStudio.templates.editor.variableType.multilineText', {
      defaultValue: 'Long text',
    }),
  },
  {
    value: 'number',
    label: t('creativeStudio.templates.editor.variableType.number', {
      defaultValue: 'Number',
    }),
  },
  {
    value: 'boolean',
    label: t('creativeStudio.templates.editor.variableType.boolean', {
      defaultValue: 'Toggle',
    }),
  },
  {
    value: 'choice',
    label: t('creativeStudio.templates.editor.variableType.choice', {
      defaultValue: 'Select',
    }),
  },
  {
    value: 'image',
    label: t('creativeStudio.templates.editor.variableType.image', {
      defaultValue: 'Reference image',
    }),
  },
  {
    value: 'image-series',
    label: t('creativeStudio.templates.editor.variableType.imageSeries', {
      defaultValue: 'Reference image set',
    }),
  },
];

const buildQualityOptions = (t: TFunction) => [
  {
    value: 'auto',
    label: t('creativeStudio.templates.editor.quality.auto', {
      defaultValue: 'Automatic',
    }),
  },
  {
    value: 'high',
    label: t('creativeStudio.templates.editor.quality.high', {
      defaultValue: 'High',
    }),
  },
  {
    value: 'medium',
    label: t('creativeStudio.templates.editor.quality.medium', {
      defaultValue: 'Medium',
    }),
  },
  {
    value: 'low',
    label: t('creativeStudio.templates.editor.quality.low', {
      defaultValue: 'Low',
    }),
  },
];

export interface TemplateEditorModalProps {
  template: CreativeTemplateDefinitionV1 | null;
  isNew: boolean;
  saving: boolean;
  onChange: (template: CreativeTemplateDefinitionV1) => void;
  onCancel: () => void;
  onSave: () => void;
  onOpenModelSettings?: () => void;
  modelCatalog?: CreativeModelCatalogSnapshot;
}

function patchGeneration(
  template: CreativeTemplateDefinitionV1,
  patch: Partial<CreativeTemplateImageGenerationSettings>
): CreativeTemplateDefinitionV1 {
  const next = cloneTemplateDefinition(template);
  const step = generationStep(next);
  step.generation = {
    ...step.generation,
    ...patch,
    model:
      patch.model === undefined
        ? step.generation.model
        : patch.model
          ? { ...patch.model }
          : null,
  };
  return next;
}

function patchPromptPlanning(
  template: CreativeTemplateDefinitionV1,
  patch: Partial<CreativeTemplatePromptPlanningSettings>
): CreativeTemplateDefinitionV1 {
  const next = cloneTemplateDefinition(template);
  const step = draftPromptsStep(next);
  if (!step) return next;
  step.planning = {
    ...step.planning,
    ...patch,
    model:
      patch.model === undefined
        ? step.planning.model
        : patch.model
          ? { ...patch.model }
          : null,
  };
  return next;
}

const patchVariable = (
  template: CreativeTemplateDefinitionV1,
  variable: CreativeTemplateVariable,
  patch: Partial<CreativeTemplateVariable>
) => replaceTemplateVariable(template, { ...variable, ...patch } as CreativeTemplateVariable);

const VariableValueEditor: React.FC<{
  template: CreativeTemplateDefinitionV1;
  variable: CreativeTemplateVariable;
  t: TFunction;
  onChange: (template: CreativeTemplateDefinitionV1) => void;
}> = ({ template, variable, t, onChange }) => {
  if (variable.type === 'text' || variable.type === 'multiline-text') {
    const defaultLabel = variable.type === 'multiline-text'
      ? t('creativeStudio.templates.editor.default.multiline', {
          defaultValue: 'Default long text (optional)',
        })
      : t('creativeStudio.templates.editor.default.value', {
          defaultValue: 'Default value (optional)',
        });
    const controlProps = {
      value: variable.defaultValue ?? '',
      placeholder: defaultLabel,
      'aria-label': defaultLabel,
      onChange: (value: string) =>
        onChange(
          patchVariable(template, variable, {
            defaultValue: value || null,
          })
        ),
    };
    return variable.type === 'multiline-text' ? (
      <Input.TextArea {...controlProps} autoSize={{ minRows: 2, maxRows: 4 }} />
    ) : (
      <NomiInput {...controlProps} />
    );
  }
  if (variable.type === 'number') {
    return (
      <div className={styles.variableValueGrid}>
        <NomiInputNumber
          value={variable.defaultValue ?? undefined}
          placeholder={t('creativeStudio.templates.editor.default.number', {
            defaultValue: 'Default number',
          })}
          onChange={(value) =>
            onChange(
              patchVariable(template, variable, {
                defaultValue: typeof value === 'number' ? value : null,
              })
            )
          }
        />
        <NomiInputNumber
          value={variable.minimum ?? undefined}
          placeholder={t('creativeStudio.templates.editor.minimum', {
            defaultValue: 'Minimum',
          })}
          onChange={(value) =>
            onChange(
              patchVariable(template, variable, {
                minimum: typeof value === 'number' ? value : null,
              })
            )
          }
        />
        <NomiInputNumber
          value={variable.maximum ?? undefined}
          placeholder={t('creativeStudio.templates.editor.maximum', {
            defaultValue: 'Maximum',
          })}
          onChange={(value) =>
            onChange(
              patchVariable(template, variable, {
                maximum: typeof value === 'number' ? value : null,
              })
            )
          }
        />
      </div>
    );
  }
  if (variable.type === 'boolean') {
    return (
      <div className={styles.toggleRow}>
        <span>
          {t('creativeStudio.templates.editor.defaultEnabled', {
            defaultValue: 'Enabled by default',
          })}
        </span>
        <Switch
          size='small'
          checked={variable.defaultValue}
          onChange={(checked) =>
            onChange(patchVariable(template, variable, { defaultValue: checked }))
          }
        />
      </div>
    );
  }
  if (variable.type === 'choice') {
    return (
      <div className={styles.variableValueGrid}>
        <NomiInput
          value={variable.options.join(' / ')}
          placeholder={t('creativeStudio.templates.editor.optionsPlaceholder', {
            defaultValue: 'Option one / Option two',
          })}
          onChange={(value) => {
            const options = value
              .split('/')
              .map((option) => option.trim())
              .filter(Boolean)
              .slice(0, 50);
            onChange(
              patchVariable(template, variable, {
                options,
                defaultValue:
                  variable.defaultValue && options.includes(variable.defaultValue)
                    ? variable.defaultValue
                    : options[0] ?? null,
              })
            );
          }}
        />
        <NomiSelect
          value={variable.defaultValue ?? undefined}
          placeholder={t('creativeStudio.templates.editor.default.option', {
            defaultValue: 'Default option',
          })}
          options={variable.options.map((option) => ({ value: option, label: option }))}
          onChange={(value) =>
            onChange(patchVariable(template, variable, { defaultValue: value }))
          }
        />
      </div>
    );
  }
  const selected = generationStep(template).referenceVariableIds.includes(variable.id);
  const assetType = t(
    variable.type === 'image-series'
      ? 'creativeStudio.templates.editor.referenceImages'
      : 'creativeStudio.templates.editor.referenceImage',
    {
      defaultValue: variable.type === 'image-series' ? 'multiple images' : 'image',
    }
  );
  return (
    <div className={styles.toggleRow}>
      <span>
        {t('creativeStudio.templates.editor.referenceSource', {
          assetType,
          defaultValue: 'Select {{assetType}} from My assets',
        })}
      </span>
      <Checkbox
        checked={selected}
        onChange={(checked) =>
          onChange(setTemplateReferenceVariable(template, variable.id, checked))
        }
      >
        {t('creativeStudio.templates.editor.referenceForModel', {
          defaultValue: 'Use as model reference image',
        })}
      </Checkbox>
    </div>
  );
};

const TemplateEditorModal: React.FC<TemplateEditorModalProps> = ({
  template,
  isNew,
  saving,
  onChange,
  onCancel,
  onSave,
  onOpenModelSettings,
  modelCatalog,
}) => {
  const { t } = useTranslation();
  const copy = useMemo(() => createTemplateTranslationCopy(t), [t]);
  const variablePanelId = useId();
  const [expandedVariableId, setExpandedVariableId] = useState<string | null>(null);
  const variableTypeOptions = useMemo(() => buildVariableTypeOptions(t), [t]);
  const qualityOptions = useMemo(() => buildQualityOptions(t), [t]);
  const imageSizeContext = useMemo(() => {
    if (!template) return null;
    const generate = generationStep(template);
    const expectedTask: CreativeTemplateImageTask =
      generate.referenceVariableIds.length > 0 ? 'image_edit' : 'image_generation';
    const options = modelCatalog
      ? exactGenerationModelOptions(modelCatalog, expectedTask)
      : [];
    const selectedModel = generate.generation.model
      ? options.find(
        (option) =>
          option.providerId === generate.generation.model?.providerId &&
          option.model === generate.generation.model.model
      )
      : null;
    const policy = imageGenerationSizePolicyForModel(selectedModel);
    const sizeOptions = imageGenerationFixedSizeOptions(policy.options);
    const selectedSize =
      sizeOptions.find(
        (option) =>
          !option.disabled &&
          option.width === generate.generation.width &&
          option.height === generate.generation.height
      ) ?? sizeOptions.find((option) => !option.disabled) ?? null;
    return {
      expectedTask,
      options,
      policy,
      sizeOptions,
      selectedSize,
      generation: generate.generation,
    };
  }, [modelCatalog, template]);

  useEffect(() => {
    if (!template || !imageSizeContext?.selectedSize) return;
    const { generation, selectedSize, policy } = imageSizeContext;
    const imagesPerPrompt = Math.min(
      generation.imagesPerPrompt,
      Math.min(4, policy.maxCount)
    );
    if (
      generation.width === selectedSize.width &&
      generation.height === selectedSize.height &&
      generation.imagesPerPrompt === imagesPerPrompt
    ) {
      return;
    }
    if (selectedSize.width === null || selectedSize.height === null) return;
    onChange(
      patchGeneration(template, {
        width: selectedSize.width,
        height: selectedSize.height,
        imagesPerPrompt,
      })
    );
  }, [imageSizeContext, onChange, template]);

  useEffect(() => setExpandedVariableId(null), [template?.id]);

  if (!template) return null;
  const generate = generationStep(template);
  const expectedTask = imageSizeContext?.expectedTask ?? 'image_generation';
  const mode = templateMode(template);
  const output = template.output;
  const promptPlanning = draftPromptsStep(template)?.planning ?? null;

  const patchMetadata = (
    patch: Partial<
      Pick<CreativeTemplateDefinitionV1['metadata'], 'name' | 'description' | 'category'>
    >
  ) =>
    onChange({ ...template, metadata: { ...template.metadata, ...patch } });

  return (
    <Modal
      visible
      className={styles.editorModal}
      title={t(
        isNew
          ? 'creativeStudio.templates.editor.createTitle'
          : 'creativeStudio.templates.editor.editTitle',
        { defaultValue: isNew ? 'New template' : 'Edit template' }
      )}
      okText={t('creativeStudio.templates.editor.save', { defaultValue: 'Save' })}
      cancelText={t('creativeStudio.templates.editor.cancel', { defaultValue: 'Cancel' })}
      autoFocus={false}
      unmountOnExit
      confirmLoading={saving}
      getPopupContainer={() => document.body}
      onCancel={onCancel}
      onOk={onSave}
    >
      <div className={styles.editorGrid} data-template-editor>
        <div className={styles.editorMain}>
          <section className={`${styles.editorSection} ${styles.metadataSection}`}>
            <label className={styles.fieldLabel}>
              <span>{t('creativeStudio.templates.editor.namePlaceholder', { defaultValue: 'Template name' })}</span>
              <NomiInput
                value={template.metadata.name}
                placeholder={t('creativeStudio.templates.editor.namePlaceholder', {
                  defaultValue: 'Template name',
                })}
                maxLength={120}
                onChange={(name) => patchMetadata({ name })}
              />
            </label>
            <div className={styles.twoColumns}>
              <label className={styles.fieldLabel}>
                <span>{t('creativeStudio.templates.editor.category', { defaultValue: 'Category' })}</span>
                <NomiInput
                  value={template.metadata.category}
                  placeholder={t('creativeStudio.templates.editor.categoryPlaceholder', {
                    defaultValue: 'Category, e.g. e-commerce poster',
                  })}
                  maxLength={80}
                  onChange={(category) => patchMetadata({ category })}
                />
              </label>
              <div className={styles.fieldLabel}>
                <span id={`${variablePanelId}-mode-label`}>{t('creativeStudio.templates.workspace.templateType', { defaultValue: 'Template type' })}</span>
                <NomiSelect
                  aria-labelledby={`${variablePanelId}-mode-label`}
                  value={mode}
                  options={[
                    {
                      value: 'single-image',
                      label: t('creativeStudio.templates.editor.modeSingle', {
                        defaultValue: 'Single-image template',
                      }),
                    },
                    {
                      value: 'multi-image-series',
                      label: t('creativeStudio.templates.editor.modeMulti', {
                        defaultValue: 'Multi-image template',
                      }),
                    },
                  ]}
                  onChange={(nextMode) =>
                    onChange(
                      switchTemplateMode(template, nextMode as TemplateEditorMode, copy)
                    )
                  }
                />
              </div>
            </div>
            <details
              key={template.id}
              className={styles.descriptionDetails}
              open={Boolean(template.metadata.description) || undefined}
            >
              <summary>
                {t('creativeStudio.templates.editor.descriptionPlaceholder', { defaultValue: 'Describe the intended use' })}
                {t('common.optional', { defaultValue: ' (optional)' })}
              </summary>
              <Input.TextArea
                value={template.metadata.description}
                placeholder={t('creativeStudio.templates.editor.descriptionPlaceholder', {
                  defaultValue: 'Describe the intended use',
                })}
                maxLength={2_000}
                autoSize={{ minRows: 2, maxRows: 4 }}
                onChange={(description) => patchMetadata({ description })}
              />
            </details>
          </section>

          <section className={styles.editorSection}>
            <div className={styles.sectionHeadingRow}>
              <h3>
                {t('creativeStudio.templates.editor.variables', {
                  defaultValue: 'Input variables',
                })}
              </h3>
              <span className={styles.sectionCount}>{template.variables.length}</span>
              <Button
                size='small'
                icon={<Plus theme='outline' size={14} fill='currentColor' />}
                onClick={() => {
                  const variable = createTemplateVariable('text', template.variables.length + 1, copy);
                  onChange({ ...template, variables: [...template.variables, variable] });
                  setExpandedVariableId(variable.id);
                }}
              >
                {t('creativeStudio.templates.editor.addVariable', {
                  defaultValue: 'Add variable',
                })}
              </Button>
            </div>
            <div className={styles.variableList}>
              {template.variables.map((variable) => {
                const expanded = expandedVariableId === variable.id;
                const VariableIcon = variableTypeIcons[variable.type];
                const valueEditor = (
                  <VariableValueEditor
                    template={template}
                    variable={variable}
                    t={t}
                    onChange={onChange}
                  />
                );
                return (
                  <article
                    key={variable.id}
                    className={styles.variableCard}
                    data-expanded={expanded || undefined}
                  >
                    <div className={styles.variableHeader}>
                      <button
                        type='button'
                        className={styles.variableSummary}
                        aria-expanded={expanded}
                        aria-controls={`${variablePanelId}-${variable.id}`}
                        aria-label={expanded ? `${t('common.collapse')} ${variable.label || variable.key}` : undefined}
                        title={expanded ? t('common.collapse') : undefined}
                        onClick={() => setExpandedVariableId(expanded ? null : variable.id)}
                      >
                        {!expanded && (
                          <>
                            <span className={styles.variableIcon} aria-hidden='true'>
                              <VariableIcon theme='outline' size={16} fill='currentColor' />
                            </span>
                            <span className={styles.variableIdentity}>
                              <span className={styles.variableName}>{variable.label || variable.key}</span>
                              <span className={styles.variableKey}>{`{{${variable.key}}}`}</span>
                            </span>
                            <span className={styles.variableMeta}>
                              <span>{variableTypeOptions.find((option) => option.value === variable.type)?.label}</span>
                              <span>{variable.required
                                ? t('creativeStudio.templates.editor.required', { defaultValue: 'Required' })
                                : t('common.optional', { defaultValue: 'Optional' })}</span>
                            </span>
                          </>
                        )}
                        <span className={styles.variableChevron} aria-hidden='true'>
                          <Down theme='outline' size={14} fill='currentColor' />
                        </span>
                      </button>
                      <Button
                        size='small'
                        type='text'
                        status='danger'
                        aria-label={t('creativeStudio.templates.editor.deleteVariable', {
                          name: variable.label,
                          defaultValue: 'Delete variable {{name}}',
                        })}
                        icon={<Delete theme='outline' size={14} fill='currentColor' />}
                        onClick={() => {
                          onChange(removeTemplateVariable(template, variable.id));
                          if (expanded) setExpandedVariableId(null);
                        }}
                      />
                    </div>
                    <div
                      id={`${variablePanelId}-${variable.id}`}
                      className={styles.variableDetails}
                      hidden={!expanded}
                    >
                      <div className={styles.variableFields}>
                        <label className={styles.fieldLabel}>
                          <span>{t('creativeStudio.templates.editor.variableLabelPlaceholder', { defaultValue: 'Display name' })}</span>
                          <NomiInput
                            value={variable.label}
                            placeholder={t('creativeStudio.templates.editor.variableLabelPlaceholder', { defaultValue: 'Display name' })}
                            onChange={(label) => onChange(patchVariable(template, variable, { label }))}
                          />
                        </label>
                        <label className={styles.fieldLabel}>
                          <span>{t('creativeStudio.templates.editor.variableKey', { defaultValue: 'Variable key' })}</span>
                          <NomiInput
                            value={variable.key}
                            placeholder={t(
                              'creativeStudio.templates.editor.variableKeyPlaceholder',
                              { defaultValue: 'Variable key, e.g. product_name' }
                            )}
                            onChange={(key) =>
                              onChange(patchVariable(template, variable, { key }))
                            }
                          />
                        </label>
                      </div>
                      <div className={styles.variableOptions}>
                        <div className={styles.fieldLabel}>
                          <span id={`${variablePanelId}-${variable.id}-type-label`}>{t('creativeStudio.templates.editor.variableTypeLabel', { defaultValue: 'Variable type' })}</span>
                          <NomiSelect
                            aria-labelledby={`${variablePanelId}-${variable.id}-type-label`}
                            value={variable.type}
                            options={variableTypeOptions}
                            onChange={(type) =>
                              onChange(
                                replaceTemplateVariable(
                                  template,
                                  convertTemplateVariable(
                                    variable,
                                    type as TemplateVariableType,
                                    copy
                                  )
                                )
                              )
                            }
                          />
                        </div>
                        <div className={styles.variableRequired}>
                          <Checkbox
                            checked={variable.required}
                            onChange={(required) =>
                              onChange(patchVariable(template, variable, { required }))
                            }
                          >
                            {t('creativeStudio.templates.editor.required', {
                              defaultValue: 'Required',
                            })}
                          </Checkbox>
                        </div>
                        {variable.type === 'text' && (
                          <div className={styles.variableInlineValue}>{valueEditor}</div>
                        )}
                      </div>
                      {variable.type !== 'text' && (
                        <div className={styles.variableValue}>{valueEditor}</div>
                      )}
                    </div>
                  </article>
                );
              })}
            </div>
          </section>

          <section className={styles.editorSection}>
            <h3>
              {t('creativeStudio.templates.editor.promptTemplate', {
                defaultValue: 'Prompt template',
              })}
            </h3>
            <div className={styles.promptEditor}>
              <Input.TextArea
                aria-label={t('creativeStudio.templates.editor.promptTemplate', { defaultValue: 'Prompt template' })}
                value={creativePromptTemplateText(template)}
                placeholder={t('creativeStudio.templates.editor.promptPlaceholder', {
                  defaultValue: 'Enter the image-generation prompt',
                })}
                autoSize={{ minRows: 6, maxRows: 14 }}
                onChange={(text) => onChange(replaceCreativePromptTemplateText(template, text))}
              />
              <p className={styles.promptFooter}>
                {t('creativeStudio.templates.editor.promptHint', {
                  variableName: '{{variableName}}',
                  defaultValue: 'Use {{variableName}} to insert structured variables.',
                })}
              </p>
            </div>
          </section>
        </div>

        <aside className={styles.editorAside}>
          <h3>
            {t('creativeStudio.templates.editor.generationConfig', {
              defaultValue: 'Generation settings',
            })}
          </h3>
          <div className={styles.settingsGroup}>
            <NomiCreativeModelSelect
              className={styles.modelField}
              filter={{ capability: 'task', task: expectedTask }}
              value={
                generate.generation.model
                  ? {
                    providerId: parseProviderId(generate.generation.model.providerId),
                    model: generate.generation.model.model,
                  }
                  : null
              }
              onChange={(selection) => {
                const selectedModel = imageSizeContext?.options.find(
                  (option) =>
                    option.providerId === selection.providerId &&
                    option.model === selection.model
                );
                const policy = imageGenerationSizePolicyForModel(selectedModel);
                const size = imageGenerationFixedSizeOptions(policy.options).find(
                  (option) => !option.disabled
                );
                onChange(
                  patchGeneration(template, {
                    model: {
                      providerId: selection.providerId,
                      model: selection.model,
                      task: expectedTask,
                    },
                    ...(size && size.width !== null && size.height !== null
                      ? { width: size.width, height: size.height }
                      : {}),
                    imagesPerPrompt: Math.min(
                      generate.generation.imagesPerPrompt,
                      Math.min(4, policy.maxCount)
                    ),
                  })
                );
              }}
              onOpenModelSettings={onOpenModelSettings}
            />
            <div className={`${styles.fieldLabel} ${styles.inlineField}`}>
              <span id={`${variablePanelId}-quality-label`}>
                {t('creativeStudio.templates.editor.quality.label', {
                  defaultValue: 'Quality',
                })}
              </span>
              <NomiSelect
                aria-labelledby={`${variablePanelId}-quality-label`}
                value={generate.generation.quality}
                options={qualityOptions}
                onChange={(quality) => onChange(patchGeneration(template, { quality }))}
              />
            </div>
          </div>
          <div className={styles.imageSizeSettings}>
            <ImageSizePicker
              options={imageSizeContext?.sizeOptions ?? []}
              value={imageSizeContext?.selectedSize?.value ?? ''}
              onChange={(size) => {
                if (size.width === null || size.height === null) return;
                onChange(patchGeneration(template, { width: size.width, height: size.height }));
              }}
            />
          </div>
          <div className={styles.settingsGroup}>
            <div className={styles.fieldLabel}>
              <span>
                {t('creativeStudio.templates.editor.imagesPerPrompt', {
                  defaultValue: 'Images per prompt',
                })}
              </span>
              <div
                className={styles.quantityOptions}
                role='group'
                aria-label={t('creativeStudio.templates.editor.imagesPerPrompt', { defaultValue: 'Images per prompt' })}
              >
                {[1, 2, 3, 4].map((imagesPerPrompt) => (
                  <button
                    key={imagesPerPrompt}
                    type='button'
                    className={styles.quantityOption}
                    aria-pressed={generate.generation.imagesPerPrompt === imagesPerPrompt}
                    disabled={imagesPerPrompt > (imageSizeContext?.policy.maxCount ?? 1)}
                    onClick={() => onChange(patchGeneration(template, { imagesPerPrompt }))}
                  >
                    {imagesPerPrompt}
                  </button>
                ))}
              </div>
            </div>
          </div>

          {output.kind === 'multi-image-series' ? (
            <div className={styles.seriesSettings}>
              <h4>
                {t('creativeStudio.templates.editor.seriesPlanning', {
                  defaultValue: 'Multi-image prompt planning',
                })}
              </h4>
              <NomiCreativeModelSelect
                className={styles.modelField}
                label={t('creativeStudio.templates.editor.planningModel', { defaultValue: 'Prompt planning model' })}
                filter={{ capability: 'task', task: 'chat' }}
                value={
                  promptPlanning?.model
                    ? {
                      providerId: parseProviderId(promptPlanning.model.providerId),
                      model: promptPlanning.model.model,
                    }
                    : null
                }
                onChange={(selection) =>
                  onChange(
                    patchPromptPlanning(template, {
                      model: {
                        providerId: selection.providerId,
                        model: selection.model,
                        task: 'chat',
                      },
                    })
                  )
                }
                onOpenModelSettings={onOpenModelSettings}
              />
              <label className={styles.fieldLabel}>
                <span>
                  {t('creativeStudio.templates.editor.splitInstruction', {
                    defaultValue: 'Series split requirements',
                  })}
                </span>
                <Input.TextArea
                  value={promptPlanning?.instruction ?? ''}
                  maxLength={2_000}
                  autoSize={{ minRows: 3, maxRows: 6 }}
                  placeholder={t('creativeStudio.templates.editor.splitPlaceholder', {
                    defaultValue:
                      'Explain how images should divide the work while staying coherent',
                  })}
                  onChange={(instruction) =>
                    onChange(patchPromptPlanning(template, { instruction }))
                  }
                />
              </label>
              <label className={styles.fieldLabel}>
                <span>
                  {t('creativeStudio.templates.editor.count', {
                    defaultValue: 'Number of images',
                  })}
                </span>
                <NomiInputNumber
                  min={2}
                  max={20}
                  value={output.targetCount}
                  onChange={(targetCount) =>
                    typeof targetCount === 'number' &&
                    onChange({
                      ...template,
                      output: { ...output, targetCount },
                    })
                  }
                />
              </label>
              <div className={styles.toggleRow}>
                <span>
                  {t('creativeStudio.templates.editor.reviewRequired', {
                    defaultValue: 'Review prompts before generation',
                  })}
                </span>
                <Switch
                  size='small'
                  checked={output.reviewRequired}
                  onChange={(reviewRequired) =>
                    onChange({
                      ...template,
                      output: { ...output, reviewRequired },
                    })
                  }
                />
              </div>
              <details className={styles.advancedSettings}>
                <summary>{t('settings.advancedSettings', { defaultValue: 'Advanced settings' })}</summary>
                <label className={styles.fieldLabel}>
                  <span>
                    {t('creativeStudio.templates.editor.maxTokens', {
                      defaultValue: 'Maximum planning output tokens',
                    })}
                  </span>
                  <OutputLimitInput
                    value={promptPlanning?.maxTokens ?? undefined}
                    onChange={(maxTokens) => onChange(patchPromptPlanning(template, { maxTokens }))}
                    compact
                  />
                </label>
                <label className={styles.fieldLabel}>
                  <span>
                    {t('creativeStudio.templates.editor.concurrency', {
                      defaultValue: 'Concurrency',
                    })}
                  </span>
                  <NomiInputNumber
                    min={1}
                    max={20}
                    value={output.concurrency}
                    onChange={(concurrency) =>
                      typeof concurrency === 'number' &&
                      onChange({ ...template, output: { ...output, concurrency } })
                    }
                  />
                </label>
              </details>
            </div>
          ) : null}
        </aside>
      </div>
    </Modal>
  );
};

export default TemplateEditorModal;
