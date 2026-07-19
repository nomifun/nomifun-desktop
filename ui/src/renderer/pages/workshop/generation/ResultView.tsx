/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Result presentation for a succeeded card:
 *  - image/video → every persisted result inline; additional live results also
 *    fan out as mode-correct canvas nodes
 *  - text → every generated body (scrollable), each with its own text-node action
 *  - unreadable results stay explicitly identifiable by their persisted asset id
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FileText, Return, TransferData } from '@icon-park/react';
import { useWorkshopMedia } from '../canvas/media';
import type { WorkshopGeneratorBatch } from '../types';
import type { GenMode } from './genTypes';
import { loadWorkshopText } from './pipeline';
import type { AssetId } from '@/common/types/ids';

export interface ResultViewProps {
  mode: GenMode;
  resultAssetIds: AssetId[];
  batch?: WorkshopGeneratorBatch;
  onContinueEdit: (instruction: string) => void;
  onToTextNode: (content: string) => void;
}

const Spinner: React.FC = () => (
  <span className='h-16px w-16px animate-spin rounded-full border-2 border-solid border-[var(--color-fill-3)] border-t-[rgb(var(--primary-6))]' />
);

const ContinueBox: React.FC<{ onSubmit: (v: string) => void }> = ({ onSubmit }) => {
  const { t } = useTranslation();
  const [draft, setDraft] = useState('');
  const submit = (): void => {
    const v = draft.trim();
    if (!v) return;
    onSubmit(v);
    setDraft('');
  };
  return (
    <div className='flex items-center gap-6px rounded-9px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-1)] px-8px py-5px focus-within:border-[rgb(var(--primary-6))]'>
      <input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === 'Enter') {
            e.preventDefault();
            submit();
          }
        }}
        placeholder={t('workshopGeneration.result.continuePlaceholder', { defaultValue: '继续编辑：输入指令回车…' })}
        className='nodrag min-w-0 flex-1 border-none bg-transparent text-12px text-[var(--color-text-1)] outline-none placeholder:text-[var(--color-text-3)]'
      />
      <span
        role='button'
        tabIndex={0}
        title={t('workshopGeneration.result.continue', { defaultValue: '继续编辑' })}
        onClick={submit}
        onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && submit()}
        className={[
          'nodrag grid h-22px w-22px shrink-0 place-items-center rounded-6px cursor-pointer transition-colors',
          draft.trim()
            ? 'bg-[rgb(var(--primary-6))] text-white hover:opacity-90'
            : 'bg-[var(--color-fill-3)] text-[var(--color-text-3)]',
        ].join(' ')}
      >
        <Return theme='outline' size={13} strokeWidth={3} />
      </span>
    </div>
  );
};

/** Put the persisted primary first without hiding or dropping any other result. */
export function orderResultAssetIds(resultAssetIds: AssetId[], primary?: AssetId): AssetId[] {
  if (!primary) return resultAssetIds;
  const index = resultAssetIds.indexOf(primary);
  if (index <= 0) return resultAssetIds;
  return [primary, ...resultAssetIds.slice(0, index), ...resultAssetIds.slice(index + 1)];
}

const ResultOrdinal: React.FC<{ index: number; total: number; assetId: AssetId }> = ({ index, total, assetId }) => {
  if (total <= 1) return null;
  return (
    <span
      title={assetId}
      className='absolute right-6px top-6px z-10 inline-flex items-center gap-3px rounded-full bg-black/55 px-7px py-2px text-10px font-600 text-white backdrop-blur-sm'
    >
      <FileText theme='outline' size={10} strokeWidth={3} />
      {index + 1}/{total}
    </span>
  );
};

const MediaResult: React.FC<{ mode: 'image' | 'video'; assetId: AssetId; index: number; total: number }> = ({
  mode,
  assetId,
  index,
  total,
}) => {
  const { t } = useTranslation();
  const media = useWorkshopMedia(assetId);
  return (
    <div
      data-workshop-result-id={assetId}
      className='relative overflow-hidden rounded-10px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-1)]'
    >
      <ResultOrdinal index={index} total={total} assetId={assetId} />
      {media.status === 'ready' ? (
        mode === 'video' ? (
          <video src={media.url} controls playsInline className='nodrag block max-h-200px w-full bg-black object-contain' />
        ) : (
          <img src={media.url} alt='' draggable={false} className='block max-h-200px w-full select-none object-contain' />
        )
      ) : (
        <div className='flex h-120px flex-col items-center justify-center gap-5px px-8px text-center'>
          {media.status === 'error' ? (
            <>
              <span className='text-11px text-[rgb(var(--danger-6))]'>
                {t('workshopGeneration.result.loadFailed', { defaultValue: '加载失败' })}
              </span>
              <span className='max-w-full break-all text-9px text-[var(--color-text-3)]'>{assetId}</span>
            </>
          ) : (
            <Spinner />
          )}
        </div>
      )}
    </div>
  );
};

