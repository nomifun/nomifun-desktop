/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Message } from '@arco-design/web-react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import { ErrorModal, ErrorNotice } from '@/renderer/components/base/ErrorFeedback';
import { normalizeKnowledgeError, type KnowledgeErrorContext, type KnowledgeErrorPresentation } from './knowledgeError';

export { ErrorDetails as KnowledgeErrorDetails } from '@/renderer/components/base/ErrorFeedback';

export function showKnowledgeErrorMessage(error: unknown, context?: KnowledgeErrorContext, t?: TFunction): void {
  const presentation = normalizeKnowledgeError(error, context, t);
  if (!presentation.handled) Message.error(presentation.message);
}

export function KnowledgeErrorNotice({ error, onRetry }: {
  error: KnowledgeErrorPresentation;
  onRetry?: () => void;
}) {
  return <ErrorNotice error={error} onRetry={onRetry} />;
}

export function KnowledgeErrorModal({ error, onClose, onRetry, retryLabel }: {
  error: KnowledgeErrorPresentation | null;
  onClose: () => void;
  onRetry?: () => void;
  retryLabel?: string;
}) {
  const { t } = useTranslation();
  const retryText = retryLabel ?? (error?.context === 'export'
    ? t('knowledge.errors.chooseLocation', { defaultValue: '重新选择保存位置' })
    : t('common.retry', { defaultValue: '重试' }));

  return <ErrorModal error={error} onClose={onClose} onRetry={onRetry} retryLabel={retryText} />;
}
