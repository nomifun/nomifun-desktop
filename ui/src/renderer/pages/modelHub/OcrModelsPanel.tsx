/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import classNames from 'classnames';
import { Button, Modal, Progress, Tag, Tooltip } from '@arco-design/web-react';
import { Delete, Download, Loading, Pause, PlayOne } from '@icon-park/react';
import type {
  OcrModelCatalogEntry,
  OcrModelComponent,
  OcrModelState,
  OcrModelTransferProgress,
} from '@/common/types/provider/ocrModelService';
import type { LocalModelErrorKind, LocalModelInstallPhase } from '@/common/types/provider/localModelService';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import LocalModelCapabilitySummary from './LocalModelCapabilitySummary';
import LocalModelDetails from './LocalModelDetails';
import { detailsForcedOpen } from './localModelCapabilityView';
import { formatLocalModelBytes, formatLocalModelRate } from './localModelView';
import { useLocalOcrModels } from './useLocalOcrModels';

type OcrPrimaryAction = 'install' | 'pause' | 'resume' | 'retry' | 'none';

export interface OcrModelsPanelProps {
  controller: ReturnType<typeof useLocalOcrModels>;
  className?: string;
}

const emptyState = (modelId: string): OcrModelState => ({
  modelId,
  installPhase: 'not_installed',
  progress: null,
  installedBytes: 0,
  errorKind: null,
  message: null,
});

const modelState = (states: OcrModelState[] | undefined, modelId: string): OcrModelState =>
  states?.find((state) => state.modelId === modelId) ?? emptyState(modelId);

const primaryAction = (state: OcrModelState): OcrPrimaryAction => {
  switch (state.installPhase) {
    case 'not_installed':
      return 'install';
    case 'downloading':
    case 'verifying':
      return 'pause';
    case 'paused':
      return 'resume';
    case 'failed':
      return 'retry';
    case 'installed':
      return 'none';
  }
};

const percentOf = (downloaded: number, total: number): number | null => {
  if (!Number.isFinite(downloaded) || !Number.isFinite(total) || total <= 0) return null;
  return Math.min(100, Math.max(0, (downloaded / total) * 100));
};

const phaseColor = (phase: LocalModelInstallPhase): string | undefined => {
  if (phase === 'installed') return 'green';
  if (phase === 'downloading' || phase === 'verifying') return 'blue';
  if (phase === 'paused') return 'orange';
  if (phase === 'failed') return 'red';
  return undefined;
};

