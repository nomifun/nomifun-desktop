/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Tooltip } from '@arco-design/web-react';
import { Copy, Delete, EditTwo, Play } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import type { CreativeTemplateDefinitionV1 } from '../domain';
import type { CreativeTemplateTranslationCopy } from '../templateI18n';
import { templateOutputLabel } from './templateViewModel';
import styles from './TemplateWorkspaceCard.module.css';

interface TemplateWorkspaceCardProps {
  template: CreativeTemplateDefinitionV1;
  disabled: boolean;
  copy: CreativeTemplateTranslationCopy;
  onRun: () => void;
  onEdit: () => void;
  onCopy: () => void;
  onDelete: () => void;
}

/** Compact Kanban surface with SearchResults' title/description/type hierarchy,
 * adapted from codevisual-share using Nomifun's shared controls and tokens. */
export default function TemplateWorkspaceCard({
  template, disabled, copy, onRun, onEdit, onCopy, onDelete,
}: TemplateWorkspaceCardProps) {
  const { t } = useTranslation();
  const category = template.metadata.category.trim();
  const tags = [...new Set([category, ...template.metadata.tags].map((tag) => tag.trim()).filter(Boolean))];
  const visibleTags = tags.slice(0, 3);
  const remainingTags = tags.slice(3);
  const editLabel = t('creativeStudio.templates.workspace.edit', { name: template.metadata.name, defaultValue: 'Edit {{name}}' });
  const copyLabel = t('creativeStudio.templates.workspace.duplicate', { name: template.metadata.name, defaultValue: 'Duplicate {{name}}' });
  const deleteLabel = t('creativeStudio.templates.workspace.delete', { name: template.metadata.name, defaultValue: 'Delete {{name}}' });

  return (
    <article className={styles.card} data-template-id={template.id}>
      <header className={styles.header}>
        <h2 className={styles.title} title={template.metadata.name}>{template.metadata.name}</h2>
        <span className={styles.output}>{templateOutputLabel(template.output, copy)}</span>
      </header>
      <p className={styles.description}>
        {template.metadata.description || t('creativeStudio.templates.workspace.noDescription', { defaultValue: 'No description' })}
      </p>
      <div className={styles.tags}>
        {visibleTags.map((tag) => (
          <span key={tag} className={styles.tag} data-category={tag === category} title={tag}>{tag}</span>
        ))}
        {remainingTags.length > 0 ? (
          <Tooltip content={remainingTags.join(' · ')} trigger={['hover', 'focus']}>
            <span className={styles.moreTags} tabIndex={0} aria-label={remainingTags.join(', ')}>+{remainingTags.length}</span>
          </Tooltip>
        ) : null}
      </div>
      <footer className={styles.footer}>
        <div className={styles.actions}>
          <Tooltip content={editLabel}><Button type='text' size='small' aria-label={editLabel} disabled={disabled} icon={<EditTwo theme='outline' size={16} fill='currentColor' />} onClick={onEdit} /></Tooltip>
          <Tooltip content={copyLabel}><Button type='text' size='small' aria-label={copyLabel} disabled={disabled} icon={<Copy theme='outline' size={16} fill='currentColor' />} onClick={onCopy} /></Tooltip>
          <Tooltip content={deleteLabel}><Button className={styles.deleteButton} type='text' size='small' aria-label={deleteLabel} disabled={disabled} icon={<Delete theme='outline' size={16} fill='currentColor' />} onClick={onDelete} /></Tooltip>
        </div>
        <Button className={styles.runButton} type='secondary' size='small' disabled={disabled} icon={<Play theme='outline' size={14} fill='currentColor' />} onClick={onRun}>
          {t('creativeStudio.templates.workspace.run', { defaultValue: 'Run' })}
        </Button>
      </footer>
    </article>
  );
}
