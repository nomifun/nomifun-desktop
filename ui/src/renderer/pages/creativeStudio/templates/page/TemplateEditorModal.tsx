/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Button,
  Checkbox,
  Input,
  InputNumber,
  Modal,
  Select,
  Switch,
} from '@arco-design/web-react';
import { Delete, Plus } from '@icon-park/react';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';

import { parseProviderId } from '@/common/types/ids';
import NomiCreativeModelSelect from '../../models/NomiCreativeModelSelect';
import {
  cloneTemplateDefinition,
  type CreativeTemplateDefinitionV1,
  type CreativeTemplateImageGenerationSettings,
  type CreativeTemplatePromptPlanningSettings,
  type CreativeTemplateVariable,
} from '../domain';
import styles from './CreativeTemplateWorkspacePage.module.css';
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
    const controlProps = {
      value: variable.defaultValue ?? '',
      placeholder:
        variable.type === 'multiline-text'
          ? t('creativeStudio.templates.editor.default.multiline', {
              defaultValue: 'Default long text (optional)',
            })
          : t('creativeStudio.templates.editor.default.value', {
              defaultValue: 'Default value (optional)',
            }),
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
      <Input {...controlProps} />
    );
  }
  if (variable.type === 'number') {
    return (
      <div className={styles.variableValueGrid}>
        <InputNumber
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
        <InputNumber
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
        <InputNumber
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
        <Input
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
        <Select
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
}) => {
  const { t } = useTranslation();
  const copy = useMemo(() => createTemplateTranslationCopy(t), [t]);
  const variableTypeOptions = useMemo(() => buildVariableTypeOptions(t), [t]);
  const qualityOptions = useMemo(() => buildQualityOptions(t), [t]);
  if (!template) return null;
  const generate = generationStep(template);
  const expectedTask = generate.referenceVariableIds.length > 0 ? 'image_edit' : 'image_generation';
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
      getPopupContainer={() =>
        document.getElementById('creative-studio-portal-root') ?? document.body
      }
      onCancel={onCancel}
      onOk={onSave}
    >
      <div className={styles.editorGrid} data-template-editor>
        <div className={styles.editorMain}>
          <section className={styles.editorSection}>
            <h3>
              {t('creativeStudio.templates.editor.basicInfo', {
                defaultValue: 'Basic information',
              })}
            </h3>
            <Input
              value={template.metadata.name}
              placeholder={t('creativeStudio.templates.editor.namePlaceholder', {
                defaultValue: 'Template name',
              })}
              maxLength={120}
              onChange={(name) => patchMetadata({ name })}
            />
            <div className={styles.twoColumns}>
              <Input
                value={template.metadata.category}
                placeholder={t('creativeStudio.templates.editor.categoryPlaceholder', {
                  defaultValue: 'Category, e.g. e-commerce poster',
                })}
                maxLength={80}
                onChange={(category) => patchMetadata({ category })}
              />
              <Select
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
            <Input.TextArea
              value={template.metadata.description}
              placeholder={t('creativeStudio.templates.editor.descriptionPlaceholder', {
                defaultValue: 'Describe the intended use',
              })}
              maxLength={2_000}
              autoSize={{ minRows: 2, maxRows: 4 }}
              onChange={(description) => patchMetadata({ description })}
            />
          </section>

          <section className={styles.editorSection}>
            <div className={styles.sectionHeadingRow}>
              <h3>
                {t('creativeStudio.templates.editor.variables', {
                  defaultValue: 'Input variables',
                })}
              </h3>
              <Button
                size='small'
                icon={<Plus theme='outline' size={14} fill='currentColor' />}
                onClick={() =>
                  onChange({
                      ...template,
                      variables: [
                        ...template.variables,
                        createTemplateVariable(
                          'text',
                          template.variables.length + 1,
                          copy
                        ),
                      ],
                    })
                  }
                >
                  {t('creativeStudio.templates.editor.addVariable', {
                    defaultValue: 'Add variable',
                  })}
              </Button>
            </div>
            <div className={styles.variableList}>
              {template.variables.map((variable) => (
                <article key={variable.id} className={styles.variableCard}>
                  <div className={styles.variableRow}>
                    <Input
                      value={variable.key}
                      placeholder={t(
                        'creativeStudio.templates.editor.variableKeyPlaceholder',
                        { defaultValue: 'Variable key, e.g. product_name' }
                      )}
                      onChange={(key) =>
                        onChange(patchVariable(template, variable, { key }))
                      }
                    />
                    <Input
                      value={variable.label}
                      placeholder={t('creativeStudio.templates.editor.variableLabelPlaceholder', {
                        defaultValue: 'Display name',
                      })}
                      onChange={(label) =>
                        onChange(patchVariable(template, variable, { label }))
                      }
                    />
                    <Select
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
                    <Button
                      size='small'
                      status='danger'
                      aria-label={t('creativeStudio.templates.editor.deleteVariable', {
                        name: variable.label,
                        defaultValue: 'Delete variable {{name}}',
                      })}
                      icon={<Delete theme='outline' size={14} fill='currentColor' />}
                      onClick={() => onChange(removeTemplateVariable(template, variable.id))}
                    />
                  </div>
                  <VariableValueEditor
                    template={template}
                    variable={variable}
                    t={t}
                    onChange={onChange}
                  />
                </article>
              ))}
            </div>
          </section>

          <section className={styles.editorSection}>
            <h3>
              {t('creativeStudio.templates.editor.promptTemplate', {
                defaultValue: 'Prompt template',
              })}
            </h3>
            <p className={styles.sectionHint}>
              {t('creativeStudio.templates.editor.promptHint', {
                variableName: '{{variableName}}',
                defaultValue: 'Use {{variableName}} to insert structured variables.',
              })}
            </p>
            <Input.TextArea
              value={creativePromptTemplateText(template)}
              placeholder={t('creativeStudio.templates.editor.promptPlaceholder', {
                defaultValue: 'Enter the image-generation prompt',
              })}
              autoSize={{ minRows: 7, maxRows: 14 }}
              onChange={(text) => onChange(replaceCreativePromptTemplateText(template, text))}
            />
          </section>
        </div>

        <aside className={styles.editorAside}>
          <h3>
            {t('creativeStudio.templates.editor.generationConfig', {
              defaultValue: 'Generation settings',
            })}
          </h3>
          <NomiCreativeModelSelect
            filter={{ capability: 'task', task: expectedTask }}
            value={
              generate.generation.model
                ? {
                    providerId: parseProviderId(generate.generation.model.providerId),
                    model: generate.generation.model.model,
                  }
                : null
            }
            onChange={(selection) =>
              onChange(
                patchGeneration(template, {
                  model: {
                    providerId: selection.providerId,
                    model: selection.model,
                    task: expectedTask,
                  },
                })
              )
            }
            onOpenModelSettings={onOpenModelSettings}
          />
          <label className={styles.fieldLabel}>
            <span>
              {t('creativeStudio.templates.editor.quality.label', {
                defaultValue: 'Quality',
              })}
            </span>
            <Select
              value={generate.generation.quality}
              options={qualityOptions}
              onChange={(quality) => onChange(patchGeneration(template, { quality }))}
            />
          </label>
          <div className={styles.twoColumns}>
            <label className={styles.fieldLabel}>
              <span>
                {t('creativeStudio.templates.editor.width', { defaultValue: 'Width' })}
              </span>
              <InputNumber
                min={64}
                max={8192}
                step={16}
                value={generate.generation.width}
                onChange={(width) =>
                  typeof width === 'number' &&
                  onChange(patchGeneration(template, { width }))
                }
              />
            </label>
            <label className={styles.fieldLabel}>
              <span>
                {t('creativeStudio.templates.editor.height', { defaultValue: 'Height' })}
              </span>
              <InputNumber
                min={64}
                max={8192}
                step={16}
                value={generate.generation.height}
                onChange={(height) =>
                  typeof height === 'number' &&
                  onChange(patchGeneration(template, { height }))
                }
              />
            </label>
          </div>
          <label className={styles.fieldLabel}>
            <span>
              {t('creativeStudio.templates.editor.imagesPerPrompt', {
                defaultValue: 'Images per prompt',
              })}
            </span>
            <InputNumber
              min={1}
              max={6}
              value={generate.generation.imagesPerPrompt}
              onChange={(imagesPerPrompt) =>
                typeof imagesPerPrompt === 'number' &&
                onChange(patchGeneration(template, { imagesPerPrompt }))
              }
            />
          </label>

          {output.kind === 'multi-image-series' ? (
            <div className={styles.seriesSettings}>
              <h4>
                {t('creativeStudio.templates.editor.seriesPlanning', {
                  defaultValue: 'Multi-image prompt planning',
                })}
              </h4>
              <label className={styles.fieldLabel}>
                <span>
                  {t('creativeStudio.templates.editor.planningModel', {
                    defaultValue: 'Prompt planning model',
                  })}
                </span>
                <NomiCreativeModelSelect
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
              </label>
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
                  {t('creativeStudio.templates.editor.maxTokens', {
                    defaultValue: 'Maximum planning output tokens',
                  })}
                </span>
                <InputNumber
                  min={128}
                  max={32_768}
                  step={128}
                  value={promptPlanning?.maxTokens ?? 4096}
                  onChange={(maxTokens) =>
                    typeof maxTokens === 'number' &&
                    onChange(patchPromptPlanning(template, { maxTokens }))
                  }
                />
              </label>
              <div className={styles.twoColumns}>
                <label className={styles.fieldLabel}>
                  <span>
                    {t('creativeStudio.templates.editor.count', {
                      defaultValue: 'Number of images',
                    })}
                  </span>
                  <InputNumber
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
                <label className={styles.fieldLabel}>
                  <span>
                    {t('creativeStudio.templates.editor.concurrency', {
                      defaultValue: 'Concurrency',
                    })}
                  </span>
                  <InputNumber
                    min={1}
                    max={20}
                    value={output.concurrency}
                    onChange={(concurrency) =>
                      typeof concurrency === 'number' &&
                      onChange({
                        ...template,
                        output: { ...output, concurrency },
                      })
                    }
                  />
                </label>
              </div>
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
            </div>
          ) : null}
        </aside>
      </div>
    </Modal>
  );
};

export default TemplateEditorModal;
