/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Spin } from '@arco-design/web-react';
import React, { useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  CreativeModelSelect,
  buildCreativeModelGroups,
  findCreativeModelOption,
  type CreativeModelFilter,
  type CreativeModelSelectionRef,
  useNomiCreativeModelCatalog,
} from '../models';
import {
  creativeProjectRepository,
  isCreativeProjectRepositoryError,
  type CreativeProjectRepository,
} from '../services/projectRepository';
import styles from './CreativeStudioHomePage.module.css';
import { creativeStudioCanvasProjectPath } from './routes';

export const CREATIVE_STUDIO_HOME_MODEL_FILTER = {
  capability: 'task',
  task: 'chat',
} as const satisfies CreativeModelFilter;

export const creativeStudioHomeProjectTitle = (prompt: string): string =>
  Array.from(prompt.trim()).slice(0, 24).join('');

export interface CreateCreativeStudioHomeProjectOptions {
  prompt: string;
  model: CreativeModelSelectionRef | null;
  repository: CreativeProjectRepository;
  navigate(path: string): void;
}

/** One request creates the project, persists its first Agent turn, then opens the canvas. */
export async function createCreativeStudioHomeProject({
  prompt,
  model,
  repository,
  navigate,
}: CreateCreativeStudioHomeProjectOptions) {
  const normalizedPrompt = prompt.trim();
  if (!normalizedPrompt || !model) return null;

  const created = await repository.create({
    title: creativeStudioHomeProjectTitle(normalizedPrompt),
    agentKickoff: {
      prompt: normalizedPrompt,
      model: {
        providerId: model.providerId,
        model: model.model,
      },
    },
  });
  navigate(creativeStudioCanvasProjectPath(created.projectId));
  return created;
}

export interface CreativeStudioHomeSurfaceProps {
  prompt: string;
  canSubmit: boolean;
  submitting: boolean;
  error: string | null;
  modelSelector: React.ReactNode;
  onPromptChange(prompt: string): void;
  onSubmit(): void;
}

/** Presentational boundary kept small so the launch interaction can be tested without live providers. */
export const CreativeStudioHomeSurface: React.FC<CreativeStudioHomeSurfaceProps> = ({
  prompt,
  canSubmit,
  submitting,
  error,
  modelSelector,
  onPromptChange,
  onSubmit,
}) => (
  <section className={styles.page} data-creative-studio-home>
    <form
      className={styles.composer}
      aria-label='新建画布'
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit();
      }}
    >
      <h1 className={styles.title}>今天想创作什么？</h1>
      <label className={styles.promptField}>
        <span className={styles.srOnly}>创作需求</span>
        <textarea
          value={prompt}
          maxLength={65_536}
          rows={6}
          disabled={submitting}
          placeholder='描述你想在画布上完成的内容…'
          onChange={(event) => onPromptChange(event.target.value)}
        />
      </label>
      <div className={styles.modelField}>{modelSelector}</div>
      {error ? (
        <p className={styles.error} role='alert'>
          {error}
        </p>
      ) : null}
      <button
        type='submit'
        className={styles.submit}
        disabled={!canSubmit}
        aria-busy={submitting}
      >
        {submitting ? <Spin size={14} /> : null}
        <span>{submitting ? '正在创建…' : '开始创作'}</span>
      </button>
    </form>
  </section>
);

const CreativeStudioHomePage: React.FC = () => {
  const navigate = useNavigate();
  const catalog = useNomiCreativeModelCatalog();
  const [prompt, setPrompt] = useState('');
  const [model, setModel] = useState<CreativeModelSelectionRef | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submitLock = useRef(false);
  const selectedModel = useMemo(
    () =>
      catalog.status === 'ready'
        ? findCreativeModelOption(
            buildCreativeModelGroups(catalog.providers, CREATIVE_STUDIO_HOME_MODEL_FILTER),
            model
          )
        : null,
    [catalog.providers, catalog.status, model]
  );
  const canSubmit = Boolean(prompt.trim()) && selectedModel !== null && !submitting;

  const submit = async () => {
    if (!canSubmit || submitLock.current) return;
    submitLock.current = true;
    setSubmitting(true);
    setError(null);
    try {
      await createCreativeStudioHomeProject({
        prompt,
        model: selectedModel,
        repository: creativeProjectRepository,
        navigate,
      });
    } catch (cause) {
      setError(
        isCreativeProjectRepositoryError(cause) && cause.message.trim()
          ? cause.message
          : '创建画布失败，请稍后重试。'
      );
    } finally {
      submitLock.current = false;
      setSubmitting(false);
    }
  };

  return (
    <CreativeStudioHomeSurface
      prompt={prompt}
      canSubmit={canSubmit}
      submitting={submitting}
      error={error}
      onPromptChange={(nextPrompt) => {
        setPrompt(nextPrompt);
        setError(null);
      }}
      onSubmit={() => void submit()}
      modelSelector={(
        <CreativeModelSelect
          catalog={catalog}
          filter={CREATIVE_STUDIO_HOME_MODEL_FILTER}
          value={model}
          disabled={submitting}
          label='对话模型'
          copy={{
            placeholder: '选择用于规划画布的模型',
            noCompatibleModel: '没有支持 chat 任务的已启用模型。',
            configureModels: '前往模型设置',
          }}
          onChange={(nextModel) => {
            setModel(nextModel);
            setError(null);
          }}
          onOpenModelSettings={() => navigate('/models?section=models')}
        />
      )}
    />
  );
};

export default CreativeStudioHomePage;
