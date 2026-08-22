/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Input, InputNumber, Message, Modal, Select, Switch } from '@arco-design/web-react';
import { Copy, Play } from '@icon-park/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';

import {
  renderWorkflowTemplate,
  validateWorkflowInputsForDefinition,
  type WorkflowDefinitionV1,
  type WorkflowInputValue,
  type WorkflowVariable,
} from '../domain';
import styles from './CreativeWorkflowWorkspacePage.module.css';
import { draftPromptsStep, generationStep } from './workflowViewModel';

export interface CreativeWorkflowRunRequest {
  workflow: WorkflowDefinitionV1;
  inputs: WorkflowInputValue[];
  referenceAssetIds: string[];
}

export interface CreativeWorkflowRunnerPort {
  start(request: CreativeWorkflowRunRequest): Promise<void>;
}

export interface WorkflowRunModalProps {
  workflow: WorkflowDefinitionV1 | null;
  runner?: CreativeWorkflowRunnerPort;
  onClose: () => void;
  onPickAssets?: (
    variable: WorkflowVariable,
    selectedAssetIds: readonly string[]
  ) => Promise<string[] | null>;
  onPickReferenceAssets?: (selectedAssetIds: readonly string[]) => Promise<string[] | null>;
  onUploadReferenceImages?: (
    files: readonly File[],
    selectedAssetIds: readonly string[]
  ) => Promise<string[]>;
}

function initialInput(variable: WorkflowVariable): WorkflowInputValue {
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
  inputs: WorkflowInputValue[],
  replacement: WorkflowInputValue
): WorkflowInputValue[] {
  return inputs.map((input) =>
    input.variableId === replacement.variableId ? replacement : input
  );
}

const WorkflowInputControl: React.FC<{
  variable: WorkflowVariable;
  input: WorkflowInputValue;
  disabled: boolean;
  onChange: (input: WorkflowInputValue) => void;
  onPickAssets?: (
    variable: WorkflowVariable,
    selectedAssetIds: readonly string[]
  ) => Promise<string[] | null>;
}> = ({ variable, input, disabled, onChange, onPickAssets }) => {
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
        placeholder={variable.options.length > 0 ? '请选择' : '未配置选项'}
        options={variable.options.map((option) => ({ value: option, label: option }))}
        disabled={disabled || variable.options.length === 0}
        onChange={(value) => onChange({ ...input, value })}
      />
    );
  }
  if (variable.type === 'image' && input.type === 'image') {
    return (
      <div className={styles.referencePlaceholder}>
        <p>{input.assetId ? `已选择素材 ${input.assetId}` : '未选择参考图'}</p>
        <Button
          size='small'
          disabled={disabled || !onPickAssets}
          title={onPickAssets ? undefined : '素材选择器尚未连接'}
          onClick={() =>
            void onPickAssets?.(variable, input.assetId ? [input.assetId] : [])
              .then((assetIds) => {
                if (assetIds) onChange({ ...input, assetId: assetIds[0] ?? null });
              })
              .catch((error) => Message.error(
                error instanceof Error ? error.message : '素材选择器打开失败'
              ))
          }
        >
          从我的素材选择
        </Button>
      </div>
    );
  }
  if (variable.type === 'image-series' && input.type === 'image-series') {
    return (
      <div className={styles.referencePlaceholder}>
        <p>{input.assetIds.length > 0 ? `已选择 ${input.assetIds.length} 张图片` : '未选择参考图'}</p>
        <Button
          size='small'
          disabled={disabled || !onPickAssets}
          title={onPickAssets ? undefined : '素材选择器尚未连接'}
          onClick={() =>
            void onPickAssets?.(variable, input.assetIds)
              .then((assetIds) => {
                if (assetIds) onChange({ ...input, assetIds });
              })
              .catch((error) => Message.error(
                error instanceof Error ? error.message : '素材选择器打开失败'
              ))
          }
        >
          从我的素材选择
        </Button>
      </div>
    );
  }
  return <div className={styles.referencePlaceholder}>变量契约不匹配，请重新打开工作流。</div>;
};

