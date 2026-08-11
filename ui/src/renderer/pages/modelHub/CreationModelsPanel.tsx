/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import classNames from 'classnames';
import { Button, Tag } from '@arco-design/web-react';
import { LinkCloud, MagicWand, Pic, VideoTwo } from '@icon-park/react';
import { configService } from '@/common/config/configService';
import type { ConfigKeyMap } from '@/common/config/configKeys';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { NomiSettingList, NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import {
  type CreationCapability,
  filterCreationModels,
  groupCreationModelsByProvider,
  useCreationModels,
} from './creationModels';
import { SerializedLatestWriteQueue } from './serializedLatestWriteQueue';

export interface CreationModelsPanelProps {
  /** The one generation capability this section lists. */
  capability: CreationCapability;
  titleKey: I18nKey;
  subtitleKey: I18nKey;
  /** Optional install-wide default owned by this model task. */
  defaultModelPreferenceKey?: 'models.default.imageGeneration';
}

type ImageGenerationDefaultModel = NonNullable<
  ConfigKeyMap['models.default.imageGeneration']
>;

interface ImageGenerationDefaultControlProps {
  preferenceKey: 'models.default.imageGeneration';
}

/**
 * The native Agent image tool's optional install-wide default. Candidate
 * membership is resolved for the exact `image_generation` task: an edit-only
 * model shown elsewhere in this section is not silently promoted to a generator.
 * Stale saved references stay visible in the shared selector until the user
 * explicitly replaces or clears them.
 */
const ImageGenerationDefaultControl: React.FC<ImageGenerationDefaultControlProps> = ({
  preferenceKey,
}) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage({ maxCount: 1 });
  const { groups, isLoading } = useModelsForTask('image_generation');
  const [defaultModel, setDefaultModel] = useState<ImageGenerationDefaultModel | null>(
    () => configService.get(preferenceKey) ?? null
  );
  const [isSavingDefault, setIsSavingDefault] = useState(false);
  const writeQueueRef = useRef(new SerializedLatestWriteQueue());

  useEffect(() => {
    let active = true;
    const sync = () => {
      if (active && !writeQueueRef.current.hasPending) {
        setDefaultModel(configService.get(preferenceKey) ?? null);
      }
    };
    const unsubscribe = configService.subscribe(preferenceKey, (value) => {
      if (active && !writeQueueRef.current.hasPending) {
        setDefaultModel((value as ImageGenerationDefaultModel | undefined) ?? null);
      }
    });
    void configService.whenReady().then(sync);
    return () => {
      active = false;
      unsubscribe();
    };
  }, [preferenceKey]);

  const persistDefault = useCallback(
    async (next: ImageGenerationDefaultModel | null) => {
      setDefaultModel(next);
      setIsSavingDefault(true);

      const queue = writeQueueRef.current;
      const { done } = queue.enqueue(
        () =>
          next
            ? configService.set(preferenceKey, next)
            : configService.remove(preferenceKey),
        {
          onLatestError: async (error, generation) => {
            await configService.reload();
            if (!queue.isLatest(generation)) return;
            setDefaultModel(configService.get(preferenceKey) ?? null);
            console.error('[CreationModels] Failed to save the default image model:', error);
            message.error(t('settings.modelHub.creation.defaultSaveFailed'));
          },
          onLatestSettled: (generation) => {
            if (queue.isLatest(generation)) setIsSavingDefault(false);
          },
        }
      );
      await done;
    },
    [message, preferenceKey, t]
  );

  const hasCandidates = groups.some((group) => group.models.length > 0);
  const noCandidates = !isLoading && !hasCandidates;
  const description = isLoading
    ? t('settings.modelHub.creation.defaultLoading')
    : noCandidates
      ? t('settings.modelHub.creation.defaultNoModels')
      : defaultModel
        ? t('settings.modelHub.creation.defaultHint')
        : t('settings.modelHub.creation.defaultUnset');

  return (
    <div className='mb-14px'>
      {messageContext}
      <NomiSettingList>
        <NomiSettingRow
          title={t('settings.modelHub.creation.defaultTitle')}
          description={description}
          controls={
            <div className='flex min-w-0 flex-wrap items-center justify-end gap-8px'>
              <TaskModelSelect
                task='image_generation'
                size='small'
                disabled={noCandidates || isSavingDefault}
                value={defaultModel}
                emptyHint={t('settings.modelHub.creation.defaultNoModels')}
                onChange={({ provider_id, model }) =>
                  void persistDefault({ provider_id, model })
                }
              />
              {defaultModel && (
                <Button
                  size='mini'
                  disabled={isSavingDefault}
                  onClick={() => void persistDefault(null)}
                >
                  {t('settings.modelHub.creation.defaultClear')}
                </Button>
              )}
            </div>
          }
        />
      </NomiSettingList>
    </div>
  );
};

/** Per-capability visual language (chip icon + accent), consistent light/dark. */
const CAP_META: Record<CreationCapability, { icon: React.ReactNode; color: string }> = {
  image_generation: { icon: <Pic theme='outline' size='12' strokeWidth={3} />, color: 'magenta' },
  video_generation: { icon: <VideoTwo theme='outline' size='12' strokeWidth={3} />, color: 'purple' },
};

const CAP_HEADER_ICON: Record<CreationCapability, React.ReactNode> = {
  image_generation: <Pic theme='outline' size='18' strokeWidth={3} />,
  video_generation: <VideoTwo theme='outline' size='18' strokeWidth={3} />,
};

