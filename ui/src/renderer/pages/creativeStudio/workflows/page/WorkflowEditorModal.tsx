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
  cloneWorkflowDefinition,
  type WorkflowDefinitionV1,
  type WorkflowImageGenerationSettings,
  type WorkflowPromptPlanningSettings,
  type WorkflowVariable,
} from '../domain';
import styles from './CreativeWorkflowWorkspacePage.module.css';
import {
  convertWorkflowVariable,
  createWorkflowVariable,
  draftPromptsStep,
  generationStep,
  removeWorkflowVariable,
  replaceWorkflowTemplateText,
  replaceWorkflowVariable,
  setWorkflowReferenceVariable,
  switchWorkflowMode,
  workflowMode,
  workflowTemplateText,
  type WorkflowEditorMode,
  type WorkflowVariableType,
} from './workflowViewModel';
import { createWorkflowTranslationCopy } from '../workflowI18n';

const buildVariableTypeOptions = (t: TFunction): Array<{
  value: WorkflowVariableType;
  label: string;
}> => [
  {
    value: 'text',
    label: t('creativeStudio.workflows.editor.variableType.text', {
      defaultValue: 'Single-line text',
    }),
  },
  {
    value: 'multiline-text',
    label: t('creativeStudio.workflows.editor.variableType.multilineText', {
      defaultValue: 'Long text',
    }),
  },
  {
    value: 'number',
    label: t('creativeStudio.workflows.editor.variableType.number', {
      defaultValue: 'Number',
    }),
  },
  {
    value: 'boolean',
    label: t('creativeStudio.workflows.editor.variableType.boolean', {
      defaultValue: 'Toggle',
    }),
  },
  {
    value: 'choice',
    label: t('creativeStudio.workflows.editor.variableType.choice', {
      defaultValue: 'Select',
    }),
  },
  {
    value: 'image',
    label: t('creativeStudio.workflows.editor.variableType.image', {
      defaultValue: 'Reference image',
    }),
  },
  {
    value: 'image-series',
    label: t('creativeStudio.workflows.editor.variableType.imageSeries', {
      defaultValue: 'Reference image set',
    }),
  },
];

const buildQualityOptions = (t: TFunction) => [
  {
    value: 'auto',
    label: t('creativeStudio.workflows.editor.quality.auto', {
      defaultValue: 'Automatic',
    }),
  },
  {
    value: 'high',
    label: t('creativeStudio.workflows.editor.quality.high', {
      defaultValue: 'High',
    }),
  },
  {
    value: 'medium',
    label: t('creativeStudio.workflows.editor.quality.medium', {
      defaultValue: 'Medium',
    }),
  },
  {
    value: 'low',
    label: t('creativeStudio.workflows.editor.quality.low', {
      defaultValue: 'Low',
    }),
  },
];

export interface WorkflowEditorModalProps {
  workflow: WorkflowDefinitionV1 | null;
  isNew: boolean;
  saving: boolean;
  onChange: (workflow: WorkflowDefinitionV1) => void;
  onCancel: () => void;
  onSave: () => void;
  onOpenModelSettings?: () => void;
}

function patchGeneration(
  workflow: WorkflowDefinitionV1,
  patch: Partial<WorkflowImageGenerationSettings>
): WorkflowDefinitionV1 {
  const next = cloneWorkflowDefinition(workflow);
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
  workflow: WorkflowDefinitionV1,
  patch: Partial<WorkflowPromptPlanningSettings>
): WorkflowDefinitionV1 {
  const next = cloneWorkflowDefinition(workflow);
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
  workflow: WorkflowDefinitionV1,
  variable: WorkflowVariable,
  patch: Partial<WorkflowVariable>
) => replaceWorkflowVariable(workflow, { ...variable, ...patch } as WorkflowVariable);

