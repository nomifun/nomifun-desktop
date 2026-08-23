/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Message, Modal } from '@arco-design/web-react';
import { Copy, FolderPlus } from '@icon-park/react';
import type { TFunction } from 'i18next';
import React from 'react';
import { useTranslation } from 'react-i18next';

import { openExternalUrl } from '@/renderer/utils/platform';

import type { PromptLibraryItem } from '../types';
import styles from './PromptLibraryDetails.module.css';

export type PromptCopyState = 'idle' | 'copying' | 'copied' | 'failed';
export type PromptSaveState = 'idle' | 'saving' | 'saved' | 'failed';

export interface PromptLibraryDetailsProps {
  item: PromptLibraryItem | null;
  locale: string;
  copyState: PromptCopyState;
  copyError?: string | null;
  saveState?: PromptSaveState;
  saveError?: string | null;
  onClose(): void;
  onCopy(): void;
  onSave?(): void;
}

export interface PromptLibraryDetailsContentProps {
  item: PromptLibraryItem;
  locale: string;
}

function sourceLabel(item: PromptLibraryItem, t: TFunction): string {
  if (item.source === 'catalog') {
    return t('creativeStudio.prompts.sourceCatalog', {
      defaultValue: 'Public prompt catalog',
    });
  }
  return item.source === 'preset'
    ? t('creativeStudio.prompts.sourcePreset', {
        defaultValue: 'NomiFun preset',
      })
    : t('creativeStudio.prompts.sourceAsset', {
        defaultValue: 'My text assets',
      });
}

function updatedAtLabel(value: number | null, locale: string): string | null {
  if (value === null) return null;
  const milliseconds = value < 10_000_000_000 ? value * 1_000 : value;
  const date = new Date(milliseconds);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  }).format(date);
}

