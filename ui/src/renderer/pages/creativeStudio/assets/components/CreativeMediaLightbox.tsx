/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Close, Download } from '@icon-park/react';
import { Message, Modal, Tooltip } from '@arco-design/web-react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';

import ImageLightbox from '@/renderer/components/media/ImageLightbox';
import imageLightboxStyles from '@/renderer/components/media/ImageLightbox.module.css';
import { saveUrlAs } from '@/renderer/utils/file/saveAs';
import CreativeVideoPlayer from './CreativeVideoPlayer';
import styles from './CreativeMediaLightbox.module.css';

interface CreativeMediaLightboxProps {
  kind: 'image' | 'video';
  src: string;
  posterSrc?: string | null;
  title: string;
  fileName?: string;
  mimeType?: string | null;
  onSaveAs?: () => Promise<unknown>;
  onClose(): void;
  zIndex?: number;
}

/** Full-resolution preview shared by compact creative-media references. */
const CreativeMediaLightbox: React.FC<CreativeMediaLightboxProps> = ({
  kind,
  src,
  posterSrc,
  title,
  fileName,
  mimeType,
  onSaveAs,
  onClose,
  zIndex,
}) => {
  const { t } = useTranslation();
  const [saving, setSaving] = useState(false);
  const suggestedName = fileName
    ?? (/\.[a-z0-9]{1,16}$/i.test(title) ? title : `${title}.${kind === 'image' ? 'png' : 'mp4'}`);
  const saveMedia = onSaveAs ?? (() => saveUrlAs(src, {
    suggestedName,
    mimeType,
    dialogTitle: t('common.saveAs', { defaultValue: '另存为' }),
  }));
  if (kind === 'image') {
    return <ImageLightbox src={src} title={title} onClose={onClose} onSaveAs={saveMedia} zIndex={zIndex} />;
  }

  const saveAsLabel = t('common.saveAs', { defaultValue: '另存为' });
  const closeLabel = t('common.close', { defaultValue: '关闭' });
  const closePreviewLabel = t('creativeStudio.canvas.video.closePreview', {
    defaultValue: '关闭视频预览',
  });
  return (
    <Modal
      visible
      title={title}
      className={`nomifun-modal-fullscreen ${imageLightboxStyles.modal}`}
      wrapClassName={imageLightboxStyles.wrap}
      maskStyle={{ background: 'rgba(0, 0, 0, .88)', zIndex }}
      wrapStyle={zIndex === undefined ? undefined : { zIndex }}
      footer={null}
      closable={false}
      focusLock
      unmountOnExit
      onCancel={onClose}
    >
      <div className={`${imageLightboxStyles.viewport} ${styles.videoViewport}`}>
        <div className={styles.videoFrame}>
          <CreativeVideoPlayer
            src={src}
            poster={posterSrc ?? undefined}
            label={title}
          />
        </div>
      </div>
      <div className={imageLightboxStyles.actions}>
        <Tooltip content={saveAsLabel}>
          <button
            type='button'
            disabled={saving}
            aria-label={`${saveAsLabel}：${title}`}
            onClick={() => {
              if (saving) return;
              setSaving(true);
              void saveMedia()
                .catch((error) => Message.error(error instanceof Error ? error.message : String(error)))
                .finally(() => setSaving(false));
            }}
          >
            <Download size={20} fill='currentColor' />
          </button>
        </Tooltip>
        <Tooltip content={closeLabel}>
          <button type='button' aria-label={closePreviewLabel} onClick={onClose}>
            <Close size={20} fill='currentColor' />
          </button>
        </Tooltip>
      </div>
    </Modal>
  );
};

export default CreativeMediaLightbox;
