/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Check,
  Close,
  Delete,
  Download,
  Error,
  FolderOpen,
  History,
  Loading,
  MagicWand,
  Pause,
  Play,
  Plus,
  Refresh,
  SettingTwo,
  Voice,
} from '@icon-park/react';
import { Button, Input, InputNumber, Progress, Select, Slider, Tag, Tooltip } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import styles from './AudioWorkbench.module.css';
import {
  DEFAULT_AUDIO_WORKBENCH_FIELD_SUPPORT,
  DEFAULT_AUDIO_WORKBENCH_SPEED_RANGE,
  canGenerateAudioWorkbench,
  clampAudioWorkbenchSpeed,
  isAudioWorkbenchBusy,
  type AudioWorkbenchCanceledResult,
  type AudioWorkbenchFailedResult,
  type AudioWorkbenchOption,
  type AudioWorkbenchProps,
  type AudioWorkbenchResult,
  type AudioWorkbenchSucceededResult,
  type AudioWorkbenchTaskState,
} from './types';

const taskLabel = (
  t: ReturnType<typeof useTranslation>['t'],
  state: AudioWorkbenchTaskState
): string => {
  switch (state) {
    case 'idle':
      return t('creativeStudio.audio.task.idle', { defaultValue: '等待生成' });
    case 'queued':
      return t('creativeStudio.audio.task.queued', { defaultValue: '排队中' });
    case 'running':
      return t('creativeStudio.audio.task.running', { defaultValue: '生成中' });
    case 'succeeded':
      return t('creativeStudio.audio.task.succeeded', { defaultValue: '生成完成' });
    case 'failed':
      return t('creativeStudio.audio.task.failed', { defaultValue: '生成失败' });
    case 'canceled':
      return t('creativeStudio.audio.task.canceled', { defaultValue: '已取消' });
  }
};

const resultLabel = (
  t: ReturnType<typeof useTranslation>['t'],
  state: Exclude<AudioWorkbenchTaskState, 'idle'>
): string => {
  switch (state) {
    case 'queued':
      return t('creativeStudio.audio.result.queued', { defaultValue: '排队中' });
    case 'running':
      return t('creativeStudio.audio.result.running', { defaultValue: '生成中' });
    case 'succeeded':
      return t('creativeStudio.audio.result.succeeded', { defaultValue: '已完成' });
    case 'failed':
      return t('creativeStudio.audio.result.failed', { defaultValue: '失败' });
    case 'canceled':
      return t('creativeStudio.audio.result.canceled', { defaultValue: '已取消' });
  }
};

const clampPercent = (value: number | undefined): number | undefined =>
  value == null ? undefined : Math.min(100, Math.max(0, value));

const formatDuration = (milliseconds: number | undefined): string | null => {
  if (milliseconds == null || !Number.isFinite(milliseconds)) return null;
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
};

const formatBytes = (bytes: number | undefined): string | null => {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return null;
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
};

const withCurrentOption = (
  options: readonly AudioWorkbenchOption[],
  current: string
): AudioWorkbenchOption[] => {
  if (!current || options.some((option) => option.value === current)) return [...options];
  return [{ value: current, label: current }, ...options];
};

const statusIcon = (state: Exclude<AudioWorkbenchTaskState, 'idle'>) => {
  if (state === 'queued' || state === 'running') {
    return <Loading className={styles.spin} theme='outline' size={16} fill='currentColor' />;
  }
  if (state === 'succeeded') return <Check theme='outline' size={16} fill='currentColor' />;
  if (state === 'failed') return <Error theme='outline' size={16} fill='currentColor' />;
  return <Close theme='outline' size={16} fill='currentColor' />;
};

const StatusPanel: React.FC<
  Pick<AudioWorkbenchProps, 'task' | 'onCancel' | 'onRetry'> & { disabled: boolean }
