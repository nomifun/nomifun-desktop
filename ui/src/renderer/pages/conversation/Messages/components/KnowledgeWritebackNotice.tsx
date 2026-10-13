import { ipcBridge } from '@/common';
import type { KnowledgeBaseId } from '@/common/types/ids';
import { normalizeKnowledgeWritebackDiagnostic } from '@/common/chat/knowledgeWritebackDiagnostic';
import { sanitizeDiagnosticDetail } from '@/common/chat/providerDiagnostic';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import { Popover } from '@arco-design/web-react';
import { Attention, BookOne, Check, Down, Info, LoadingOne } from '@icon-park/react';
import React, { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import type { KnowledgeWritebackNotice as Notice } from '../knowledgeWritebackNoticeModel';
import styles from './KnowledgeWritebackNotice.module.css';
import ErrorDiagnosticDetails, { modelFailureDiagnosticRows, type DiagnosticMetadataRow } from './ErrorDiagnosticDetails';

const KnowledgeWritebackNotice: React.FC<{ notice: Notice }> = ({ notice }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const detailsId = useId();
  const [names, setNames] = useState<Record<string, string>>({});
  const ids = [...new Set(notice.files.map(file => file.kb_id))];
  const idsKey = ids.join(',');
  useEffect(() => {
    if (!idsKey) return;
    let active = true;
    void Promise.all(idsKey.split(',').map(async id => {
      try {
        const base = await ipcBridge.knowledge.getBase.invoke({ knowledge_base_id: id as KnowledgeBaseId });
        return [id, base.name] as const;
      } catch { return [id, t('messages.knowledgeWriteback.base')] as const; }
    })).then(entries => { if (active) setNames(Object.fromEntries(entries)); });
    return () => { active = false; };
  }, [idsKey, t]);
  if (notice.status === 'no_candidate' || (!notice.files.length && (notice.status === 'written' || notice.status === 'partial'))) return null;
  const saved = notice.status === 'written' || notice.status === 'partial';
  const label = notice.status === 'written' || notice.status === 'partial'
    ? t(notice.status === 'partial' ? 'messages.knowledgeWriteback.savedPartial' : 'messages.knowledgeWriteback.saved', { count: notice.files.length })
    : t({ running: 'messages.knowledgeWriteback.running', failed: 'messages.knowledgeWriteback.failed',
      cancelled: 'messages.knowledgeWriteback.cancelled', interrupted: 'messages.knowledgeWriteback.interrupted',
      no_completer: 'messages.knowledgeWriteback.noCompleter' }[notice.status])
      + (notice.files.length ? ` · ${t('messages.knowledgeWriteback.savedCount', { count: notice.files.length })}` : '');
  const warning = notice.status === 'partial' || notice.status === 'failed' || notice.status === 'interrupted' || notice.status === 'no_completer';
  const diagnostic = normalizeKnowledgeWritebackDiagnostic(notice.diagnostic);
  const provider = diagnostic?.provider_diagnostic;
  const detail = diagnostic
    ? [...new Set([diagnostic.detail, provider?.transportDetail].filter(Boolean))].join('\n') || undefined
    : sanitizeDiagnosticDetail(notice.detail);
  const hasDetails = notice.files.length > 0 || Boolean(diagnostic || detail);
  const causeKey = warning && provider ? `conversation.agentError.providerReasons.${provider.reason}.title`
    : warning && diagnostic?.code ? `conversation.agentError.codes.${diagnostic.code}.title` : undefined;
  const detailsTitle = causeKey ? t(causeKey, { defaultValue: label }) : t('messages.knowledgeWriteback.diagnostics');
  const unknown = t('messages.knowledgeWriteback.details.unknown');
  const number = (value: number | undefined, unit = '') => value === undefined ? undefined
    : `${new Intl.NumberFormat().format(value)}${unit}`;
  const metadata: DiagnosticMetadataRow[] = diagnostic ? [
    [t('messages.knowledgeWriteback.details.stage'), t(`messages.knowledgeWriteback.stages.${diagnostic.stage}`)],
    ...(diagnostic.code ? [[t('messages.knowledgeWriteback.details.code'), diagnostic.code] as const] : []),
    [t('messages.knowledgeWriteback.details.elapsed'), number(diagnostic.elapsed_ms, ' ms')],
    [t('messages.knowledgeWriteback.details.firstFrame'), number(diagnostic.first_frame_ms, ' ms')],
    [t('messages.knowledgeWriteback.details.receivedFrames'), number(diagnostic.received_frames)],
    [t('messages.knowledgeWriteback.details.receivedBytes'), number(diagnostic.received_bytes, ' B')],
    [t('messages.knowledgeWriteback.details.outputBytes'), number(diagnostic.output_bytes, ' B')],
    [t('messages.knowledgeWriteback.details.inputEstimate'), number(diagnostic.input_tokens_estimate)],
    [t('messages.knowledgeWriteback.details.outputBudget'), number(diagnostic.output_token_limit)],
    [t('messages.knowledgeWriteback.details.outputTokens'), number(diagnostic.output_tokens)],
    ...(provider ? [[t('messages.knowledgeWriteback.details.providerReason'), provider.reason] as const,
      [t('conversation.agentError.details.model'), provider.modelName] as const] : []),
    ...modelFailureDiagnosticRows(provider, key => t(`conversation.agentError.details.${key}`)),
  ] : [];
  const report = [label, detailsTitle,
    ...metadata.map(([key, value]) => `${key}: ${value ?? unknown}`),
    detail, ...notice.files.map(file => `${names[file.kb_id] ?? t('messages.knowledgeWriteback.base')}: ${file.rel_path}`),
  ].filter(Boolean).join('\n');
  const icon = notice.status === 'running' ? <LoadingOne size={12} fill='currentColor' />
    : saved ? <Check size={12} fill='currentColor' />
    : warning ? <Attention size={12} fill='currentColor' /> : <Info size={12} fill='currentColor' />;
  const chipClass = `${styles.chip}${saved ? ` ${styles.saved}` : ''}${warning ? ` ${styles.warning}` : ''}`;
  const chipContent = <>
    <span className={styles.stateIcon} aria-hidden='true'><BookOne size={14} fill='currentColor' /></span>
    <span className={styles.label}>{label}</span>
    {ids.length === 1 && names[ids[0]!] && <span className={styles.summaryBase}>{names[ids[0]!]}</span>}
    <span className={`${styles.stateIcon}${notice.status === 'running' ? ` ${styles.running}` : ''}`} aria-hidden='true'>{icon}</span>
    {hasDetails && <span className={styles.chevron} aria-hidden='true'><Down size={12} fill='currentColor' /></span>}
  </>;
  return (
    <div className={styles.notice} role='status' aria-live='polite' data-testid='knowledge-writeback-notice'
      onKeyDown={event => {
        if (event.key === 'Escape' && open) {
          event.preventDefault(); event.stopPropagation();
          setOpen(false); buttonRef.current?.focus();
        }
      }}>
      {hasDetails ? <Popover trigger='click' position='bl' popupVisible={open} onVisibleChange={setOpen}
        content={<div className={styles.details} id={detailsId} role='region' aria-label={t('messages.knowledgeWriteback.diagnostics')}>
          <div className={styles.detailsHeader}>
            <div className={styles.heading}>{detailsTitle}</div>
            <CopyIconButton text={report} tooltip={t('conversation.agentError.copyDetails')} />
          </div>
          {(diagnostic || detail) && <ErrorDiagnosticDetails metadata={metadata} unavailable={unknown}
            detail={detail} detailLabel={t('conversation.agentError.details.rawDetail')} />}
          {notice.files.length > 0 && <div className={styles.documentsHeading}>{t('messages.knowledgeWriteback.documents')}</div>}
          {notice.files.map(file => <Link key={`${file.kb_id}:${file.rel_path}`} to={`/knowledge/${file.kb_id}`}
            className={styles.file} title={t('messages.knowledgeWriteback.openBase')}
            onClick={() => setOpen(false)}>
            <span className={styles.path}>{file.rel_path}</span>
            <span className={styles.base}>{names[file.kb_id] ?? t('messages.knowledgeWriteback.base')}</span>
          </Link>)}
        </div>}>
        <button ref={buttonRef} type='button' className={chipClass} aria-expanded={open} aria-controls={detailsId} aria-label={label}>
          {chipContent}
        </button>
      </Popover> : <span className={`${chipClass} ${styles.passive}`}>{chipContent}</span>}
    </div>
  );
};

export default KnowledgeWritebackNotice;
