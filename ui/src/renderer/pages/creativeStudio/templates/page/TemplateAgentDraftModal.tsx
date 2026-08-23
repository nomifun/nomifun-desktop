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
import type { CreativeTemplateDefinitionV1 } from '../domain';
import {
  parseCreativeTemplateDraftArtifact,
  type CreativeTemplateDraftArtifact,
} from '../agent/artifacts';
import { convertCreativeTemplateDraft } from '../agent/converter';
import type {
  TemplateDraftPort,
  TemplateDraftPortResult,
} from '../agent/draftPort';
import {
  createTemplateTranslationCopy,
  templateFallbackError,
} from '../templateI18n';
import styles from './TemplateAgentDraftModal.module.css';

const CHAT_FILTER = { capability: 'task', task: 'chat' } as const;

export interface GeneratedTemplateAgentDraft {
  artifact: CreativeTemplateDraftArtifact;
  template: CreativeTemplateDefinitionV1;
  model: CreativeModelSelectionRef;
}

export async function generateTemplateAgentDraft(input: {
  prompt: string;
  model: CreativeModelOption;
  catalog: CreativeModelCatalogSnapshot;
  port: TemplateDraftPort;
  t?: TFunction;
}): Promise<GeneratedTemplateAgentDraft> {
  const translate = input.t ?? ((key: string, options?: Record<string, unknown>) =>
    typeof options?.defaultValue === 'string' ? options.defaultValue : key);
  const prompt = input.prompt.trim();
  if (!prompt) {
    throw new Error(
      translate('creativeStudio.templates.agent.error.promptRequired', {
        defaultValue: 'Describe the template you want to save first.',
      })
    );
  }
  if (input.catalog.status !== 'ready') {
    throw new Error(
      translate('creativeStudio.templates.agent.error.catalogNotReady', {
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
      translate('creativeStudio.templates.agent.error.modelUnavailable', {
        defaultValue: 'The selected model is unavailable.',
      })
    );
  }

  const result: TemplateDraftPortResult = await input.port.draft({
    providerId: exactModel.providerId,
    model: exactModel.model,
    prompt,
  });
  const artifact = parseCreativeTemplateDraftArtifact(result.text);
  if (!artifact) {
    throw new Error(
      translate('creativeStudio.templates.agent.error.noArtifact', {
        defaultValue: 'Agent did not return an applicable template draft.',
      })
    );
  }
  const copy = createTemplateTranslationCopy(input.t);
  return {
    artifact,
    template: convertCreativeTemplateDraft(artifact, exactModel, copy),
    model: { providerId: exactModel.providerId, model: exactModel.model },
  };
}

export const TemplateAgentDraftPreview: React.FC<{
  draft: GeneratedTemplateAgentDraft | null;
}> = ({ draft }) => {
  const { t } = useTranslation();
  return (
    <section
      className={styles.preview}
      aria-label={t('creativeStudio.templates.agent.previewAria', {
        defaultValue: 'Template draft preview',
      })}
    >
      <div className={styles.previewHeading}>
        <MagicWand theme='outline' size={17} fill='currentColor' />
        <strong>
          {t('creativeStudio.templates.agent.previewLabel', {
            defaultValue: 'Draft preview',
          })}
        </strong>
      </div>
      {draft ? (
        <div className={styles.previewBody} data-template-agent-preview='ready'>
          <h3>{draft.template.metadata.name}</h3>
          <div className={styles.chips}>
            <span>
              {t(
                draft.artifact.draft.mode === 'single-image'
                  ? 'creativeStudio.templates.agent.modeSingle'
                  : 'creativeStudio.templates.agent.modeMulti',
                { defaultValue: draft.artifact.draft.mode === 'single-image' ? 'Single image' : 'Multi-image' }
              )}
            </span>
            <span>
              {draft.template.metadata.category ||
                t('creativeStudio.templates.workspace.categoryFallback', {
                  defaultValue: 'Uncategorized',
                })}
            </span>
            <span>
              {t('creativeStudio.templates.agent.private', { defaultValue: 'Private' })}
            </span>
          </div>
          <p>
            {draft.template.metadata.description ||
              t('creativeStudio.templates.agent.noDescription', {
                defaultValue: 'No description',
              })}
          </p>
          <pre>{draft.artifact.draft.promptTemplate}</pre>
          <small>
            {t('creativeStudio.templates.agent.appliedHint', {
              defaultValue: 'Review and save the draft manually in the editor.',
            })}
          </small>
        </div>
      ) : (
        <div className={styles.previewEmpty} data-template-agent-preview='empty'>
          {t('creativeStudio.templates.agent.emptyPreview', {
            defaultValue:
              'Review the draft here after generation. It will not be saved or run automatically.',
          })}
        </div>
      )}
    </section>
  );
};

export interface TemplateAgentDraftModalProps {
  visible: boolean;
  catalog: CreativeModelCatalogSnapshot;
  port: TemplateDraftPort;
  onApply(template: CreativeTemplateDefinitionV1): void;
  onClose(): void;
  onOpenModelSettings?(): void;
}

const errorText = (error: unknown, t: TFunction): string =>
  templateFallbackError(
    error,
    t,
    'creativeStudio.templates.agent.error.generic',
    'Failed to generate the template draft. Try again later.'
  );

const TemplateAgentDraftModal: React.FC<TemplateAgentDraftModalProps> = ({
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
  const [draft, setDraft] = useState<GeneratedTemplateAgentDraft | null>(null);
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
        await generateTemplateAgentDraft({
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
      title={t('creativeStudio.templates.agent.title', {
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
        data-template-agent-draft-modal
      >
        <section
          className={styles.form}
          aria-label={t('creativeStudio.templates.agent.requestAria', {
            defaultValue: 'Template draft requirements',
          })}
        >
          <label>
            <span>
              {t('creativeStudio.templates.agent.requestLabel', {
                defaultValue: 'Template request',
              })}
            </span>
            <Input.TextArea
              value={prompt}
              maxLength={20_000}
              autoSize={{ minRows: 7, maxRows: 12 }}
              disabled={generating}
              placeholder={t('creativeStudio.templates.agent.requestPlaceholder', {
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
            label={t('creativeStudio.templates.agent.chatModel', {
              defaultValue: 'Chat model',
            })}
            copy={{
              placeholder: t('creativeStudio.templates.agent.modelPlaceholder', {
                defaultValue: 'Choose a model to generate the draft',
              }),
              noCompatibleModel: t('creativeStudio.templates.agent.noCompatibleModel', {
                defaultValue: 'No enabled model supports the chat task.',
              }),
              configureModels: t('creativeStudio.templates.agent.configureModels', {
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
            {t('creativeStudio.templates.agent.note', {
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
              ? t('creativeStudio.templates.agent.generating', {
                  defaultValue: 'Generating draft…',
                })
              : t('creativeStudio.templates.agent.generate', {
                  defaultValue: 'Generate template draft',
                })}
          </Button>
        </section>

        <TemplateAgentDraftPreview draft={draft} />
      </div>
      <div className={styles.actions}>
        <Button disabled={generating} onClick={onClose}>
          {t('creativeStudio.templates.agent.cancel', { defaultValue: 'Cancel' })}
        </Button>
        <Button
          type='primary'
          disabled={!draftMatchesSelection || generating}
          onClick={() => draftMatchesSelection && draft && onApply(draft.template)}
        >
          {t('creativeStudio.templates.agent.apply', {
            defaultValue: 'Apply to editor',
          })}
        </Button>
      </div>
    </Modal>
  );
};

export default TemplateAgentDraftModal;