const WorkflowRunModal: React.FC<WorkflowRunModalProps> = ({
  workflow,
  runner,
  onClose,
  onPickAssets,
  onPickReferenceAssets,
  onUploadReferenceImages,
}) => {
  const [inputs, setInputs] = useState<WorkflowInputValue[]>([]);
  const [referenceAssetIds, setReferenceAssetIds] = useState<string[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const referenceInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setInputs(workflow?.variables.map(initialInput) ?? []);
    setReferenceAssetIds([]);
    setSubmitting(false);
  }, [workflow]);

  const validation = useMemo(
    () =>
      workflow
        ? validateWorkflowInputsForDefinition(workflow, inputs)
        : ({
            ok: false,
            error: {
              code: 'invalid-value',
              path: '$.workflow',
              message: 'workflow is unavailable',
            },
          } as const),
    [inputs, workflow]
  );
  const prompt = useMemo(() => {
    if (!workflow) return { ok: false as const, value: '' };
    const template = workflow.templates[0];
    if (!template) return { ok: false as const, value: '' };
    const result = renderWorkflowTemplate(workflow, template.id, inputs);
    return result.ok
      ? { ok: true as const, value: result.value }
      : { ok: false as const, value: result.error.message };
  }, [inputs, workflow]);
  const promptPreview = useMemo(() => {
    if (!workflow) return '';
    const values = new Map(inputs.map((input) => [input.variableId, input]));
    const variables = new Map(workflow.variables.map((variable) => [variable.id, variable]));
    return (workflow.templates[0]?.segments ?? [])
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
  }, [inputs, workflow]);

  if (!workflow) return null;
  const generate = generationStep(workflow);
  const model = generate.generation.model;
  const planningModel = draftPromptsStep(workflow)?.planning.model ?? null;
  const requiresPlanningModel = workflow.output.kind === 'multi-image-series';
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
        workflow,
        inputs,
        referenceAssetIds,
      });
      Message.success('工作流任务已提交');
      onClose();
    } catch (error) {
      Message.error(error instanceof Error ? error.message : '工作流任务提交失败');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Modal
      visible
      alignCenter={false}
      className={styles.runModal}
      title={workflow.metadata.name || '运行工作流'}
      footer={null}
      autoFocus={false}
      unmountOnExit
      getPopupContainer={() =>
        document.getElementById('creative-studio-portal-root') ?? document.body
      }
      onCancel={onClose}
    >
      <div className={styles.runGrid} data-workflow-runner>
        <div className={styles.runColumn}>
          <section className={styles.runSection}>
            <h3>变量输入</h3>
            <div className={styles.inputList}>
              {workflow.variables.map((variable) => {
                const input = inputs.find((candidate) => candidate.variableId === variable.id);
                if (!input) return null;
                return (
                  <label key={variable.id} className={styles.runField}>
                    <span>
                      {variable.label || variable.key}
                      {variable.required ? <span className={styles.required}>*</span> : null}
                    </span>
                    <WorkflowInputControl
                      variable={variable}
                      input={input}
                      disabled={submitting}
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
              <h3>参考图</h3>
              <div className={styles.referenceActions}>
                <Button
                  size='small'
                  disabled={submitting || !onPickReferenceAssets}
                  title={onPickReferenceAssets ? undefined : '素材选择器尚未连接'}
                  onClick={() =>
                    void onPickReferenceAssets?.(referenceAssetIds)
                      .then((assetIds) => {
                        if (assetIds) setReferenceAssetIds(assetIds);
                      })
                      .catch((error) => Message.error(
                        error instanceof Error ? error.message : '素材选择器打开失败'
                      ))
                  }
                >
                  我的素材
                </Button>
                <Button
                  size='small'
                  disabled={submitting || !onUploadReferenceImages}
                  title={onUploadReferenceImages ? undefined : '图片上传网关尚未连接'}
                  onClick={() => referenceInputRef.current?.click()}
                >
                  上传
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
                    error instanceof Error ? error.message : '参考图上传失败'
                  ));
              }}
            />
            <div className={styles.referencePlaceholder}>
              {referenceAssetIds.length > 0
                ? `已添加 ${referenceAssetIds.length} 张参考图`
                : '未添加参考图'}
            </div>
          </section>

          {!runner ? (
            <div className={styles.runnerUnavailable} role='status'>
              运行网关正在接入 NomiFun 任务系统；当前不会伪造生成结果。
            </div>
          ) : !model ? (
            <div className={styles.runnerUnavailable} role='status'>
              请先编辑工作流并选择支持当前任务的已启用模型。
            </div>
          ) : requiresPlanningModel && !planningModel ? (
            <div className={styles.runnerUnavailable} role='status'>
              请先为多图提示词规划选择一个已启用的对话模型。
            </div>
          ) : !validation.ok ? (
            <div className={styles.runnerUnavailable} role='status'>
              {validation.error.message}
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
            {workflow.output.kind === 'multi-image-series' ? '生成提示词' : '启动任务'}
          </Button>
        </div>

        <div className={styles.runColumn}>
          <section className={styles.runSection}>
            <div className={styles.sectionHeadingRow}>
              <h3>生成提示词预览</h3>
              <Button
                size='small'
                icon={<Copy theme='outline' size={14} fill='currentColor' />}
                disabled={!promptPreview}
                onClick={() => promptPreview && void navigator.clipboard.writeText(promptPreview)}
              >
                复制
              </Button>
            </div>
            <div className={styles.promptResult}>
              {promptPreview || '填写变量后会在这里预览最终提示词'}
            </div>
          </section>

          <div className={styles.infoGrid}>
            <div className={styles.infoPill}>
              <p>模型</p>
              <strong>{model?.model ?? '未选择'}</strong>
            </div>
            <div className={styles.infoPill}>
              <p>任务</p>
              <strong>{model?.task === 'image_edit' ? '图像编辑' : '图片生成'}</strong>
            </div>
            <div className={styles.infoPill}>
              <p>尺寸</p>
              <strong>
                {generate.generation.width} × {generate.generation.height}
              </strong>
            </div>
            <div className={styles.infoPill}>
              <p>{workflow.output.kind === 'multi-image-series' ? '草稿数量' : '数量'}</p>
              <strong>
                {workflow.output.kind === 'multi-image-series'
                  ? workflow.output.targetCount
                  : generate.generation.imagesPerPrompt}{' '}
                张
              </strong>
            </div>
          </div>
        </div>
      </div>
    </Modal>
  );
};

export default WorkflowRunModal;
