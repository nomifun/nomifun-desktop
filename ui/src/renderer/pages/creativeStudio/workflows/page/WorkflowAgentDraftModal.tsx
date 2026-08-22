/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Input, Modal } from '@arco-design/web-react';
import { MagicWand } from '@icon-park/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';

import {
  CreativeModelSelect,
  buildCreativeModelGroups,
  findCreativeModelOption,
  type CreativeModelCatalogSnapshot,
  type CreativeModelOption,
  type CreativeModelSelectionRef,
} from '../../models';
import type { WorkflowDefinitionV1 } from '../domain';
import {
  parseCreativeWorkflowDraftArtifact,
  type CreativeWorkflowDraftArtifact,
} from '../agent/artifacts';
import { convertCreativeWorkflowDraft } from '../agent/converter';
import type {
  WorkflowDraftPort,
  WorkflowDraftPortResult,
} from '../agent/draftPort';
import styles from './WorkflowAgentDraftModal.module.css';

const CHAT_FILTER = { capability: 'task', task: 'chat' } as const;

export interface GeneratedWorkflowAgentDraft {
  artifact: CreativeWorkflowDraftArtifact;
  workflow: WorkflowDefinitionV1;
  model: CreativeModelSelectionRef;
}

export async function generateWorkflowAgentDraft(input: {
  prompt: string;
  model: CreativeModelOption;
  catalog: CreativeModelCatalogSnapshot;
  port: WorkflowDraftPort;
}): Promise<GeneratedWorkflowAgentDraft> {
  const prompt = input.prompt.trim();
  if (!prompt) throw new Error('请先描述要沉淀的工作流。');
  if (input.catalog.status !== 'ready') throw new Error('模型目录尚未就绪。');
  const exactModel = findCreativeModelOption(
    buildCreativeModelGroups(input.catalog.providers, CHAT_FILTER),
    input.model
  );
  if (!exactModel) throw new Error('所选模型已不可用。');

  const result: WorkflowDraftPortResult = await input.port.draft({
    providerId: exactModel.providerId,
    model: exactModel.model,
    prompt,
  });
  const artifact = parseCreativeWorkflowDraftArtifact(result.text);
  if (!artifact) {
    throw new Error('Agent 未返回可应用的 Workflow 草稿。');
  }
  return {
    artifact,
    workflow: convertCreativeWorkflowDraft(artifact, exactModel),
    model: { providerId: exactModel.providerId, model: exactModel.model },
  };
}

export const WorkflowAgentDraftPreview: React.FC<{
  draft: GeneratedWorkflowAgentDraft | null;
}> = ({ draft }) => (
  <section className={styles.preview} aria-label='Workflow 草稿预览'>
    <div className={styles.previewHeading}>
      <MagicWand theme='outline' size={17} fill='currentColor' />
      <strong>草稿预览</strong>
    </div>
    {draft ? (
      <div className={styles.previewBody} data-workflow-agent-preview='ready'>
        <h3>{draft.workflow.metadata.name}</h3>
        <div className={styles.chips}>
          <span>{draft.artifact.draft.mode === 'single-image' ? '单图' : '多图'}</span>
          <span>{draft.workflow.metadata.category || '未分类'}</span>
          <span>个人</span>
        </div>
        <p>{draft.workflow.metadata.description || '暂无描述'}</p>
        <pre>{draft.artifact.draft.promptTemplate}</pre>
        <small>应用后仍需在编辑器中手动检查并保存。</small>
      </div>
    ) : (
      <div className={styles.previewEmpty} data-workflow-agent-preview='empty'>
        生成后在这里检查草稿，不会自动保存或运行。
      </div>
    )}
  </section>
);

export interface WorkflowAgentDraftModalProps {
  visible: boolean;
  catalog: CreativeModelCatalogSnapshot;
  port: WorkflowDraftPort;
  onApply(workflow: WorkflowDefinitionV1): void;
  onClose(): void;
  onOpenModelSettings?(): void;
}

const errorText = (error: unknown): string =>
  error instanceof Error && error.message.trim()
    ? error.message
    : 'Workflow 草稿生成失败，请稍后重试。';

