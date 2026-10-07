/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageToolCall } from '@/common/chat/chatLib';
import { normalizeToolCall } from '@/common/chat/normalizeToolCall';
import { formatToolDiagnostics, resolveToolPresentation } from '@/common/chat/toolPresentation';
import { IconDown, IconRight } from '@arco-design/web-react/icon';
import { BookOne } from '@icon-park/react';
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import './MessageToolDetails.css';

/** Parse a hit count from the knowledge_search output text
 *  ("N result(s) for …" / "No matches …"). Count, 0 for explicit no-match,
 *  or null when undeterminable (e.g. still running). */
function parseHitCount(output: string | undefined): number | null {
  if (!output) return null;
  const m = output.match(/^\s*(\d+)\s+result/);
  if (m) return Number(m[1]);
  if (/^\s*No matches/.test(output)) return 0;
  return null;
}

const KnowledgeSearchChip: React.FC<{ message: IMessageToolCall }> = ({ message }) => {
  const { t, i18n } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  const query = String(message.content.args?.query ?? message.content.input?.query ?? '').trim();
  const normalized = normalizeToolCall(message);
  const presentation = resolveToolPresentation(normalized ?? { name: message.content.name },
    i18n.resolvedLanguage ?? i18n.language);
  const output = normalized?.output;
  const status = normalized?.status;
  const count = useMemo(() => parseHitCount(output), [output]);

  const statusNode = (() => {
    if (status === 'running') return <span className='text-t-secondary'>{t('knowledge.searchChip.searching')}</span>;
    if (status === 'error') return <span className='text-t-secondary'>{t('knowledge.searchChip.error')}</span>;
    if (count === 0) return <span className='text-t-secondary'>{t('knowledge.searchChip.noHit')}</span>;
    if (count != null && count > 0) return <span className='text-brand'>{t('knowledge.searchChip.hit', { count })}</span>;
    return null;
  })();

  const canExpand = Boolean(normalized);

  return (
    <div className='flex flex-col'>
      <div
        className={'inline-flex items-center gap-6px px-8px py-2px rounded-6px bg-fill-2 text-13px max-w-full' + (canExpand ? ' cursor-pointer hover:bg-3' : '')}
        onClick={canExpand ? () => setExpanded(!expanded) : undefined}
      >
        <span className='flex-shrink-0 inline-flex'>
          <BookOne theme='outline' size='14' fill='currentColor' />
        </span>
        <span className='font-medium text-t-primary flex-shrink-0'>{presentation.title}</span>
        {query && <span className='text-t-secondary truncate'>{t('knowledge.searchChip.query', { query })}</span>}
        {statusNode && <span className='flex-shrink-0 m-l-2px'>{statusNode}</span>}
        {canExpand && <span className='flex-shrink-0 text-t-secondary'>{expanded ? <IconDown style={{ fontSize: 12 }} /> : <IconRight style={{ fontSize: 12 }} />}</span>}
      </div>
      {expanded && canExpand && (
        <div className='tool-detail-panel m-l-20px m-t-4px'>
          <div className='tool-detail-label'>{t('messages.toolDetailIdentity')}</div>
          <pre className='tool-detail-content'>{formatToolDiagnostics(presentation)}</pre>
          {output && <>
            <div className='tool-detail-label'>{t('messages.toolDetailOutput')}</div>
            <pre className='tool-detail-content'>{output}</pre>
          </>}
        </div>
      )}
    </div>
  );
};

export default KnowledgeSearchChip;