type TextLoadState = { status: 'loading' } | { status: 'ready'; content: string } | { status: 'error' };

const TextResult: React.FC<{
  assetId: AssetId;
  index: number;
  total: number;
  onToTextNode: (content: string) => void;
}> = ({ assetId, index, total, onToTextNode }) => {
  const { t } = useTranslation();
  const [state, setState] = useState<TextLoadState>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    setState({ status: 'loading' });
    void loadWorkshopText(assetId).then((content) => {
      if (cancelled) return;
      setState(content == null ? { status: 'error' } : { status: 'ready', content });
    });
    return () => {
      cancelled = true;
    };
  }, [assetId]);

  return (
    <div data-workshop-result-id={assetId} className='relative flex flex-col gap-6px'>
      <div className='relative max-h-160px min-h-56px overflow-y-auto whitespace-pre-wrap break-words rounded-9px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-1)] px-10px py-8px pr-44px text-12px leading-[1.6] text-[var(--color-text-1)] nowheel'>
        <ResultOrdinal index={index} total={total} assetId={assetId} />
        {state.status === 'ready' ? (
          state.content || <span className='text-[var(--color-text-3)]'>{t('workshopGeneration.result.emptyText', { defaultValue: '空文本' })}</span>
        ) : state.status === 'error' ? (
          <span className='flex flex-col gap-3px text-[rgb(var(--danger-6))]'>
            {t('workshopGeneration.result.loadFailed', { defaultValue: '加载失败' })}
            <span className='break-all text-9px text-[var(--color-text-3)]'>{assetId}</span>
          </span>
        ) : (
          <span className='text-[var(--color-text-3)]'>{t('workshopGeneration.result.loading', { defaultValue: '加载中…' })}</span>
        )}
      </div>
      {state.status === 'ready' && (
        <div
          role='button'
          tabIndex={0}
          onClick={() => onToTextNode(state.content)}
          onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && onToTextNode(state.content)}
          className='nodrag inline-flex w-fit items-center gap-5px rounded-7px border border-solid border-[var(--color-border-2)] px-9px py-5px text-11px font-500 text-[var(--color-text-2)] cursor-pointer hover:border-[rgb(var(--primary-6))] hover:text-[rgb(var(--primary-6))] transition-colors'
        >
          <TransferData theme='outline' size={12} strokeWidth={3} />
          {t('workshopGeneration.result.toTextNode', { defaultValue: '转为文本节点' })}
        </div>
      )}
    </div>
  );
};

const ResultView: React.FC<ResultViewProps> = ({ mode, resultAssetIds, batch, onContinueEdit, onToTextNode }) => {
  const { t } = useTranslation();
  const orderedIds = useMemo(
    () => orderResultAssetIds(resultAssetIds, batch?.primary),
    [batch?.primary, resultAssetIds]
  );

  if (orderedIds.length === 0) return null;

  if (mode === 'text') {
    return (
      <div className='flex flex-col gap-8px'>
        {orderedIds.map((assetId, index) => (
          <TextResult
            key={`${assetId}:${index}`}
            assetId={assetId}
            index={index}
            total={orderedIds.length}
            onToTextNode={onToTextNode}
          />
        ))}
      </div>
    );
  }

  return (
    <div className='flex flex-col gap-8px'>
      {orderedIds.length > 1 && (
        <span className='text-10px font-600 text-[var(--color-text-3)]'>
          {t('workshopGeneration.result.batch', { count: orderedIds.length, defaultValue: '{{count}} 项' })}
        </span>
      )}
      {orderedIds.map((assetId, index) => (
        <MediaResult
          key={`${assetId}:${index}`}
          mode={mode}
          assetId={assetId}
          index={index}
          total={orderedIds.length}
        />
      ))}
      <ContinueBox onSubmit={onContinueEdit} />
    </div>
  );
};

export default ResultView;
