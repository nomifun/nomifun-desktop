/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Checkbox, Input } from '@arco-design/web-react';
import { Check, Close, Delete, Download, EditTwo } from '@icon-park/react';
import React from 'react';

import type { CreativeStudioProjectsCopy } from './copy';
import { formatProjectTimestamp } from './projectList';
import styles from './CreativeStudioProjectsPage.module.css';
import type { CreativeStudioProjectSummary } from './types';

interface CreativeStudioProjectCardProps {
  project: CreativeStudioProjectSummary;
  copy: CreativeStudioProjectsCopy;
  language?: string;
  selected: boolean;
  editing: boolean;
  editingTitle: string;
  disabled?: boolean;
  exportDisabled?: boolean;
  archiveUnavailableMessage?: string;
  onOpen: (project: CreativeStudioProjectSummary) => void;
  onToggleSelected: (project: CreativeStudioProjectSummary, selected: boolean) => void;
  onStartRename: (project: CreativeStudioProjectSummary) => void;
  onEditingTitleChange: (title: string) => void;
  onSaveRename: () => void;
  onCancelRename: () => void;
  onExport: (project: CreativeStudioProjectSummary) => void;
  onDelete: (project: CreativeStudioProjectSummary) => void;
}

const CreativeStudioProjectCard: React.FC<CreativeStudioProjectCardProps> = ({
  project,
  copy,
  language,
  selected,
  editing,
  editingTitle,
  disabled = false,
  exportDisabled = false,
  archiveUnavailableMessage,
  onOpen,
  onToggleSelected,
  onStartRename,
  onEditingTitleChange,
  onSaveRename,
  onCancelRename,
  onExport,
  onDelete,
}) => {
  const open = () => {
    if (!editing && !disabled) onOpen(project);
  };
  const saveDisabled = disabled || editingTitle.trim().length === 0;

  return (
    <article
      className={styles.card}
      role='button'
      tabIndex={disabled || editing ? -1 : 0}
      aria-label={`${copy.openProject}: ${project.title}`}
      data-project-id={project.id}
      data-project-selected={selected ? 'true' : 'false'}
      onClick={open}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          open();
        }
      }}
    >
      <div className={styles.cardTop}>
        <Checkbox
          className={styles.cardCheckbox}
          checked={selected}
          disabled={disabled}
          aria-label={copy.selectProject(project.title)}
          onClick={(event) => event.stopPropagation()}
          onChange={(checked) => onToggleSelected(project, checked)}
        />

        <div className={styles.cardIdentity}>
          {editing ? (
            <Input
              autoFocus
              value={editingTitle}
              maxLength={80}
              aria-label={copy.renamePlaceholder}
              placeholder={copy.renamePlaceholder}
              onClick={(event) => event.stopPropagation()}
              onChange={onEditingTitleChange}
              onPressEnter={() => {
                if (!saveDisabled) onSaveRename();
              }}
              onKeyDown={(event) => {
                if (event.key === 'Escape') onCancelRename();
              }}
            />
          ) : (
            <>
              <h2 className={styles.cardTitle}>{project.title}</h2>
              <p className={styles.cardStats}>{copy.projectStats(project.nodeCount, project.connectionCount)}</p>
            </>
          )}
        </div>
      </div>

      <div className={styles.cardFooter}>
        <p className={styles.cardTimestamp}>
          {copy.updatedAt(formatProjectTimestamp(project.updatedAt, language))}
        </p>
        <div className={styles.cardActions} onClick={(event) => event.stopPropagation()}>
          {editing ? (
            <>
              <Button
                type='text'
                size='small'
                shape='circle'
                icon={<Check theme='outline' size={15} fill='currentColor' />}
                aria-label={copy.saveRename}
                title={copy.saveRename}
                disabled={saveDisabled}
                onClick={onSaveRename}
              />
              <Button
                type='text'
                size='small'
                shape='circle'
                icon={<Close theme='outline' size={15} fill='currentColor' />}
                aria-label={copy.cancelRename}
                title={copy.cancelRename}
                disabled={disabled}
                onClick={onCancelRename}
              />
            </>
          ) : (
            <>
              <Button
                type='text'
                size='small'
                shape='circle'
                icon={<Download theme='outline' size={15} fill='currentColor' />}
                aria-label={`${copy.exportProject}: ${project.title}`}
                title={exportDisabled ? archiveUnavailableMessage : copy.exportProject}
                disabled={disabled || exportDisabled}
                onClick={() => onExport(project)}
              />
              <Button
                type='text'
                size='small'
                shape='circle'
                icon={<EditTwo theme='outline' size={15} fill='currentColor' />}
                aria-label={`${copy.renameProject}: ${project.title}`}
                title={copy.renameProject}
                disabled={disabled}
                onClick={() => onStartRename(project)}
              />
              <Button
                type='text'
                size='small'
                shape='circle'
                icon={<Delete theme='outline' size={15} fill='currentColor' />}
                aria-label={`${copy.deleteProject}: ${project.title}`}
                title={copy.deleteProject}
                disabled={disabled}
                onClick={() => onDelete(project)}
              />
            </>
          )}
        </div>
      </div>
    </article>
  );
};

export default CreativeStudioProjectCard;
