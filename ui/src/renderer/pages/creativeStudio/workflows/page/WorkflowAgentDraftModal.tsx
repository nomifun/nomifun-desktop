/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Input, Modal } from '@arco-design/web-react';
import { MagicWand } from '@icon-park/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';

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
import {
  createWorkflowTranslationCopy,
  workflowFallbackError,
} from '../workflowI18n';
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
  t?: TFunction;
}): Promise<GeneratedWorkflowAgentDraft> {
  const translate = input.t ?? ((key: string, options?: Record<string, unknown>) =>
    typeof options?.defaultValue === 'string' ? options.defaultValue : key);
  const prompt = input.prompt.trim();
  if (!prompt) {
    throw new Error(
      translate('creativeStudio.workflows.agent.error.promptRequired', {
        defaultValue: 'Describe the template you want to save first.',
      })
    );
  }
  if (input.catalog.status !== 'ready') {
    throw new Error(
      translate('creativeStudio.workflows.agent.error.catalogNotReady', {
        defaultValue: 'The model catalog is not ready.',
      })
    );
  }
  const exactModel = findCreativeModelOption(
    buildCreativeModelGroups(input.catalog.providers, CHAT_FILTER),
    input.model
  );
  if (!exactModel) {
    throw new Error(
      translate('creativeStudio.workflows.agent.error.modelUnavailable', {
        defaultValue: 'The selected model is unavailable.',
      })
    );
  }

  const result: WorkflowDraftPortResult = await input.port.draft({
    providerId: exactModel.providerId,
    model: exactModel.model,
    prompt,
  });
  const artifact = parseCreativeWorkflowDraftArtifact(result.text);
  if (!artifact) {
    throw new Error(
      translate('creativeStudio.workflows.agent.error.noArtifact', {
        defaultValue: 'Agent did not return an applicable template draft.',
      })
    );
  }
  const copy = createWorkflowTranslationCopy(input.t);
  return {
    artifact,
    workflow: convertCreativeWorkflowDraft(artifact, exactModel, copy),
    model: { providerId: exactModel.providerId, model: exactModel.model },
  };
}

export const WorkflowAgentDraftPreview: React.FC<{
  draft: GeneratedWorkflowAgentDraft | null;
}> = ({ draft }) => {
  const { t } = useTranslation();
  return (
    <section
      className={styles.preview}
      aria-label={t('creativeStudio.workflows.agent.previewAria', {
        defaultValue: 'Template draft preview',
      })}
    >
      <div className={styles.previewHeading}>
        <MagicWand theme='outline' size={17} fill='currentColor' />
        <strong>
          {t('creativeStudio.workflows.agent.previewLabel', {
            defaultValue: 'Draft preview',
          })}
        </strong>
      </div>
      {draft ? (
        <div className={styles.previewBody} data-workflow-agent-preview='ready'>
          <h3>{draft.workflow.metadata.name}</h3>
          <div className={styles.chips}>
            <span>
              {t(
                draft.artifact.draft.mode === 'single-image'
                  ? 'creativeStudio.workflows.agent.modeSingle'
                  : 'creativeStudio.workflows.agent.modeMulti',
                { defaultValue: draft.artifact.draft.mode === 'single-image' ? 'Single image' : 'Multi-image' }
              )}
            </span>
            <span>
              {draft.workflow.metadata.category ||
                t('creativeStudio.workflows.workspace.categoryFallback', {
                  defaultValue: 'Uncategorized',
                })}
            </span>
            <span>
              {t('creativeStudio.workflows.agent.private', { defaultValue: 'Private' })}
            </span>
          </div>
          <p>
            {draft.workflow.metadata.description ||
              t('creativeStudio.workflows.agent.noDescription', {
                defaultValue: 'No description',
              })}
          </p>
          <pre>{draft.artifact.draft.promptTemplate}</pre>
          <small>
            {t('creativeStudio.workflows.agent.appliedHint', {
              defaultValue: 'Review and save the draft manually in the editor.',
            })}
          </small>
        </div>
      ) : (
        <div className={styles.previewEmpty} data-workflow-agent-preview='empty'>
          {t('creativeStudio.workflows.agent.emptyPreview', {
            defaultValue:
              'Review the draft here after generation. It will not be saved or run automatically.',
          })}
        </div>
      )}
    </section>
  );
};

