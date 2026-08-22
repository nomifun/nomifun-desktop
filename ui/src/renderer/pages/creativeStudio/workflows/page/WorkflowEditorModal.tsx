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
import React from 'react';

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

const VARIABLE_TYPE_OPTIONS: Array<{ value: WorkflowVariableType; label: string }> = [
  { value: 'text', label: '单行文本' },
  { value: 'multiline-text', label: '长文本' },
  { value: 'number', label: '数字' },
  { value: 'boolean', label: '开关' },
  { value: 'choice', label: '下拉选项' },
  { value: 'image', label: '参考图' },
  { value: 'image-series', label: '参考图组' },
];

const QUALITY_OPTIONS = [
  { value: 'auto', label: '自动质量' },
  { value: 'high', label: '高质量' },
  { value: 'medium', label: '中等质量' },
  { value: 'low', label: '低质量' },
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
  onChange: (workflow: WorkflowDefinitionV1) => void;
}> = ({ workflow, variable, onChange }) => {
  if (variable.type === 'text' || variable.type === 'multiline-text') {
    const controlProps = {
      value: variable.defaultValue ?? '',
      placeholder: variable.type === 'multiline-text' ? '默认长文本（可选）' : '默认值（可选）',
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
          placeholder='默认数字'
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
          placeholder='最小值'
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
          placeholder='最大值'
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
        <span>默认开启</span>
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
          placeholder='选项一 / 选项二'
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
          placeholder='默认选项'
          options={variable.options.map((option) => ({ value: option, label: option }))}
          onChange={(value) =>
            onChange(patchVariable(workflow, variable, { defaultValue: value }))
          }
        />
      </div>
    );
  }
  const selected = generationStep(workflow).referenceVariableIds.includes(variable.id);
  return (
    <div className={styles.toggleRow}>
      <span>运行时从“我的素材”选择{variable.type === 'image-series' ? '多张图片' : '图片'}</span>
      <Checkbox
        checked={selected}
        onChange={(checked) =>
          onChange(setWorkflowReferenceVariable(workflow, variable.id, checked))
        }
      >
        用作模型参考图
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
      title={isNew ? '新建工作流' : '编辑工作流'}
      okText='保存'
      cancelText='取消'
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
            <h3>基础信息</h3>
            <Input
              value={workflow.metadata.name}
              placeholder='工作流名称'
              maxLength={120}
              onChange={(name) => patchMetadata({ name })}
            />
            <div className={styles.twoColumns}>
              <Input
                value={workflow.metadata.category}
                placeholder='分类，例如 电商海报'
                maxLength={80}
                onChange={(category) => patchMetadata({ category })}
              />
              <Select
                value={mode}
                options={[
                  { value: 'single-image', label: '单图工作流' },
                  { value: 'multi-image-series', label: '多图工作流' },
                ]}
                onChange={(nextMode) =>
                  onChange(switchWorkflowMode(workflow, nextMode as WorkflowEditorMode))
                }
              />
            </div>
            <Input.TextArea
              value={workflow.metadata.description}
              placeholder='适用场景说明'
              maxLength={2_000}
              autoSize={{ minRows: 2, maxRows: 4 }}
              onChange={(description) => patchMetadata({ description })}
            />
          </section>

          <section className={styles.editorSection}>
            <div className={styles.sectionHeadingRow}>
              <h3>输入变量</h3>
              <Button
                size='small'
                icon={<Plus theme='outline' size={14} fill='currentColor' />}
                onClick={() =>
                  onChange({
                    ...workflow,
                    variables: [
                      ...workflow.variables,
                      createWorkflowVariable('text', workflow.variables.length + 1),
                    ],
                  })
                }
              >
                添加变量
              </Button>
            </div>
            <div className={styles.variableList}>
              {workflow.variables.map((variable) => (
                <article key={variable.id} className={styles.variableCard}>
                  <div className={styles.variableRow}>
                    <Input
                      value={variable.key}
                      placeholder='变量名 product_name'
                      onChange={(key) =>
                        onChange(patchVariable(workflow, variable, { key }))
                      }
                    />
                    <Input
                      value={variable.label}
                      placeholder='显示名称'
                      onChange={(label) =>
                        onChange(patchVariable(workflow, variable, { label }))
                      }
                    />
                    <Select
                      value={variable.type}
                      options={VARIABLE_TYPE_OPTIONS}
                      onChange={(type) =>
                        onChange(
                          replaceWorkflowVariable(
                            workflow,
                            convertWorkflowVariable(variable, type as WorkflowVariableType)
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
                      必填
                    </Checkbox>
                    <Button
                      size='small'
                      status='danger'
                      aria-label={`删除变量 ${variable.label}`}
                      icon={<Delete theme='outline' size={14} fill='currentColor' />}
                      onClick={() => onChange(removeWorkflowVariable(workflow, variable.id))}
                    />
                  </div>
                  <VariableValueEditor
                    workflow={workflow}
                    variable={variable}
                    onChange={onChange}
                  />
                </article>
              ))}
            </div>
          </section>

          <section className={styles.editorSection}>
            <h3>提示词模板</h3>
            <p className={styles.sectionHint}>使用 {'{{变量名}}'} 插入结构化变量。</p>
            <Input.TextArea
              value={workflowTemplateText(workflow)}
              placeholder='填写图片生成提示词'
              autoSize={{ minRows: 7, maxRows: 14 }}
              onChange={(text) => onChange(replaceWorkflowTemplateText(workflow, text))}
            />
          </section>
        </div>

        <aside className={styles.editorAside}>
          <h3>生成配置</h3>
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
            <span>生成质量</span>
            <Select
              value={generate.generation.quality}
              options={QUALITY_OPTIONS}
              onChange={(quality) => onChange(patchGeneration(workflow, { quality }))}
            />
          </label>
          <div className={styles.twoColumns}>
            <label className={styles.fieldLabel}>
              <span>宽度</span>
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
              <span>高度</span>
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
            <span>每条提示词生成数量</span>
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
              <h4>多图提示词规划</h4>
              <label className={styles.fieldLabel}>
                <span>提示词规划模型</span>
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
                <span>系列拆分要求</span>
                <Input.TextArea
                  value={promptPlanning?.instruction ?? ''}
                  maxLength={2_000}
                  autoSize={{ minRows: 3, maxRows: 6 }}
                  placeholder='说明每张图之间如何分工并保持连贯'
                  onChange={(instruction) =>
                    onChange(patchPromptPlanning(workflow, { instruction }))
                  }
                />
              </label>
              <label className={styles.fieldLabel}>
                <span>规划最大输出 Token</span>
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
                  <span>张数</span>
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
                  <span>并发</span>
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
                <span>生成前审核提示词</span>
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