const VariableValueEditor: React.FC<{
  workflow: WorkflowDefinitionV1;
  variable: WorkflowVariable;
  t: TFunction;
  onChange: (workflow: WorkflowDefinitionV1) => void;
}> = ({ workflow, variable, t, onChange }) => {
  if (variable.type === 'text' || variable.type === 'multiline-text') {
    const controlProps = {
      value: variable.defaultValue ?? '',
      placeholder:
        variable.type === 'multiline-text'
          ? t('creativeStudio.workflows.editor.default.multiline', {
              defaultValue: 'Default long text (optional)',
            })
          : t('creativeStudio.workflows.editor.default.value', {
              defaultValue: 'Default value (optional)',
            }),
      onChange: (value: string) =>
        onChange(
          patchVariable(workflow, variable, {
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
          placeholder={t('creativeStudio.workflows.editor.default.number', {
            defaultValue: 'Default number',
          })}
          onChange={(value) =>
            onChange(
              patchVariable(workflow, variable, {
                defaultValue: typeof value === 'number' ? value : null,
              })
            )
          }
        />
        <InputNumber
          value={variable.minimum ?? undefined}
          placeholder={t('creativeStudio.workflows.editor.minimum', {
            defaultValue: 'Minimum',
          })}
          onChange={(value) =>
            onChange(
              patchVariable(workflow, variable, {
                minimum: typeof value === 'number' ? value : null,
              })
            )
          }
        />
        <InputNumber
          value={variable.maximum ?? undefined}
          placeholder={t('creativeStudio.workflows.editor.maximum', {
            defaultValue: 'Maximum',
          })}
          onChange={(value) =>
            onChange(
              patchVariable(workflow, variable, {
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
          {t('creativeStudio.workflows.editor.defaultEnabled', {
            defaultValue: 'Enabled by default',
          })}
        </span>
        <Switch
          size='small'
          checked={variable.defaultValue}
          onChange={(checked) =>
            onChange(patchVariable(workflow, variable, { defaultValue: checked }))
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
          placeholder={t('creativeStudio.workflows.editor.optionsPlaceholder', {
            defaultValue: 'Option one / Option two',
          })}
          onChange={(value) => {
            const options = value
              .split('/')
              .map((option) => option.trim())
              .filter(Boolean)
              .slice(0, 50);
            onChange(
              patchVariable(workflow, variable, {
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
          placeholder={t('creativeStudio.workflows.editor.default.option', {
            defaultValue: 'Default option',
          })}
          options={variable.options.map((option) => ({ value: option, label: option }))}
          onChange={(value) =>
            onChange(patchVariable(workflow, variable, { defaultValue: value }))
          }
        />
      </div>
    );
  }
  const selected = generationStep(workflow).referenceVariableIds.includes(variable.id);
  const assetType = t(
    variable.type === 'image-series'
      ? 'creativeStudio.workflows.editor.referenceImages'
      : 'creativeStudio.workflows.editor.referenceImage',
    {
      defaultValue: variable.type === 'image-series' ? 'multiple images' : 'image',
    }
  );
  return (
    <div className={styles.toggleRow}>
      <span>
        {t('creativeStudio.workflows.editor.referenceSource', {
          assetType,
          defaultValue: 'Select {{assetType}} from My assets',
        })}
      </span>
      <Checkbox
        checked={selected}
        onChange={(checked) =>
          onChange(setWorkflowReferenceVariable(workflow, variable.id, checked))
        }
      >
        {t('creativeStudio.workflows.editor.referenceForModel', {
          defaultValue: 'Use as model reference image',
        })}
      </Checkbox>
    </div>
  );
};

const WorkflowEditorModal: React.FC<WorkflowEditorModalProps> = ({
  workflow,
  isNew,
  saving,
  onChange,
  onCancel,
  onSave,
  onOpenModelSettings,
}) => {
  const { t } = useTranslation();
  const copy = useMemo(() => createWorkflowTranslationCopy(t), [t]);
  const variableTypeOptions = useMemo(() => buildVariableTypeOptions(t), [t]);
  const qualityOptions = useMemo(() => buildQualityOptions(t), [t]);
  if (!workflow) return null;
  const generate = generationStep(workflow);
  const expectedTask = generate.referenceVariableIds.length > 0 ? 'image_edit' : 'image_generation';
  const mode = workflowMode(workflow);
  const output = workflow.output;
  const promptPlanning = draftPromptsStep(workflow)?.planning ?? null;

  const patchMetadata = (
    patch: Partial<
      Pick<WorkflowDefinitionV1['metadata'], 'name' | 'description' | 'category'>
    >
  ) =>
    onChange({ ...workflow, metadata: { ...workflow.metadata, ...patch } });

  return (
    <Modal
      visible
      className={styles.editorModal}
      title={t(
        isNew
          ? 'creativeStudio.workflows.editor.createTitle'
          : 'creativeStudio.workflows.editor.editTitle',
        { defaultValue: isNew ? 'New template' : 'Edit template' }
      )}
      okText={t('creativeStudio.workflows.editor.save', { defaultValue: 'Save' })}
      cancelText={t('creativeStudio.workflows.editor.cancel', { defaultValue: 'Cancel' })}
      autoFocus={false}
      unmountOnExit
      confirmLoading={saving}
      getPopupContainer={() =>
        document.getElementById('creative-studio-portal-root') ?? document.body
      }
      onCancel={onCancel}
      onOk={onSave}
    >
      <div className={styles.editorGrid} data-workflow-editor>
        <div className={styles.editorMain}>
          <section className={styles.editorSection}>
            <h3>
              {t('creativeStudio.workflows.editor.basicInfo', {
                defaultValue: 'Basic information',
              })}
            </h3>
            <Input
              value={workflow.metadata.name}
              placeholder={t('creativeStudio.workflows.editor.namePlaceholder', {
                defaultValue: 'Template name',
              })}
              maxLength={120}
              onChange={(name) => patchMetadata({ name })}
            />
            <div className={styles.twoColumns}>
              <Input
                value={workflow.metadata.category}
                placeholder={t('creativeStudio.workflows.editor.categoryPlaceholder', {
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
                    label: t('creativeStudio.workflows.editor.modeSingle', {
                      defaultValue: 'Single-image template',
                    }),
                  },
                  {
                    value: 'multi-image-series',
                    label: t('creativeStudio.workflows.editor.modeMulti', {
                      defaultValue: 'Multi-image template',
                    }),
                  },
                ]}
                onChange={(nextMode) =>
                  onChange(
                    switchWorkflowMode(workflow, nextMode as WorkflowEditorMode, copy)
                  )
                }
              />
            </div>
            <Input.TextArea
              value={workflow.metadata.description}
              placeholder={t('creativeStudio.workflows.editor.descriptionPlaceholder', {
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
                {t('creativeStudio.workflows.editor.variables', {
                  defaultValue: 'Input variables',
                })}
              </h3>
              <Button
                size='small'
                icon={<Plus theme='outline' size={14} fill='currentColor' />}
                onClick={() =>
                  onChange({
                      ...workflow,
                      variables: [
                        ...workflow.variables,
                        createWorkflowVariable(
                          'text',
                          workflow.variables.length + 1,
                          copy
                        ),
                      ],
                    })
                  }
                >
                  {t('creativeStudio.workflows.editor.addVariable', {
                    defaultValue: 'Add variable',
                  })}
              </Button>
            </div>
            <div className={styles.variableList}>
              {workflow.variables.map((variable) => (
                <article key={variable.id} className={styles.variableCard}>
                  <div className={styles.variableRow}>
                    <Input
                      value={variable.key}
                      placeholder={t(
                        'creativeStudio.workflows.editor.variableKeyPlaceholder',
                        { defaultValue: 'Variable key, e.g. product_name' }
                      )}
                      onChange={(key) =>
                        onChange(patchVariable(workflow, variable, { key }))
                      }
                    />
                    <Input
                      value={variable.label}
                      placeholder={t('creativeStudio.workflows.editor.variableLabelPlaceholder', {
                        defaultValue: 'Display name',
                      })}
                      onChange={(label) =>
                        onChange(patchVariable(workflow, variable, { label }))
                      }
                    />
                    <Select
                      value={variable.type}
                      options={variableTypeOptions}
                      onChange={(type) =>
                        onChange(
                          replaceWorkflowVariable(
                            workflow,
                            convertWorkflowVariable(
                              variable,
                              type as WorkflowVariableType,
                              copy
                            )
                          )
                        )
                      }
                    />
                    <Checkbox
                      checked={variable.required}
                      onChange={(required) =>
                        onChange(patchVariable(workflow, variable, { required }))
                      }
                    >
                      {t('creativeStudio.workflows.editor.required', {
                        defaultValue: 'Required',
                      })}
                    </Checkbox>
                    <Button
                      size='small'
                      status='danger'
                      aria-label={t('creativeStudio.workflows.editor.deleteVariable', {
                        name: variable.label,
                        defaultValue: 'Delete variable {{name}}',
                      })}
                      icon={<Delete theme='outline' size={14} fill='currentColor' />}
                      onClick={() => onChange(removeWorkflowVariable(workflow, variable.id))}
                    />
                  </div>
                  <VariableValueEditor
                    workflow={workflow}
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
              {t('creativeStudio.workflows.editor.promptTemplate', {
                defaultValue: 'Prompt template',
              })}
            </h3>
            <p className={styles.sectionHint}>
              {t('creativeStudio.workflows.editor.promptHint', {
                variableName: '{{variableName}}',
                defaultValue: 'Use {{variableName}} to insert structured variables.',
              })}
            </p>
            <Input.TextArea
              value={workflowTemplateText(workflow)}
              placeholder={t('creativeStudio.workflows.editor.promptPlaceholder', {
                defaultValue: 'Enter the image-generation prompt',
              })}
              autoSize={{ minRows: 7, maxRows: 14 }}
              onChange={(text) => onChange(replaceWorkflowTemplateText(workflow, text))}
            />
          </section>
        </div>

        <aside className={styles.editorAside}>
          <h3>
            {t('creativeStudio.workflows.editor.generationConfig', {
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
                patchGeneration(workflow, {
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
              {t('creativeStudio.workflows.editor.quality.label', {
                defaultValue: 'Quality',
              })}
            </span>
            <Select
              value={generate.generation.quality}
              options={qualityOptions}
              onChange={(quality) => onChange(patchGeneration(workflow, { quality }))}
            />
          </label>
          <div className={styles.twoColumns}>
            <label className={styles.fieldLabel}>
              <span>
                {t('creativeStudio.workflows.editor.width', { defaultValue: 'Width' })}
              </span>
              <InputNumber
                min={64}
                max={8192}
                step={16}
                value={generate.generation.width}
                onChange={(width) =>
                  typeof width === 'number' &&
                  onChange(patchGeneration(workflow, { width }))
                }
              />
            </label>
            <label className={styles.fieldLabel}>
              <span>
                {t('creativeStudio.workflows.editor.height', { defaultValue: 'Height' })}
              </span>
              <InputNumber
                min={64}
                max={8192}
                step={16}
                value={generate.generation.height}
                onChange={(height) =>
                  typeof height === 'number' &&
                  onChange(patchGeneration(workflow, { height }))
                }
              />
            </label>
          </div>
          <label className={styles.fieldLabel}>
            <span>
              {t('creativeStudio.workflows.editor.imagesPerPrompt', {
                defaultValue: 'Images per prompt',
              })}
            </span>
            <InputNumber
              min={1}
              max={6}
              value={generate.generation.imagesPerPrompt}
              onChange={(imagesPerPrompt) =>
                typeof imagesPerPrompt === 'number' &&
                onChange(patchGeneration(workflow, { imagesPerPrompt }))
              }
            />
          </label>

          {output.kind === 'multi-image-series' ? (
            <div className={styles.seriesSettings}>
              <h4>
                {t('creativeStudio.workflows.editor.seriesPlanning', {
                  defaultValue: 'Multi-image prompt planning',
                })}
              </h4>
              <label className={styles.fieldLabel}>
                <span>
                  {t('creativeStudio.workflows.editor.planningModel', {
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
                      patchPromptPlanning(workflow, {
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
                  {t('creativeStudio.workflows.editor.splitInstruction', {
                    defaultValue: 'Series split requirements',
                  })}
                </span>
                <Input.TextArea
                  value={promptPlanning?.instruction ?? ''}
                  maxLength={2_000}
                  autoSize={{ minRows: 3, maxRows: 6 }}
                  placeholder={t('creativeStudio.workflows.editor.splitPlaceholder', {
                    defaultValue:
                      'Explain how images should divide the work while staying coherent',
                  })}
                  onChange={(instruction) =>
                    onChange(patchPromptPlanning(workflow, { instruction }))
                  }
                />
              </label>
              <label className={styles.fieldLabel}>
                <span>
                  {t('creativeStudio.workflows.editor.maxTokens', {
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
                    onChange(patchPromptPlanning(workflow, { maxTokens }))
                  }
                />
              </label>
              <div className={styles.twoColumns}>
                <label className={styles.fieldLabel}>
                  <span>
                    {t('creativeStudio.workflows.editor.count', {
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
                        ...workflow,
                        output: { ...output, targetCount },
                      })
                    }
                  />
                </label>
                <label className={styles.fieldLabel}>
                  <span>
                    {t('creativeStudio.workflows.editor.concurrency', {
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
                        ...workflow,
                        output: { ...output, concurrency },
                      })
                    }
                  />
                </label>
              </div>
              <div className={styles.toggleRow}>
                <span>
                  {t('creativeStudio.workflows.editor.reviewRequired', {
                    defaultValue: 'Review prompts before generation',
                  })}
                </span>
                <Switch
                  size='small'
                  checked={output.reviewRequired}
                  onChange={(reviewRequired) =>
                    onChange({
                      ...workflow,
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

export default WorkflowEditorModal;