> = ({ task, onCancel, onRetry, disabled }) => {
  const { t } = useTranslation();
  if (task.state === 'idle') return null;
  const progress = clampPercent(task.progress);
  const canCancel = (task.state === 'queued' || task.state === 'running') && onCancel;
  const canRetry = (task.state === 'failed' || task.state === 'canceled') && onRetry;

  return (
    <section
      className={styles.statusPanel}
      data-tone={task.state}
      data-audio-task-state={task.state}
      aria-live='polite'
    >
      <div className={styles.statusHeading}>
        <span className={styles.statusIcon} aria-hidden='true'>
          {statusIcon(task.state)}
        </span>
        <div>
          <strong>{taskLabel(t, task.state)}</strong>
          {task.message ? <p>{task.message}</p> : null}
        </div>
        {canCancel ? (
          <Button size='small' type='text' disabled={disabled} onClick={onCancel}>
            {t('creativeStudio.audio.actions.cancelTask', { defaultValue: '取消任务' })}
          </Button>
        ) : null}
        {canRetry ? (
          <Button size='small' icon={<Refresh />} disabled={disabled} onClick={onRetry}>
            {t('creativeStudio.audio.actions.regenerate', { defaultValue: '重新生成' })}
          </Button>
        ) : null}
      </div>
      {progress !== undefined && (task.state === 'queued' || task.state === 'running') ? (
        <Progress percent={progress} size='small' showText />
      ) : null}
      {task.state === 'failed' && task.errorMessage ? (
        <p className={styles.statusError} role='alert'>
          {task.errorMessage}
        </p>
      ) : null}
    </section>
  );
};

const ReferenceList: React.FC<
  Pick<
    AudioWorkbenchProps,
    'references' | 'referenceRequired' | 'onChooseReferences' | 'onRemoveReference'
  > & { disabled: boolean; supported: boolean }
