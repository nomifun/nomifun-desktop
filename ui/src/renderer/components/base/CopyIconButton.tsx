/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Message, Tooltip } from '@arco-design/web-react';
import { Copy } from '@icon-park/react';
import classNames from 'classnames';
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';

import { copyText } from '@/renderer/utils/ui/clipboard';

type CopyIconButtonProps = {
  /** Text to copy to the clipboard. */
  text: string;
  /** Tooltip + aria label. Defaults to common.copy. */
  tooltip?: string;
  /** Success toast text. Defaults to common.copySuccess. */
  successMessage?: string;
  /** Icon size in px. */
  size?: number;
  /** Extra classes for the clickable wrapper (sizing / placement). */
  className?: string;
};

/**
 * Icon-only "copy to clipboard" affordance with a tooltip and success/failure
 * toast — the single elegant copy primitive shared by the workspace pill, the
 * sidebar workpath header, and the session hover cards. Stops propagation so a
 * copy never doubles as a row click.
 */
const CopyIconButton: React.FC<CopyIconButtonProps> = ({ text, tooltip, successMessage, size = 13, className }) => {
  const { t } = useTranslation();
  const label = tooltip ?? t('common.copy');

  const handleCopy = useCallback(
    (event: React.SyntheticEvent) => {
      event.preventDefault();
      event.stopPropagation();
      copyText(text)
        .then(() => Message.success(successMessage ?? t('common.copySuccess')))
        .catch(() => Message.error(t('common.copyFailed')));
    },
    [text, successMessage, t]
  );

  return (
    <Tooltip content={label} position='top' mini>
      <span
        role='button'
        tabIndex={0}
        aria-label={label}
        className={classNames(
          'inline-flex items-center justify-center cursor-pointer rd-4px text-t-tertiary transition-colors hover:text-t-primary',
          className
        )}
        onClick={handleCopy}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') handleCopy(event);
        }}
      >
        <Copy theme='outline' size={size} fill='currentColor' className='block leading-none' />
      </span>
    </Tooltip>
  );
};

export default CopyIconButton;
