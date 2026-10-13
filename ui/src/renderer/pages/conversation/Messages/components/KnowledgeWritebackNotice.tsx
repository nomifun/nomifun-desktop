import { ipcBridge } from '@/common';
import type { KnowledgeBaseId } from '@/common/types/ids';
import { Popover } from '@arco-design/web-react';
import { BookOne, Check, Down } from '@icon-park/react';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import type { KnowledgeWritebackNotice as Notice } from '../knowledgeWritebackNoticeModel';
import styles from './KnowledgeWritebackNotice.module.css';

const KnowledgeWritebackNotice: React.FC<{ notice: Notice }> = ({ notice }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
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
  if (!notice.files.length) return null;
  const label = t(notice.partial ? 'messages.knowledgeWriteback.savedPartial' : 'messages.knowledgeWriteback.saved',
    { count: notice.files.length });
  return (
    <div className={styles.notice} role='status' aria-live='polite' data-testid='knowledge-writeback-notice'
      onKeyDown={event => {
        if (event.key === 'Escape' && open) {
          event.preventDefault(); event.stopPropagation();
          setOpen(false); buttonRef.current?.focus();
        }
      }}>
      <Popover trigger='click' position='bl' popupVisible={open} onVisibleChange={setOpen}
        content={<div className={styles.details}>
          <div className={styles.heading}>{t('messages.knowledgeWriteback.documents')}</div>
          {notice.files.map(file => <Link key={`${file.kb_id}:${file.rel_path}`} to={`/knowledge/${file.kb_id}`}
            className={styles.file} title={t('messages.knowledgeWriteback.openBase')}
            onClick={() => setOpen(false)}>
            <span className={styles.path}>{file.rel_path}</span>
            <span className={styles.base}>{names[file.kb_id] ?? t('messages.knowledgeWriteback.base')}</span>
          </Link>)}
        </div>}>
        <button ref={buttonRef} type='button' className={`${styles.chip}${notice.partial ? ` ${styles.partial}` : ''}`} aria-expanded={open} aria-label={label}>
          <span className={styles.success} aria-hidden='true'><BookOne size={14} fill='currentColor' /></span>
          <span className={styles.label}>{label}</span>
          {ids.length === 1 && names[ids[0]!] && <span className={styles.summaryBase}>{names[ids[0]!]}</span>}
          <span className={styles.success} aria-hidden='true'><Check size={12} fill='currentColor' /></span>
          <span className={styles.chevron} aria-hidden='true'><Down size={12} fill='currentColor' /></span>
        </button>
      </Popover>
    </div>
  );
};

export default KnowledgeWritebackNotice;
