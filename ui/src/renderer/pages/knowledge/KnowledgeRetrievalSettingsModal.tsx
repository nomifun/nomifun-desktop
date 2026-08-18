/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { Message, Modal, Radio, Spin } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IKnowledgeRetrievalConfig } from '@/common/adapter/ipcBridge';
import TaskModelSelect, {
  type TaskModelSelection,
} from '@/renderer/components/model/TaskModelSelect';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import styles from './KnowledgeRetrievalSettingsModal.module.css';

type ModelChoice = Pick<TaskModelSelection, 'provider_id' | 'model'>;
type StageDraft =
  | { mode: 'local' }
  | { mode: 'remote'; model: ModelChoice | null };

type RetrievalDraft = {
  embedding: StageDraft;
  rerank: StageDraft;
};

const stageFromWire = (
  stage: IKnowledgeRetrievalConfig['embedding'] | IKnowledgeRetrievalConfig['rerank']
): StageDraft =>
  stage.mode === 'remote'
    ? { mode: 'remote', model: { provider_id: stage.provider_id, model: stage.model } }
    : { mode: 'local' };

export const retrievalDraftFromWire = (
  config: IKnowledgeRetrievalConfig
): RetrievalDraft => ({
  embedding: stageFromWire(config.embedding),
  rerank: stageFromWire(config.rerank),
});

const wireStage = (draft: StageDraft) => {
  if (draft.mode === 'local') return { mode: 'local' as const };
  if (!draft.model) throw new Error('remote retrieval stage requires a model');
  return {
    mode: 'remote' as const,
    provider_id: draft.model.provider_id,
    model: draft.model.model,
  };
};

export const retrievalWireFromDraft = (draft: RetrievalDraft): IKnowledgeRetrievalConfig => ({
  embedding: wireStage(draft.embedding),
  rerank: wireStage(draft.rerank),
});

const isLiveChoice = (
  groups: ReturnType<typeof useModelsForTask>['groups'],
  choice: ModelChoice | null
) =>
  choice != null &&
  groups.some(
    (group) =>
      group.provider.id === choice.provider_id && group.models.includes(choice.model)
  );

interface KnowledgeRetrievalSettingsModalProps {
  visible: boolean;
  onClose: () => void;
}

const KnowledgeRetrievalSettingsModal: React.FC<KnowledgeRetrievalSettingsModalProps> = ({
  visible,
  onClose,
}) => {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<RetrievalDraft | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const embeddingModels = useModelsForTask('embedding');
  const rerankModels = useModelsForTask('rerank');

  useEffect(() => {
    if (!visible) return;
    let active = true;
    setLoading(true);
    setDraft(null);
    void ipcBridge.knowledge.getRetrievalConfig
      .invoke()
      .then((config) => {
        if (active) setDraft(retrievalDraftFromWire(config));
      })
      .catch((error) => {
        if (active) Message.error(String(error));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [visible]);

  const invalidRemoteSelection = useMemo(() => {
    if (!draft) return true;
    const embeddingInvalid =
      draft.embedding.mode === 'remote' &&
      !isLiveChoice(embeddingModels.groups, draft.embedding.model);
    const rerankInvalid =
      draft.rerank.mode === 'remote' &&
      !isLiveChoice(rerankModels.groups, draft.rerank.model);
    return embeddingInvalid || rerankInvalid;
  }, [draft, embeddingModels.groups, rerankModels.groups]);

  const save = async () => {
    if (!draft || invalidRemoteSelection) return;
    setSaving(true);
    try {
      const saved = await ipcBridge.knowledge.setRetrievalConfig.invoke(
        retrievalWireFromDraft(draft)
      );
      setDraft(retrievalDraftFromWire(saved));
      Message.success(t('knowledge.retrieval.saved'));
      onClose();
    } catch (error) {
      Message.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  const renderMode = (stage: 'embedding' | 'rerank') => {
    if (!draft) return null;
    const value = draft[stage];
    return (
      <section className={styles.stageCard}>
        <div className={styles.stageHeader}>
          <div className={styles.stageCopy}>
            <div className={styles.stageTitle}>
              {t(`knowledge.retrieval.${stage}Title`)}
            </div>
            <div className={styles.stageHint}>
              {t(`knowledge.retrieval.${stage}Hint`)}
            </div>
          </div>
          <Radio.Group
            type='button'
            direction='vertical'
            size='small'
            className={styles.modeGroup}
            value={value.mode}
            onChange={(mode: 'local' | 'remote') =>
              setDraft((current) =>
                current
                  ? {
                      ...current,
                      [stage]: mode === 'local' ? { mode: 'local' } : { mode: 'remote', model: null },
                    }
                  : current
              )
            }
          >
            <Radio value='local'>{t('knowledge.retrieval.local')}</Radio>
            <Radio value='remote'>{t('knowledge.retrieval.remote')}</Radio>
          </Radio.Group>
        </div>
        {value.mode === 'remote' && (
          <div className={styles.modelSelector}>
            <TaskModelSelect
              task={stage}
              value={value.model}
              onChange={(model) =>
                setDraft((current) =>
                  current
                    ? {
                        ...current,
                        [stage]: {
                          mode: 'remote',
                          model: { provider_id: model.provider_id, model: model.model },
                        },
                      }
                    : current
                )
              }
              size='small'
              emptyHint={t(`knowledge.retrieval.${stage}Empty`)}
            />
          </div>
        )}
      </section>
    );
  };

  return (
    <Modal
      className={styles.modal}
      title={t('knowledge.retrieval.title')}
      visible={visible}
      confirmLoading={saving}
      okButtonProps={{
        disabled:
          loading ||
          !draft ||
          invalidRemoteSelection ||
          embeddingModels.isLoading ||
          rerankModels.isLoading,
      }}
      onOk={() => void save()}
      onCancel={onClose}
      autoFocus={false}
      unmountOnExit
    >
      <Spin loading={loading} className='w-full'>
        <div className={styles.modalBody}>
          <p className={styles.intro}>
            {t('knowledge.retrieval.description')}
          </p>
          {renderMode('embedding')}
          {renderMode('rerank')}
        </div>
      </Spin>
    </Modal>
  );
};

export default KnowledgeRetrievalSettingsModal;
