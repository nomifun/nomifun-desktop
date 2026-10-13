import type { ModelFailureDiagnostic } from '@/common/chat/providerDiagnostic';
import React from 'react';
import '../messages.css';

export type DiagnosticMetadataRow = readonly [label: string, value: string | undefined];

/** Shared evidence fields; classification, recovery and billing stay with callers. */
export function modelFailureDiagnosticRows(
  diagnostic: ModelFailureDiagnostic | undefined,
  label: (key: string) => string,
): DiagnosticMetadataRow[] {
  return ([
    ['httpStatus', diagnostic?.httpStatus], ['providerId', diagnostic?.providerId],
    ['endpoint', diagnostic?.endpoint], ['protocol', diagnostic?.protocol],
    ['authScheme', diagnostic?.authScheme], ['providerCode', diagnostic?.providerCode],
    ['providerType', diagnostic?.providerType], ['providerParam', diagnostic?.providerParam],
    ['requestId', diagnostic?.requestId], ['retryAfterMs', diagnostic?.retryAfterMs], ['contentType', diagnostic?.contentType],
  ] as const).filter(([, value]) => value !== undefined).map(([key, value]) => [label(key), String(value)]);
}

/** Pure details renderer. It performs no requests, policy decisions or actions. */
const ErrorDiagnosticDetails: React.FC<{
  metadata: readonly DiagnosticMetadataRow[];
  unavailable: string;
  detail?: string;
  detailLabel: string;
}> = ({ metadata, unavailable, detail, detailLabel }) => <>
  <dl className='message-error-note__context'>
    {metadata.map(([label, value]) => <div className='message-error-note__context-row' key={label}>
      <dt>{label}</dt><dd>{value || unavailable}</dd>
    </div>)}
  </dl>
  {detail && <div className='message-error-note__diagnostic'>
    <div className='message-error-note__detail-label'>{detailLabel}</div>
    <pre className='message-error-note__detail-body'>{detail}</pre>
  </div>}
</>;

export default ErrorDiagnosticDetails;
