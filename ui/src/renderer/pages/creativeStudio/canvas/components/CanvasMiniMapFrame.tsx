/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';
import { useTranslation } from 'react-i18next';

import styles from './CanvasMiniMapFrame.module.css';

export interface CanvasMiniMapFrameProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
  label?: string;
  footer?: React.ReactNode;
}

/**
 * Screen-space chrome for a controlled minimap renderer. The child owns its
 * drawing and navigation semantics; this component only supplies product
 * chrome and a stable responsive boundary.
 */
const CanvasMiniMapFrame: React.FC<CanvasMiniMapFrameProps> = ({
  children,
  className,
  label,
  footer,
  ...rest
}) => {
  const { t } = useTranslation();

  return (
    <div
      {...rest}
      className={classNames(styles.frame, className)}
      role='group'
      aria-label={label ?? t('creativeStudio.canvas.minimap.label')}
      data-canvas-no-zoom
      data-canvas-minimap
    >
      <div className={styles.content}>{children}</div>
      {footer ? <div className={styles.footer}>{footer}</div> : null}
    </div>
  );
};

export default CanvasMiniMapFrame;
