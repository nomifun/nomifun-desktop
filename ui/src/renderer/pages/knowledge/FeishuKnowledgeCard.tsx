/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * FeishuKnowledgeCard — pinned entry to the connector-backed Feishu knowledge
 * space (/knowledge/feishu). It is not a local knowledge base: the card only
 * navigates to the cloud workbench, so it carries no edit/delete actions and
 * hides while kind/tag/search filters are active.
 */
import React from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { LarkOne } from '@icon-park/react';
import styles from './KnowledgeCard.module.css';

const FeishuKnowledgeCard: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const title = t('knowledge.feishu.title', { defaultValue: '飞书知识库' });
  return (
    <article className={styles.card} data-testid='feishu-knowledge-card'>
      <button
        type='button'
        className={styles.openButton}
        aria-label={`${t('knowledge.card.actionOpen', { defaultValue: '打开' })}: ${title}`}
        onClick={() => navigate('/knowledge/feishu')}
      >
        <span className={styles.heading}>
          <span className={styles.kindIcon} aria-hidden='true'>
            <LarkOne theme='outline' size={15} strokeWidth={3} fill='currentColor' />
          </span>
          <span className={styles.name}>{title}</span>
          <span className={styles.source}>{t('knowledge.feishu.connectorTag', { defaultValue: '飞书连接器' })}</span>
        </span>
        <span className={styles.body}>
          <span className={styles.description}>{t('knowledge.feishu.description', { defaultValue: '浏览飞书云文档与多维表格，正文在浏览器中打开。' })}</span>
        </span>
        <span className={`knowledge-card-footer knowledge-card-meta ${styles.meta}`}>
          <span>{t('knowledge.feishu.browserHint', { defaultValue: '云端内容，正文在浏览器中打开' })}</span>
        </span>
      </button>
    </article>
  );
};

export default FeishuKnowledgeCard;
