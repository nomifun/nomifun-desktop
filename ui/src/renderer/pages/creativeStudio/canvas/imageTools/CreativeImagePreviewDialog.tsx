/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Image, Modal, Spin } from '@arco-design/web-react';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { CreativeAsset } from '../../assets';
import type { CreativeCanvasNode } from '../../domain';
import styles from './CreativeImageTools.module.css';

type ImageNode = Extract<CreativeCanvasNode, { type: 'image' }>;

export interface CreativeImagePreviewDialogProps {
  node: ImageNode;
  resolveAsset(node: ImageNode): Promise<CreativeAsset>;
  onClose(): void;
}

/** Mounted for one preview session; never changes the canvas document or viewport. */
const CreativeImagePreviewDialog: React.FC<CreativeImagePreviewDialogProps> = ({
  node,
  resolveAsset,
  onClose,
}) => {
  const { t } = useTranslation();
  const returnFocusRef = useRef(
    typeof document === 'undefined' ? null : document.activeElement
  );
  const [asset, setAsset] = useState<CreativeAsset | null>(null);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [stage, setStage] = useState<HTMLDivElement | null>(null);
  const getStage = useCallback(() => stage!, [stage]);

  useEffect(() => {
    let active = true;
    setAsset(null);
    setFailed(false);
    void resolveAsset(node).then(
      (resolved) => {
        if (!active) return;
        if (resolved.kind !== 'image' || !resolved.originalUrl.trim()) {
          setFailed(true);
          return;
        }
        setAsset(resolved);
      },
      () => {
        if (active) setFailed(true);
      }
    );
    return () => { active = false; };
  }, [attempt, node, resolveAsset]);

  useEffect(() => () => {
    const target = returnFocusRef.current;
    queueMicrotask(() => {
      if (target instanceof HTMLElement && target.isConnected) {
        target.focus({ preventScroll: true });
      }
    });
  }, []);

  const title = t('creativeStudio.canvas.imageTools.toolbar.previewLabel');

  return (
    <Modal
      visible
      title={title}
      aria-label={title}
      className={styles.previewModal}
      // Escape the canvas shell's clipping/stacking context, including its
      // viewport-portaled composers (1600) and node toolbars (1601).
      getPopupContainer={() => document.body}
      maskStyle={{ zIndex: 1700 }}
      wrapStyle={{ zIndex: 1700 }}
      alignCenter
      autoFocus
      focusLock
      escToExit
      maskClosable
      unmountOnExit
      onCancel={onClose}
      footer={null}
    >
      <div ref={setStage} className={styles.previewStage} data-creative-image-preview>
        {failed ? (
          <div className={styles.previewStatus} role='alert'>
            <span>{t('creativeStudio.canvas.imageTools.preview.loadFailed')}</span>
            <Button onClick={() => setAttempt((current) => current + 1)}>
              {t('common.retry')}
            </Button>
          </div>
        ) : asset && stage ? (
          <Image.Preview
            key={`${asset.id}:${attempt}`}
            visible
            src={asset.originalUrl}
            imgAttributes={{
              alt: asset.title || node.data.alt || node.data.caption || title,
              draggable: false,
              onError: () => setFailed(true),
            }}
            getPopupContainer={getStage}
            closable={false}
            escToExit={false}
            maskClosable={false}
            actionsLayout={['zoomIn', 'zoomOut', 'originalSize', 'rotateLeft', 'rotateRight']}
          />
        ) : (
          <div className={styles.previewStatus} role='status'>
            <Spin />
            <span>{t('common.loading')}</span>
          </div>
        )}
      </div>
    </Modal>
  );
};

export default CreativeImagePreviewDialog;