const OcrModelsPanel: React.FC<OcrModelsPanelProps> = ({ controller, className }) => {
  const { t, i18n } = useTranslation();
  const [message, messageContext] = useArcoMessage();
  const {
    catalog,
    status,
    catalogError,
    statusError,
    isLoading,
    pendingAction,
    install,
    pause,
    resume,
    remove,
  } = controller;
  const locale = i18n.resolvedLanguage ?? i18n.language;

  const phaseLabel = (phase: LocalModelInstallPhase): string => {
    switch (phase) {
      case 'not_installed':
        return t('settings.modelHub.local.ocr.phase.notInstalled');
      case 'downloading':
        return t('settings.modelHub.local.ocr.phase.downloading');
      case 'verifying':
        return t('settings.modelHub.local.ocr.phase.verifying');
      case 'installed':
        return t('settings.modelHub.local.ocr.phase.installed');
      case 'paused':
        return t('settings.modelHub.local.ocr.phase.paused');
      case 'failed':
        return t('settings.modelHub.local.ocr.phase.failed');
    }
  };

  const errorLabel = (kind: LocalModelErrorKind | null): string => {
    switch (kind) {
      case 'network':
        return t('settings.modelHub.local.ocr.error.network');
      case 'insufficient_space':
        return t('settings.modelHub.local.ocr.error.insufficientSpace');
      case 'checksum_mismatch':
        return t('settings.modelHub.local.ocr.error.checksumMismatch');
      case 'unsupported_platform':
        return t('settings.modelHub.local.ocr.error.unsupportedPlatform');
      case 'runtime_unavailable':
        return t('settings.modelHub.local.ocr.error.runtimeUnavailable');
      case 'busy':
        return t('settings.modelHub.local.ocr.error.busy');
      case 'not_found':
        return t('settings.modelHub.local.ocr.error.notFound');
      case 'unknown':
      case null:
        return t('settings.modelHub.local.ocr.error.unknown');
    }
  };

  const componentLabel = (component: OcrModelComponent): string => {
    switch (component) {
      case 'detector':
        return t('settings.modelHub.local.ocr.component.detector');
      case 'detector_config':
        return t('settings.modelHub.local.ocr.component.detectorConfig');
      case 'recognizer':
        return t('settings.modelHub.local.ocr.component.recognizer');
      case 'recognizer_config':
        return t('settings.modelHub.local.ocr.component.recognizerConfig');
    }
  };

  const actionLabel = (action: OcrPrimaryAction): string => {
    switch (action) {
      case 'install':
        return t('settings.modelHub.local.ocr.action.install');
      case 'pause':
        return t('settings.modelHub.local.ocr.action.pause');
      case 'resume':
        return t('settings.modelHub.local.ocr.action.resume');
      case 'retry':
        return t('settings.modelHub.local.ocr.action.retry');
      case 'none':
        return t('settings.modelHub.local.ocr.phase.installed');
    }
  };

  const actionIcon = (action: OcrPrimaryAction): React.ReactNode => {
    switch (action) {
      case 'install':
      case 'retry':
        return <Download theme='outline' size='14' />;
      case 'pause':
        return <Pause theme='outline' size='14' />;
      case 'resume':
        return <PlayOne theme='outline' size='14' />;
      case 'none':
        return null;
    }
  };

  const runAction = async (
    action: () => Promise<unknown>,
    successKey: string,
    context: string
  ): Promise<void> => {
    try {
      await action();
      message.success(t(successKey));
    } catch (error) {
      console.error(`Local OCR ${context} failed:`, error);
      message.error(t('settings.modelHub.local.ocr.actionFailed'));
    }
  };

  const invokePrimaryAction = async (modelId: string, action: OcrPrimaryAction): Promise<void> => {
    switch (action) {
      case 'install':
      case 'retry':
        await runAction(
          () => install(modelId),
          'settings.modelHub.local.ocr.installSuccess',
          'installation'
        );
        return;
      case 'pause':
        await runAction(() => pause(modelId), 'settings.modelHub.local.ocr.pauseSuccess', 'pause');
        return;
      case 'resume':
        await runAction(() => resume(modelId), 'settings.modelHub.local.ocr.resumeSuccess', 'resume');
        return;
      case 'none':
        return;
    }
  };

  const confirmRemove = (model: OcrModelCatalogEntry): void => {
    Modal.confirm({
      title: t('settings.modelHub.local.ocr.deleteConfirmTitle'),
      content: t('settings.modelHub.local.ocr.deleteConfirmContent', { model: model.name }),
      okText: t('settings.modelHub.local.ocr.action.delete'),
      cancelText: t('common.cancel'),
      okButtonProps: { status: 'danger' },
      onOk: () =>
        runAction(
          () => remove(model.id),
          'settings.modelHub.local.ocr.deleteSuccess',
          'deletion'
        ),
    });
  };

  const renderProgress = (
    state: OcrModelState,
    progress: OcrModelTransferProgress
  ): React.ReactNode => {
    const overallPercent = percentOf(progress.overallDownloadedBytes, progress.overallTotalBytes);
    const componentPercent = percentOf(progress.downloadedBytes, progress.totalBytes);
    return (
      <div className='mt-11px rd-9px bg-[var(--fill-0)] px-11px py-10px'>
        <div className='mb-6px flex items-center justify-between gap-8px text-12px font-500 text-t-primary'>
          <span>{t('settings.modelHub.local.ocr.progress.overall')}</span>
          <span>{overallPercent == null ? t('settings.modelHub.local.ocr.progress.preparing') : `${overallPercent.toFixed(1)}%`}</span>
        </div>
        {overallPercent != null && <Progress percent={overallPercent} showText={false} strokeWidth={5} />}
        <div className='mt-6px flex items-center justify-between gap-8px text-11px text-t-secondary'>
          <span>
            {formatLocalModelBytes(progress.overallDownloadedBytes, locale)} /{' '}
            {formatLocalModelBytes(progress.overallTotalBytes, locale)}
          </span>
          {progress.bytesPerSecond > 0 && (
            <span>{formatLocalModelRate(progress.bytesPerSecond, locale)}</span>
          )}
        </div>
        <div className='mt-8px border-t border-solid border-[var(--color-border-2)] pt-7px text-11px text-t-secondary flex items-center justify-between gap-8px flex-wrap'>
          <span>
            {t('settings.modelHub.local.ocr.progress.currentComponent', {
              component: componentLabel(progress.component),
            })}
          </span>
          <span>
            {formatLocalModelBytes(progress.downloadedBytes, locale)} /{' '}
            {formatLocalModelBytes(progress.totalBytes, locale)}
            {componentPercent == null ? '' : ` · ${componentPercent.toFixed(1)}%`}
          </span>
        </div>
        {state.installPhase === 'verifying' && (
          <div className='mt-7px flex items-center gap-6px text-11px text-t-secondary'>
            <Loading theme='outline' size='12' className='animate-spin' />
            {t('settings.modelHub.local.ocr.progress.verifyingHint')}
          </div>
        )}
      </div>
    );
  };

  const loadFailed = (catalogError || statusError) && !catalog && !status;
  const installedCount = status?.models.filter((model) => model.installPhase === 'installed').length ?? 0;

  return (
    <div className={classNames('space-y-12px', className)}>
      {messageContext}
      <LocalModelCapabilitySummary
        items={[
          {
            label: t('settings.modelHub.local.ocr.title'),
            value: t('settings.modelHub.local.capabilityCenter.availableModels', { count: catalog?.length ?? 0 }),
          },
          {
            label: t('settings.modelHub.local.ocr.readiness.artifacts'),
            value: t('settings.modelHub.local.capabilityCenter.installedModels', { count: installedCount }),
            tone: status?.artifactsReady ? 'success' : 'neutral',
          },
          {
            label: t('settings.modelHub.local.ocr.readiness.inference'),
            value: status?.inferenceReady
              ? t('settings.modelHub.local.ocr.readiness.inferenceReady')
              : status?.artifactsReady
                ? t('settings.modelHub.local.ocr.readiness.runtimePending')
                : t('settings.modelHub.local.ocr.readiness.installFirst'),
            tone: status?.inferenceReady ? 'success' : status?.artifactsReady ? 'warning' : 'neutral',
          },
        ]}
      />

      {status?.artifactsReady && !status.inferenceReady && (
        <div className='mt-8px text-11px leading-17px text-[rgb(var(--warning-6))]'>
          {t('settings.modelHub.local.ocr.readiness.runtimePendingHint')}
        </div>
      )}

      <div>
        {isLoading && !catalog ? (
          <div className='flex items-center justify-center gap-7px py-24px text-12px text-t-secondary'>
            <Loading theme='outline' size='16' className='animate-spin' />
            {t('settings.modelHub.local.ocr.loading')}
          </div>
        ) : loadFailed ? (
          <div className='py-24px text-center'>
            <div className='text-13px font-500 text-t-primary'>{t('settings.modelHub.local.ocr.loadFailed')}</div>
            <div className='mt-4px text-12px text-t-secondary'>{t('settings.modelHub.local.ocr.loadFailedHint')}</div>
          </div>
        ) : !catalog?.length ? (
          <div className='py-24px text-center text-12px text-t-secondary'>
            {t('settings.modelHub.local.ocr.empty')}
          </div>
        ) : (
          <div className='space-y-10px'>
            {catalog.map((model) => {
              const state = modelState(status?.models, model.id);
              const action = primaryAction(state);
              const actionPending = pendingAction?.endsWith(`:${model.id}`) ?? false;
              const deleteAllowed =
                state.installPhase === 'installed' ||
                state.installPhase === 'paused' ||
                state.installPhase === 'failed';
              const actionDisabled = !status || Boolean(statusError) || pendingAction != null;
              const savedPercent = percentOf(state.installedBytes, model.downloadSizeBytes);

              return (
                <article key={model.id} className='rd-11px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-13px py-12px shadow-[0_5px_18px_rgba(0,0,0,0.025)]'>
                  <div className='flex items-start justify-between gap-12px flex-wrap'>
                    <div className='min-w-0 flex-1'>
                      <div className='flex items-center gap-6px flex-wrap'>
                        <span className='text-14px font-600 text-t-primary'>{model.name}</span>
                        {model.recommended && (
                          <Tag size='small' color='arcoblue'>
                            {t('settings.modelHub.local.ocr.recommended')}
                          </Tag>
                        )}
                        <Tag size='small' color={phaseColor(state.installPhase)}>
                          {phaseLabel(state.installPhase)}
                        </Tag>
                      </div>
                      <div className='mt-5px text-12px leading-18px text-t-secondary'>
                        {model.id === 'pp-ocrv6-small-onnx'
                          ? t('settings.modelHub.local.ocr.catalogDescription')
                          : model.description}
                      </div>
                      <div className='mt-8px flex items-center gap-x-11px gap-y-5px flex-wrap text-11px text-t-secondary'>
                        <span>{model.format}</span>
                        <span>
                          {t('settings.modelHub.local.ocr.metadata.download', {
                            size: formatLocalModelBytes(model.downloadSizeBytes, locale),
                          })}
                        </span>
                        <span>
                          {t('settings.modelHub.local.ocr.metadata.memory', {
                            size: formatLocalModelBytes(model.requiredMemoryBytes, locale),
                          })}
                        </span>
                      </div>
                    </div>

                    <div className='flex items-center gap-7px shrink-0'>
                      {action !== 'none' && (
                        <Button
                          size='small'
                          type='primary'
                          icon={actionIcon(action)}
                          loading={actionPending}
                          disabled={actionDisabled}
                          onClick={() => void invokePrimaryAction(model.id, action)}
                        >
                          {actionLabel(action)}
                        </Button>
                      )}
                    </div>
                  </div>

                  <LocalModelDetails
                    forcedOpen={detailsForcedOpen(state.installPhase, Boolean(state.errorKind))}
                  >
                    <div className='flex items-center gap-6px flex-wrap'>
                      {model.components.map((component) => (
                        <Tag key={component} size='small'>
                          {componentLabel(component)}
                        </Tag>
                      ))}
                      <span className='text-11px text-t-secondary'>{model.license}</span>
                      <span className='text-11px text-t-secondary'>
                        {t('settings.modelHub.local.ocr.metadata.source', { source: model.source })}
                      </span>
                      {deleteAllowed && (
                        <Tooltip content={t('settings.modelHub.local.ocr.action.delete')}>
                          <Button
                            size='mini'
                            type='text'
                            status='danger'
                            icon={<Delete theme='outline' size='13' />}
                            disabled={pendingAction != null || Boolean(statusError)}
                            onClick={() => confirmRemove(model)}
                            aria-label={t('settings.modelHub.local.ocr.deleteModelLabel', { model: model.name })}
                          >
                            {t('settings.modelHub.local.ocr.action.delete')}
                          </Button>
                        </Tooltip>
                      )}
                    </div>
                    {state.progress && renderProgress(state, state.progress)}
                    {state.installPhase === 'verifying' && !state.progress && (
                      <div className='mt-9px flex items-center gap-6px text-12px text-t-secondary'>
                        <Loading theme='outline' size='13' className='animate-spin' />
                        {t('settings.modelHub.local.ocr.progress.verifyingHint')}
                      </div>
                    )}
                    {state.installPhase === 'paused' && savedPercent != null && (
                      <div className='mt-10px rd-8px bg-[var(--fill-0)] px-10px py-8px'>
                        <div className='mb-5px flex items-center justify-between gap-8px text-11px text-t-secondary'>
                          <span>{t('settings.modelHub.local.ocr.progress.checkpointSaved')}</span>
                          <span>
                            {formatLocalModelBytes(state.installedBytes, locale)} /{' '}
                            {formatLocalModelBytes(model.downloadSizeBytes, locale)}
                          </span>
                        </div>
                        <Progress percent={savedPercent} showText={false} strokeWidth={4} color='rgb(var(--warning-6))' />
                      </div>
                    )}
                    {state.errorKind && (
                      <div className='mt-9px rd-7px bg-[rgba(var(--danger-6),0.07)] px-9px py-7px text-12px text-[rgb(var(--danger-6))]'>
                        {errorLabel(state.errorKind)}
                      </div>
                    )}
                  </LocalModelDetails>
                </article>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
};

export default OcrModelsPanel;
