/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Alert, Button } from '@arco-design/web-react';
import type { TFunction } from 'i18next';

export function WorkspaceReadFailure({ t, hasSnapshot, retrying, onRetry }: {
  t: TFunction;
  hasSnapshot: boolean;
  retrying: boolean;
  onRetry: () => void;
}) {
  return (
    <Alert
      className='mx-12px my-8px'
      type='warning'
      title={t('conversation.workspace.readErrorTitle')}
      content={
        <>
          <div>{t('conversation.workspace.readErrorDescription')}</div>
          {hasSnapshot && <div>{t('conversation.workspace.staleFiles')}</div>}
          <Button className='mt-8px' size='small' loading={retrying} disabled={retrying} onClick={onRetry}>
            {t('common.retry')}
          </Button>
        </>
      }
    />
  );
}