> = ({ references, referenceRequired, onChooseReferences, onRemoveReference, disabled, supported }) => {
  const { t } = useTranslation();
  return (
    <section className={styles.composerSection} data-audio-reference-section data-supported={supported}>
      <header className={styles.sectionHeader}>
        <div>
          <span>{t('creativeStudio.audio.references.title', { defaultValue: '参考音频' })}</span>
          {referenceRequired ? (
            <small>{t('creativeStudio.audio.references.required', { defaultValue: '必需' })}</small>
          ) : (
            <small>{t('creativeStudio.audio.references.optional', { defaultValue: '可选' })}</small>
          )}
        </div>
        <Tag>{references.length}</Tag>
      </header>
      <div className={styles.sectionBody}>
        <Button
          size='small'
          icon={<FolderOpen />}
          disabled={disabled || !supported || !onChooseReferences}
          onClick={onChooseReferences}
        >
          {t('creativeStudio.audio.references.choose', { defaultValue: '从画布或素材库选择' })}
        </Button>
        {!supported ? (
          <p className={styles.fieldHint}>
            {t('creativeStudio.audio.references.unsupported', {
              defaultValue: '当前模型未声明参考音频能力。',
            })}
          </p>
        ) : null}
        {references.length === 0 ? (
          <div className={styles.referenceEmpty} data-audio-references='empty'>
            <Voice theme='outline' size={23} fill='currentColor' />
            <span>
              {referenceRequired
                ? t('creativeStudio.audio.references.requiredEmpty', {
                    defaultValue: '请选择一段真实参考音频',
                  })
                : t('creativeStudio.audio.references.empty', { defaultValue: '没有参考音频' })}
            </span>
          </div>
        ) : (
          <div className={styles.referenceList} data-audio-references='ready'>
            {references.map((reference) => {
              const metadata = [
                reference.mimeType,
                formatDuration(reference.durationMs),
                formatBytes(reference.sizeBytes),
              ].filter(Boolean);
              return (
                <div key={reference.assetId} className={styles.referenceItem}>
                  <span className={styles.referenceIcon} aria-hidden='true'>
                    <Voice theme='outline' size={16} fill='currentColor' />
                  </span>
                  <span className={styles.referenceIdentity}>
                    <strong title={reference.name}>{reference.name}</strong>
                    {metadata.length ? <small>{metadata.join(' · ')}</small> : null}
                  </span>
                  <Tooltip
                    content={t('creativeStudio.audio.references.remove', {
                      defaultValue: '移除 {{name}}',
                      name: reference.name,
                    })}
                  >
                    <Button
                      type='text'
                      size='mini'
                      shape='circle'
                      icon={<Delete />}
                      aria-label={t('creativeStudio.audio.references.removeAria', {
                        defaultValue: '移除参考音频 {{name}}',
                        name: reference.name,
                      })}
                      disabled={disabled}
                      onClick={() => onRemoveReference(reference.assetId)}
                    />
                  </Tooltip>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
};

const ResultStatus: React.FC<{ result: AudioWorkbenchResult }> = ({ result }) => {
  const { t } = useTranslation();
  const progress = result.status === 'running' ? clampPercent(result.progress) : undefined;
  const label =
    result.status === 'queued' || result.status === 'running'
      ? result.statusLabel || resultLabel(t, result.status)
      : resultLabel(t, result.status);
  const color =
    result.status === 'succeeded'
      ? 'green'
      : result.status === 'failed'
        ? 'red'
        : result.status === 'canceled'
          ? 'gray'
          : 'arcoblue';

  return (
    <div className={styles.resultStatus}>
      <Tag color={color} icon={statusIcon(result.status)}>
        {label}
      </Tag>
      {progress !== undefined ? <Progress percent={progress} size='small' showText={false} /> : null}
    </div>
  );
};

const SucceededActions: React.FC<{
  result: AudioWorkbenchSucceededResult;
  playing: boolean;
  disabled: boolean;
  onPlaybackChange: AudioWorkbenchProps['onPlaybackChange'];
  onDownloadResult: AudioWorkbenchProps['onDownloadResult'];
  onInsertResult: AudioWorkbenchProps['onInsertResult'];
}> = ({ result, playing, disabled, onPlaybackChange, onDownloadResult, onInsertResult }) => {
  const { t } = useTranslation();
  return (
    <div className={styles.resultActions}>
      <Button
        size='small'
        icon={playing ? <Pause /> : <Play />}
        aria-pressed={playing}
        disabled={disabled}
        onClick={() => onPlaybackChange(result, !playing)}
      >
        {playing
          ? t('creativeStudio.audio.actions.pause', { defaultValue: '暂停' })
          : t('creativeStudio.audio.actions.play', { defaultValue: '播放' })}
      </Button>
      <Tooltip content={t('creativeStudio.audio.actions.downloadAudio', { defaultValue: '下载音频' })}>
        <Button
          size='small'
          icon={<Download />}
          aria-label={t('creativeStudio.audio.actions.downloadResult', {
            defaultValue: '下载 {{title}}',
            title: result.title,
          })}
          disabled={disabled}
          onClick={() => onDownloadResult(result)}
        />
      </Tooltip>
      <Button
        size='small'
        type='primary'
        icon={<Plus />}
        disabled={disabled}
        onClick={() => onInsertResult(result)}
      >
        {t('creativeStudio.audio.actions.insertCanvas', { defaultValue: '插入画布' })}
      </Button>
    </div>
  );
};

const ResultCard: React.FC<{
  result: AudioWorkbenchResult;
  playingResultId?: string | null;
  disabled: boolean;
  onPlaybackChange: AudioWorkbenchProps['onPlaybackChange'];
  onDownloadResult: AudioWorkbenchProps['onDownloadResult'];
  onInsertResult: AudioWorkbenchProps['onInsertResult'];
  onRetryResult?: AudioWorkbenchProps['onRetryResult'];
}> = ({
  result,
  playingResultId,
  disabled,
  onPlaybackChange,
  onDownloadResult,
  onInsertResult,
  onRetryResult,
}) => {
  const { t } = useTranslation();
  const metadata = [result.modelLabel, result.formatLabel, result.createdAtLabel].filter(
    (item): item is string => Boolean(item)
  );
  if (result.status === 'succeeded') {
    metadata.splice(
      2,
      0,
      ...[formatDuration(result.durationMs), formatBytes(result.sizeBytes)].filter(
        (item): item is string => Boolean(item)
      )
    );
  }

  return (
    <article className={styles.resultCard} data-audio-result-state={result.status}>
      <div className={styles.resultGlyph} aria-hidden='true'>
        <Voice theme='outline' size={22} fill='currentColor' />
      </div>
      <div className={styles.resultContent}>
        <div className={styles.resultTitleRow}>
          <div>
            <strong title={result.title}>{result.title}</strong>
            {result.text ? <p title={result.text}>{result.text}</p> : null}
          </div>
          <ResultStatus result={result} />
        </div>
        {metadata.length ? (
          <div className={styles.resultMeta}>
            {metadata.map((item, index) => (
              <span key={`${item}-${index}`}>{item}</span>
            ))}
          </div>
        ) : null}
        {result.status === 'failed' ? (
          <p className={styles.resultError} role='alert'>
            {result.errorMessage}
          </p>
        ) : null}
        {result.status === 'canceled' && result.message ? (
          <p className={styles.resultMessage}>{result.message}</p>
        ) : null}
        <div className={styles.resultFooter}>
          {result.status === 'succeeded' ? (
            <SucceededActions
              result={result}
              playing={playingResultId === result.id}
              disabled={disabled}
              onPlaybackChange={onPlaybackChange}
              onDownloadResult={onDownloadResult}
              onInsertResult={onInsertResult}
            />
          ) : (result.status === 'failed' || result.status === 'canceled') && onRetryResult ? (
            <Button
              size='small'
              icon={<Refresh />}
              disabled={disabled}
              onClick={() => onRetryResult(result as AudioWorkbenchFailedResult | AudioWorkbenchCanceledResult)}
            >
              {t('creativeStudio.audio.actions.retryResult', { defaultValue: '重试此结果' })}
            </Button>
          ) : result.status === 'queued' || result.status === 'running' ? (
            <span className={styles.waitingLabel}>
              {t('creativeStudio.audio.result.waitingActions', {
                defaultValue: '任务完成后可播放、下载或插入画布',
              })}
            </span>
          ) : null}
        </div>
      </div>
    </article>
  );
};

const AudioWorkbench: React.FC<AudioWorkbenchProps> = ({
  value,
  modelSlot,
  voiceOptions,
  formatOptions,
  references,
  results,
  task,
  playingResultId,
  disabled = false,
  maxTextLength = 4096,
  speedRange = DEFAULT_AUDIO_WORKBENCH_SPEED_RANGE,
  fieldSupport,
  referenceRequired = false,
  onValueChange,
  onChooseReferences,
  onRemoveReference,
  onGenerate,
  onCancel,
  onRetry,
  onPlaybackChange,
  onDownloadResult,
  onInsertResult,
  onRetryResult,
}) => {
  const { t } = useTranslation();
  const support = { ...DEFAULT_AUDIO_WORKBENCH_FIELD_SUPPORT, ...fieldSupport };
  const busy = isAudioWorkbenchBusy(task.state);
  const controlsDisabled = disabled || busy;
  const textLength = Array.from(value.text).length;
  const canGenerate = canGenerateAudioWorkbench(value, task.state, references.length, {
    disabled: disabled || (referenceRequired && !support.references),
    maxTextLength,
    referenceRequired,
  });
  const update = (patch: Partial<AudioWorkbenchProps['value']>) =>
    onValueChange({ ...value, ...patch });
  const resolvedVoiceOptions = withCurrentOption(voiceOptions, value.voice);
  const resolvedFormatOptions = withCurrentOption(formatOptions, value.format);

  return (
    <section
      className={styles.workbench}
      data-audio-workbench
      data-audio-task-state={task.state}
      aria-busy={busy}
    >
      <aside className={styles.composer} data-audio-workbench-composer>
        <header className={styles.composerHeader}>
          <div>
            <span className={styles.eyebrow}>
              {t('creativeStudio.audio.header.eyebrow', { defaultValue: 'AUDIO STUDIO' })}
            </span>
            <h1>{t('creativeStudio.audio.header.title', { defaultValue: '音频工作台' })}</h1>
            <p>
              {t('creativeStudio.audio.header.description', {
                defaultValue: '把文字、语气与真实参考素材交给已配置的语音模型。',
              })}
            </p>
          </div>
          <span className={styles.headerIcon} aria-hidden='true'>
            <Voice theme='outline' size={22} fill='currentColor' />
          </span>
        </header>

        <div className={styles.composerScroll}>
          <section className={styles.composerSection}>
            <header className={styles.sectionHeader}>
              <span>{t('creativeStudio.audio.fields.text', { defaultValue: '朗读文本' })}</span>
              <span className={textLength > maxTextLength ? styles.charCountError : styles.charCount}>
                {textLength} / {maxTextLength}
              </span>
            </header>
            <div className={styles.sectionBody}>
              <Input.TextArea
                value={value.text}
                rows={7}
                maxLength={maxTextLength + 1}
                placeholder={t('creativeStudio.audio.fields.textPlaceholder', {
                  defaultValue: '输入需要合成的旁白、对白或播报文本…',
                })}
                disabled={controlsDisabled}
                onChange={(text) => update({ text })}
                onPressEnter={(event) => {
                  if ((event.ctrlKey || event.metaKey) && canGenerate) onGenerate(value);
                }}
              />
              <p className={styles.fieldHint}>
                {t('creativeStudio.audio.fields.textShortcut', {
                  defaultValue:
                    'Ctrl / ⌘ + Enter 提交；当前 NomiFun `/api/tts` 上限为 4096 字符。',
                })}
              </p>
            </div>
          </section>

          <section className={styles.composerSection}>
            <header className={styles.sectionHeader}>
              <span>
                {t('creativeStudio.audio.fields.modelAndVoice', { defaultValue: '模型与声音' })}
              </span>
              <SettingTwo aria-hidden='true' />
            </header>
            <div className={styles.sectionBody}>
              <label className={styles.field}>
                <span>{t('creativeStudio.audio.fields.model', { defaultValue: '语音模型' })}</span>
                <div
                  className={styles.modelSlot}
                  data-audio-model-slot
                  data-disabled={controlsDisabled || undefined}
                  aria-disabled={controlsDisabled}
                >
                  {modelSlot}
                </div>
              </label>

              <label className={styles.field}>
                <span>{t('creativeStudio.audio.fields.voice', { defaultValue: '音色' })}</span>
                <Select
                  value={value.voice || undefined}
                  options={resolvedVoiceOptions}
                  placeholder={t('creativeStudio.audio.fields.voicePlaceholder', {
                    defaultValue: '使用模型默认音色或输入 voice ID',
                  })}
                  allowCreate
                  allowClear
                  showSearch
                  disabled={controlsDisabled || !support.voice}
                  onChange={(voice) => update({ voice: typeof voice === 'string' ? voice : '' })}
                />
                <small>
                  {t('creativeStudio.audio.fields.voiceHint', {
                    defaultValue: 'voice 保持自由文本，避免把供应商音色 ID 锁死在前端。',
                  })}
                </small>
              </label>

              <div className={styles.twoColumnFields}>
                <label className={styles.field}>
                  <span>{t('creativeStudio.audio.fields.format', { defaultValue: '输出格式' })}</span>
                  <Select
                    value={value.format || undefined}
                    options={resolvedFormatOptions}
                    placeholder={t('creativeStudio.audio.fields.modelDefault', {
                      defaultValue: '模型默认',
                    })}
                    allowClear
                    disabled={controlsDisabled || !support.format}
                    onChange={(format) => update({ format: typeof format === 'string' ? format : '' })}
                  />
                </label>
                <label className={styles.field}>
                  <span>{t('creativeStudio.audio.fields.speed', { defaultValue: '语速' })}</span>
                  <div className={styles.speedControl}>
                    <Slider
                      min={speedRange.min}
                      max={speedRange.max}
                      step={speedRange.step}
                      value={clampAudioWorkbenchSpeed(value.speed, speedRange)}
                      disabled={controlsDisabled || !support.speed}
                      onChange={(speed) => {
                        if (typeof speed === 'number') update({ speed: clampAudioWorkbenchSpeed(speed, speedRange) });
                      }}
                    />
                    <InputNumber
                      min={speedRange.min}
                      max={speedRange.max}
                      step={speedRange.step}
                      precision={2}
                      value={clampAudioWorkbenchSpeed(value.speed, speedRange)}
                      disabled={controlsDisabled || !support.speed}
                      onChange={(speed) => update({ speed: clampAudioWorkbenchSpeed(speed ?? 1, speedRange) })}
                    />
                    <span>×</span>
                  </div>
                  {!support.speed ? (
                    <small>
                      {t('creativeStudio.audio.fields.speedUnsupported', {
                        defaultValue: '当前模型协议未声明语速参数。',
                      })}
                    </small>
                  ) : null}
                </label>
              </div>

              <label className={styles.field}>
                <span>
                  {t('creativeStudio.audio.fields.instructions', { defaultValue: '声音指令' })}
                </span>
                <Input.TextArea
                  value={value.instructions}
                  rows={3}
                  placeholder={t('creativeStudio.audio.fields.instructionsPlaceholder', {
                    defaultValue: '例如：自然、温暖、语速轻快，结尾略微上扬。',
                  })}
                  disabled={controlsDisabled || !support.instructions}
                  onChange={(instructions) => update({ instructions })}
                />
                {!support.instructions ? (
                  <small>
                    {t('creativeStudio.audio.fields.instructionsUnsupported', {
                      defaultValue: '当前模型协议未声明声音指令能力。',
                    })}
                  </small>
                ) : null}
              </label>
            </div>
          </section>

          <ReferenceList
            references={references}
            referenceRequired={referenceRequired}
            onChooseReferences={onChooseReferences}
            onRemoveReference={onRemoveReference}
            disabled={controlsDisabled}
            supported={support.references}
          />
        </div>

        <footer className={styles.generateFooter}>
          <StatusPanel task={task} onCancel={onCancel} onRetry={onRetry} disabled={disabled} />
          {!canGenerate && !busy ? (
            <p className={styles.generateHint}>
              {!value.model
                ? t('creativeStudio.audio.validation.modelRequired', {
                    defaultValue: '请选择 speech_synthesis 模型',
                  })
                : !value.text.trim()
                  ? t('creativeStudio.audio.validation.textRequired', {
                      defaultValue: '请先填写朗读文本',
                    })
                  : textLength > maxTextLength
                    ? t('creativeStudio.audio.validation.textTooLong', {
                        defaultValue: '朗读文本超过长度限制',
                      })
                    : referenceRequired && !support.references
                      ? t('creativeStudio.audio.validation.referenceUnsupported', {
                          defaultValue: '当前模型不支持必需的参考音频',
                        })
                      : referenceRequired && references.length === 0
                      ? t('creativeStudio.audio.validation.referenceRequired', {
                          defaultValue: '当前模式需要参考音频',
                        })
                      : t('creativeStudio.audio.validation.unavailable', {
                          defaultValue: '当前配置暂不可提交',
                        })}
            </p>
          ) : null}
          <Button
            type='primary'
            long
            size='large'
            icon={busy ? <Loading className={styles.spin} /> : <MagicWand />}
            disabled={!canGenerate}
            onClick={() => onGenerate(value)}
          >
            {task.state === 'queued'
              ? t('creativeStudio.audio.generate.waiting', { defaultValue: '等待模型' })
              : task.state === 'running'
                ? t('creativeStudio.audio.generate.running', { defaultValue: '正在生成' })
                : t('creativeStudio.audio.generate.submit', { defaultValue: '生成音频' })}
          </Button>
        </footer>
      </aside>

      <main className={styles.resultsPanel} data-audio-workbench-results data-result-count={results.length}>
        <header className={styles.resultsHeader}>
          <div>
            <History theme='outline' size={18} fill='currentColor' />
            <h2>{t('creativeStudio.audio.results.title', { defaultValue: '音频结果' })}</h2>
            <Tag>{results.length}</Tag>
          </div>
          <p>
            {t('creativeStudio.audio.results.description', {
              defaultValue: '结果由外部任务与资产服务提供；这里不创建临时音频。',
            })}
          </p>
        </header>

        {results.length === 0 ? (
          <div className={styles.emptyResults} data-audio-results='empty'>
            <span className={styles.emptyIcon} aria-hidden='true'>
              <Voice theme='outline' size={34} fill='currentColor' />
            </span>
            <strong>
              {t('creativeStudio.audio.results.emptyTitle', { defaultValue: '还没有音频结果' })}
            </strong>
            <p>
              {t('creativeStudio.audio.results.emptyDescription', {
                defaultValue: '填写文本并选择语音模型，真实生成结果会显示在这里。',
              })}
            </p>
          </div>
        ) : (
          <div className={styles.resultList} data-audio-results='ready'>
            {results.map((result) => (
              <ResultCard
                key={result.id}
                result={result}
                playingResultId={playingResultId}
                disabled={disabled}
                onPlaybackChange={onPlaybackChange}
                onDownloadResult={onDownloadResult}
                onInsertResult={onInsertResult}
                onRetryResult={onRetryResult}
              />
            ))}
          </div>
        )}
      </main>
    </section>
  );
};

export default AudioWorkbench;