const WorkflowAgentDraftModal: React.FC<WorkflowAgentDraftModalProps> = ({
  visible,
  catalog,
  port,
  onApply,
  onClose,
  onOpenModelSettings,
}) => {
  const [prompt, setPrompt] = useState('');
  const [model, setModel] = useState<CreativeModelSelectionRef | null>(null);
  const [generating, setGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState<GeneratedWorkflowAgentDraft | null>(null);
  const modalContentRef = useRef<HTMLDivElement>(null);
  const groups = useMemo(
    () => buildCreativeModelGroups(catalog.providers, CHAT_FILTER),
    [catalog.providers]
  );
  const selectedModel = useMemo(
    () => catalog.status === 'ready' ? findCreativeModelOption(groups, model) : null,
    [catalog.status, groups, model]
  );
  const selectedModelKey = selectedModel
    ? `${selectedModel.providerId}\u0000${selectedModel.model}`
    : '';
  const draftMatchesSelection = Boolean(
    draft &&
      selectedModel &&
      draft.model.providerId === selectedModel.providerId &&
      draft.model.model === selectedModel.model
  );

  useEffect(() => {
    if (visible) return;
    setPrompt('');
    setModel(null);
    setGenerating(false);
    setError(null);
    setDraft(null);
  }, [visible]);

  useEffect(() => {
    setDraft(null);
    setError(null);
  }, [catalog.status, selectedModelKey]);

  const generate = async () => {
    if (generating || !prompt.trim() || !selectedModel) return;
    setGenerating(true);
    setError(null);
    setDraft(null);
    try {
      setDraft(
        await generateWorkflowAgentDraft({
          prompt,
          model: selectedModel,
          catalog,
          port,
        })
      );
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setGenerating(false);
    }
  };

  return (
    <Modal
      visible={visible}
      title='AI 创建工作流'
      className={styles.modal}
      style={{ width: 880, maxWidth: 'calc(100vw - 32px)' }}
      footer={null}
      autoFocus={false}
      unmountOnExit
      maskClosable={!generating}
      closable={!generating}
      getPopupContainer={() =>
        document.getElementById('creative-studio-portal-root') ?? document.body
      }
      onCancel={() => !generating && onClose()}
    >
      <div
        ref={modalContentRef}
        className={styles.layout}
        data-workflow-agent-draft-modal
      >
        <section className={styles.form} aria-label='Workflow 草稿需求'>
          <label>
            <span>工作流需求</span>
            <Input.TextArea
              value={prompt}
              maxLength={20_000}
              autoSize={{ minRows: 7, maxRows: 12 }}
              disabled={generating}
              placeholder='例如：创建一个电商主图工作流，固定商业摄影风格，只替换产品名称和卖点。'
              onChange={(value) => {
                setPrompt(value);
                setDraft(null);
                setError(null);
              }}
            />
          </label>
          <CreativeModelSelect
            catalog={catalog}
            filter={CHAT_FILTER}
            value={model}
            disabled={generating}
            label='对话模型'
            copy={{
              placeholder: '选择生成草稿的模型',
              noCompatibleModel: '没有支持 chat 任务的已启用模型。',
              configureModels: '前往模型设置',
            }}
            onChange={(next) => {
              setModel(next);
              setDraft(null);
              setError(null);
            }}
            onOpenModelSettings={onOpenModelSettings}
            getPopupContainer={() =>
              modalContentRef.current ??
              document.getElementById('creative-studio-portal-root') ??
              document.body
            }
          />
          <p className={styles.note}>
            首批仅支持固定变量的单图/多图草稿；不会自动保存、运行或调用图片模型。
          </p>
          {error ? <p className={styles.error} role='alert'>{error}</p> : null}
          <Button
            type='primary'
            long
            disabled={!prompt.trim() || !selectedModel || generating}
            loading={generating}
            icon={generating ? undefined : <MagicWand theme='outline' size={15} />}
            onClick={() => void generate()}
          >
            {generating ? '正在生成草稿…' : '生成工作流草稿'}
          </Button>
        </section>

        <WorkflowAgentDraftPreview draft={draft} />
      </div>
      <div className={styles.actions}>
        <Button disabled={generating} onClick={onClose}>取消</Button>
        <Button
          type='primary'
          disabled={!draftMatchesSelection || generating}
          onClick={() => draftMatchesSelection && draft && onApply(draft.workflow)}
        >
          应用到编辑器
        </Button>
      </div>
    </Modal>
  );
};

export default WorkflowAgentDraftModal;