const CAP_LABEL_KEY: Record<CreationCapability, I18nKey> = {
  image_generation: 'settings.modelHub.creation.capImage',
  video_generation: 'settings.modelHub.creation.capVideo',
};

/**
 * One generation capability's section of Model Management (图像生成 / 视频生成).
 * Lists the models that can produce that medium across configured providers,
 * grouped by provider. Capability comes from the authoritative catalog
 * resolution (per-model task tags; `image_edit` folds into image generation) —
 * this is a read-only view, tagging lives on the 供应商与密钥 page.
 *
 * A row is tagged only with the OTHER capabilities it also carries: the section
 * already states the capability every row shares, so repeating it on each row is
 * noise, while "this image model also does video" is not.
 */
const CreationModelsPanel: React.FC<CreationModelsPanelProps> = ({
  capability,
  titleKey,
  subtitleKey,
  defaultModelPreferenceKey,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { data } = useProvidersQuery();
  const { entries } = useCreationModels();

  const groups = useMemo(
    () => groupCreationModelsByProvider(filterCreationModels(entries, capability)),
    [entries, capability]
  );

  const providersWithModels = (data ?? []).filter((p) => p.enabled !== false && (p.models ?? []).length > 0).length;

  return (
    <div className='flex flex-col bg-2 rd-16px px-24px py-16px'>
      {/* Header */}
      <div className='flex-shrink-0 border-b border-b-solid border-[var(--color-border-2)] pb-12px mb-14px flex flex-col gap-10px'>
        <div className='flex items-center gap-9px'>
          <span className='size-30px flex items-center justify-center rd-9px bg-primary-1 text-primary-6 shrink-0'>
            {CAP_HEADER_ICON[capability]}
          </span>
          <div className='min-w-0'>
            <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>{t(titleKey)}</h2>
            <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>{t(subtitleKey)}</p>
          </div>
        </div>
        {/* Capability comes from per-model task tags; link to where they are edited. */}
        <div
          className='rd-8px px-12px py-8px text-12px leading-5 border border-solid flex items-center justify-between gap-8px flex-wrap'
          style={{
            borderColor: 'rgba(var(--primary-6),0.32)',
            backgroundColor: 'rgba(var(--primary-6),0.08)',
            color: 'rgb(var(--primary-6))',
          }}
        >
          <span className='min-w-0'>{t('settings.modelHub.creation.note')}</span>
          <Button
            type='text'
            size='mini'
            icon={<LinkCloud theme='outline' size='12' />}
            onClick={() => navigate('/models?section=models')}
          >
            {t('settings.modelHub.creation.manageModels')}
          </Button>
        </div>
      </div>

      {defaultModelPreferenceKey && (
        <ImageGenerationDefaultControl preferenceKey={defaultModelPreferenceKey} />
      )}

      {/* Content */}
      <NomiScrollArea className='flex-1 min-h-0' disableOverflow>
        {groups.length === 0 ? (
          <div className='flex flex-col items-center justify-center py-48px text-center'>
            <MagicWand theme='outline' size='44' className='text-t-tertiary mb-14px' />
            <h3 className='text-16px font-500 text-t-primary mb-6px'>{t('settings.modelHub.creation.empty')}</h3>
            <p className='text-13px text-t-secondary max-w-420px leading-20px'>
              {providersWithModels === 0
                ? t('settings.modelHub.creation.emptyNoProviders')
                : t('settings.modelHub.creation.emptyHint')}
            </p>
          </div>
        ) : (
          <div className='space-y-12px'>
            {groups.map((group) => (
              <div
                key={group.providerId}
                className='rd-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] overflow-hidden'
              >
                {/* Group header */}
                <div className='flex items-center justify-between gap-8px px-14px py-10px bg-[var(--fill-0)] border-b border-b-solid border-[var(--color-border-2)] flex-wrap'>
                  <div className='flex items-center gap-8px min-w-0'>
                    <span className='text-14px font-600 text-t-primary truncate'>{group.providerName}</span>
                    <span className='text-11px text-t-tertiary shrink-0'>{group.platform}</span>
                    <span className='text-11px text-t-tertiary shrink-0'>
                      · {t('settings.modelHub.creation.modelCount', { count: group.models.length })}
                    </span>
                  </div>
                </div>

                {/* Model rows */}
                <div className='flex flex-col'>
                  {group.models.map((entry, idx) => (
                    <div
                      key={entry.model}
                      className={classNames(
                        'flex items-center justify-between gap-8px px-14px py-10px transition-colors hover:bg-[var(--fill-0)]',
                        idx < group.models.length - 1 && 'border-b border-b-solid border-[var(--color-border-2)]/70'
                      )}
                    >
                      <span className='text-13px text-t-primary min-w-0 truncate' title={entry.model}>
                        {entry.model}
                      </span>
                      <div className='flex items-center gap-6px shrink-0'>
                        {entry.capabilities
                          .filter((cap) => cap !== capability)
                          .map((cap) => (
                            <Tag key={cap} size='small' color={CAP_META[cap].color} icon={CAP_META[cap].icon}>
                              {t(CAP_LABEL_KEY[cap])}
                            </Tag>
                          ))}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}
      </NomiScrollArea>
    </div>
  );
};

export default CreationModelsPanel;