export const PromptLibraryDetailsContent: React.FC<PromptLibraryDetailsContentProps> = ({
  item,
  locale,
}) => {
  const { t } = useTranslation();
  const updatedAt = updatedAtLabel(item.updatedAt, locale);
  const preview = item.preview?.replace(/!\[[^\]]*]\([^)]+\)/g, '').trim() ?? '';
  const openAuditableSource = (event: React.MouseEvent<HTMLAnchorElement>): void => {
    event.preventDefault();
    const url = event.currentTarget.href;
    void openExternalUrl(url).catch(() =>
      Message.error(
        t('creativeStudio.prompts.externalLinkError', {
          defaultValue: 'Could not open the external link',
        })
      )
    );
  };
  return (
    <div
      className={styles.content}
      data-prompt-library-details='true'
      data-prompt-source={item.source}
    >
      <div className={styles.detailsGrid} data-has-cover={item.coverUrl ? 'true' : 'false'}>
        {item.coverUrl ? (
          <div className={styles.previewColumn}>
            <img
              className={styles.cover}
              src={item.coverUrl}
              alt={item.title}
              referrerPolicy='no-referrer'
            />
            {preview ? <pre className={styles.preview}>{preview}</pre> : null}
          </div>
        ) : null}

        <div className={styles.promptColumn}>
          <div className={styles.metadata}>
            <span className={styles.source}>{sourceLabel(item, t)}</span>
            <span className={styles.category}>
              {item.category ??
                t('creativeStudio.prompts.uncategorized', {
                  defaultValue: 'Uncategorized',
                })}
            </span>
          </div>

          {item.description ? <p className={styles.description}>{item.description}</p> : null}

          <div className={styles.promptBlock}>
            <span className={styles.sectionLabel}>
              {t('creativeStudio.prompts.fullPrompt', {
                defaultValue: 'Full prompt',
              })}
            </span>
            <pre className={styles.prompt}>{item.prompt}</pre>
          </div>

          {item.tags.length > 0 ? (
            <div
              className={styles.tagList}
              aria-label={t('creativeStudio.prompts.tagsLabel', {
                defaultValue: 'Tags',
              })}
            >
              {item.tags.map((tag) => (
                <span key={tag} className={styles.tag}>
                  {tag}
                </span>
              ))}
            </div>
          ) : null}

          {updatedAt || item.knowledgeBaseIds.length > 0 || item.license ? (
            <div className={styles.facts}>
              {updatedAt ? (
                <span>
                  {t('creativeStudio.prompts.updatedAt', {
                    defaultValue: 'Updated {{date}}',
                    date: updatedAt,
                  })}
                </span>
              ) : null}
              {item.knowledgeBaseIds.length > 0 ? (
                <span>
                  {t('creativeStudio.prompts.relatedKnowledgeBases', {
                    defaultValue: '{{count}} linked knowledge bases',
                    count: item.knowledgeBaseIds.length,
                  })}
                </span>
              ) : null}
              {item.sourceUrl ? (
                <a
                  href={item.sourceUrl}
                  target='_blank'
                  rel='noreferrer'
                  onClick={openAuditableSource}
                >
                  {t('creativeStudio.prompts.viewSource', {
                    defaultValue: 'View source',
                  })}
                </a>
              ) : null}
              {item.license ? (
                item.licenseUrl ? (
                  <a
                    href={item.licenseUrl}
                    target='_blank'
                    rel='noreferrer'
                    onClick={openAuditableSource}
                  >
                    {item.license}
                  </a>
                ) : (
                  <span>{item.license}</span>
                )
              ) : null}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
};

export const PromptLibraryDetails: React.FC<PromptLibraryDetailsProps> = ({
  item,
  locale,
  copyState,
  copyError,
  saveState = 'idle',
  saveError,
  onClose,
  onCopy,
  onSave,
}) => {
  const { t } = useTranslation();
  const feedback =
    saveState === 'saved'
      ? t('creativeStudio.prompts.savedFeedback', {
          defaultValue: 'Added to "My assets".',
        })
      : saveState === 'failed'
        ? saveError ||
          t('creativeStudio.prompts.saveFailedFallback', {
            defaultValue: 'Save failed. Try again later.',
          })
        : copyState === 'copied'
          ? t('creativeStudio.prompts.copiedFeedback', {
              defaultValue: 'Prompt copied to the clipboard.',
            })
          : copyState === 'failed'
            ? copyError ||
              t('creativeStudio.prompts.copyFailedFallback', {
                defaultValue: 'Copy failed. Check clipboard permissions.',
              })
            : t('creativeStudio.prompts.noCanvasMutation', {
                defaultValue: 'The standalone prompt library does not modify any canvas.',
              });

  return (
    <Modal
      visible={item !== null}
      title={
        item?.title ??
        t('creativeStudio.prompts.detailsTitle', {
          defaultValue: 'Prompt details',
        })
      }
      footer={null}
      style={{ width: 860, maxWidth: 'calc(100vw - 32px)' }}
      autoFocus={false}
      unmountOnExit
      getPopupContainer={() =>
        document.getElementById('creative-studio-portal-root') ?? document.body
      }
      onCancel={onClose}
    >
      {item ? (
        <>
          <PromptLibraryDetailsContent item={item} locale={locale} />
          <div className={styles.actions}>
            <p
              className={styles.copyFeedback}
              data-copy-state={copyState}
              role={copyState === 'failed' || saveState === 'failed' ? 'alert' : 'status'}
              aria-live='polite'
            >
              {feedback}
            </p>
            <div className={styles.actionButtons}>
              <Button
                type='primary'
                icon={<Copy theme='outline' size={15} fill='currentColor' />}
                loading={copyState === 'copying'}
                onClick={onCopy}
              >
                {t('creativeStudio.prompts.copyPromptAction', {
                  defaultValue: 'Copy prompt',
                })}
              </Button>
              {onSave ? (
                <Button
                  icon={<FolderPlus theme='outline' size={15} fill='currentColor' />}
                  loading={saveState === 'saving'}
                  disabled={saveState === 'saved'}
                  onClick={onSave}
                >
                  {saveState === 'saved'
                    ? t('creativeStudio.prompts.addedToAssets', {
                        defaultValue: 'Added to assets',
                      })
                    : t('creativeStudio.prompts.addToAssets', {
                        defaultValue: 'Add to my assets',
                      })}
                </Button>
              ) : null}
            </div>
          </div>
        </>
      ) : null}
    </Modal>
  );
};

export default PromptLibraryDetails;