export interface WorkflowAgentDraftModalProps {
  visible: boolean;
  catalog: CreativeModelCatalogSnapshot;
  port: WorkflowDraftPort;
  onApply(workflow: WorkflowDefinitionV1): void;
  onClose(): void;
  onOpenModelSettings?(): void;
}

const errorText = (error: unknown, t: TFunction): string =>
  workflowFallbackError(
    error,
    t,
    'creativeStudio.workflows.agent.error.generic',
    'Failed to generate the template draft. Try again later.'
  );

const WorkflowAgentDraftModal: React.FC<WorkflowAgentDraftModalProps> = ({
  visible,
  catalog,
  port,
  onApply,
  onClose,
  onOpenModelSettings,
}) => {
  const { t } = useTranslation();
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
          t,
        })
      );
    } catch (cause) {
      setError(errorText(cause, t));
    } finally {
      setGenerating(false);
    }
  };

  return (
    <Modal
      visible={visible}
      title={t('creativeStudio.workflows.agent.title', {
        defaultValue: 'Create template with AI',
      })}
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
        <section
          className={styles.form}
          aria-label={t('creativeStudio.workflows.agent.requestAria', {
            defaultValue: 'Template draft requirements',
          })}
        >
          <label>
            <span>
              {t('creativeStudio.workflows.agent.requestLabel', {
                defaultValue: 'Template request',
              })}
            </span>
            <Input.TextArea
              value={prompt}
              maxLength={20_000}
              autoSize={{ minRows: 7, maxRows: 12 }}
              disabled={generating}
              placeholder={t('creativeStudio.workflows.agent.requestPlaceholder', {
                defaultValue:
                  'e.g. Create an e-commerce hero-image template with a fixed commercial photography style; only replace the product name and selling points.',
              })}
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
            label={t('creativeStudio.workflows.agent.chatModel', {
              defaultValue: 'Chat model',
            })}
            copy={{
              placeholder: t('creativeStudio.workflows.agent.modelPlaceholder', {
                defaultValue: 'Choose a model to generate the draft',
              }),
              noCompatibleModel: t('creativeStudio.workflows.agent.noCompatibleModel', {
                defaultValue: 'No enabled model supports the chat task.',
              }),
              configureModels: t('creativeStudio.workflows.agent.configureModels', {
                defaultValue: 'Open model settings',
              }),
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
            {t('creativeStudio.workflows.agent.note', {
              defaultValue:
                'The first release supports single-image and multi-image drafts with fixed variables. It will not save, run, or call image models automatically.',
            })}
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
            {generating
              ? t('creativeStudio.workflows.agent.generating', {
                  defaultValue: 'Generating draft…',
                })
              : t('creativeStudio.workflows.agent.generate', {
                  defaultValue: 'Generate template draft',
                })}
          </Button>
        </section>

        <WorkflowAgentDraftPreview draft={draft} />
      </div>
      <div className={styles.actions}>
        <Button disabled={generating} onClick={onClose}>
          {t('creativeStudio.workflows.agent.cancel', { defaultValue: 'Cancel' })}
        </Button>
        <Button
          type='primary'
          disabled={!draftMatchesSelection || generating}
          onClick={() => draftMatchesSelection && draft && onApply(draft.workflow)}
        >
          {t('creativeStudio.workflows.agent.apply', {
            defaultValue: 'Apply to editor',
          })}
        </Button>
      </div>
    </Modal>
  );
};

export default WorkflowAgentDraftModal;
