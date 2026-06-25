/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Message } from '@arco-design/web-react';
import { Copy } from '@icon-park/react';
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';

import { copyText } from '@/renderer/utils/ui/clipboard';

type CopyFullIdButtonProps = {
  /** The full entity ID to copy (never rendered as text). Numeric for the
   *  integer-keyed entities (conversation/requirement/terminal); string for
   *  TEXT short-id entities. */
  id: string | number;
  /** Button size, defaults to 'mini'. */
  size?: 'mini' | 'small' | 'default' | 'large';
  className?: string;
};

/**
 * "Copy full ID" action — the single entry point for grabbing an entity's
 * long ID now that `#N` (seq) is the only identifier shown as visible text.
 * Renders a compact button; clicking copies the full ID and toasts feedback.
 */
const CopyFullIdButton: React.FC<CopyFullIdButtonProps> = ({ id, size = 'mini', className }) => {
  const { t } = useTranslation();

  const handleClick = useCallback(
    (event: Event) => {
      event.stopPropagation();
      copyText(String(id))
        .then(() => Message.success(t('common.copySuccess')))
        .catch(() => Message.error(t('common.copyFailed')));
    },
    [id, t]
  );

  return (
    <Button
      size={size}
      type='secondary'
      className={className}
      icon={<Copy theme='outline' size='12' fill='currentColor' />}
      onClick={handleClick}
    >
      {t('common.copyFullId')}
    </Button>
  );
};

export default CopyFullIdButton;
