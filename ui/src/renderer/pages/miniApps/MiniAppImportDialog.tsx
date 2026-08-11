/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 「导入小程序」 — adopt an app the user already wrote (spec D14).
 *
 * Three steps, and the order is load-bearing:
 *
 *  1. **Pick a source.** Which picker exists depends on the runtime, not on a
 *     guess: the native dialog returns absolute PATHS and lives only in the
 *     desktop shell, where the picker and the backend share a filesystem. A WebUI
 *     browser session has no such bridge — and `dialog.showOpen`'s web fallback
 *     goes to `bridge.invoke`, whose promise never settles without a responder —
 *     so that runtime gets `<input type='file'>` and sends the document's BYTES
 *     instead. Folder import therefore exists on desktop only: a browser cannot
 *     produce a directory's contents, and a path from the user's machine would
 *     mean nothing to a remote backend.
 *  2. **Validate.** `POST /api/miniapps/validate` writes nothing and always
 *     answers 200 for a readable candidate, so the report is rendered before the
 *     user commits to anything. This also keeps the import call off the failure
 *     path: `POST /api/miniapps/import` answers a blocked candidate with 400, and
 *     while {@link miniAppImportReportFromError} can recover the report from that
 *     body, relying on it would make a rejection look like a network fault.
 *  3. **Import**, or 「用会话改造」 when the report is blocked — a fatal finding is
 *     a rewrite job, not a retry, so the way forward is a conversation that turns
 *     the app into one self-contained document.
 *
 * Every finding renders its own sentence keyed by `rule_id`
 * ({@link resolveMiniAppImportRuleKeys}); the backend deliberately sends no prose.
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Modal, Spin } from '@arco-design/web-react';
import { Attention, CheckOne, CloseOne, Comment, FileCode, FolderClose, Upload } from '@icon-park/react';
import { ipcBridge } from '@/common';
import {
  miniAppImportReportFromError,
  type IApiMiniApp,
  type IApiMiniAppImportFinding,
  type IApiMiniAppImportRequest,
  type IApiMiniAppImportResponse,
  type IApiMiniAppImportSeverity,
} from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';
import type { I18nKey } from '@/renderer/services/i18n';
import { useArcoMessage } from '@renderer/utils/ui/useArcoMessage';
import { useNomiQuickStart } from '@renderer/hooks/agent/useNomiQuickStart';
import { MINI_APP_EXTRA_FLAG, MINI_APP_NAME_SNIPPET_LENGTH } from './contract';
import {
  MINI_APP_IMPORT_CONVERSION_SYSTEM_PROMPT,
  buildMiniAppImportConversionPrompt,
  miniAppImportSourceBaseName,
  type MiniAppImportConversionSource,
} from './importConversion';
import {
  formatMiniAppImportBytes,
  formatMiniAppImportDetail,
  groupMiniAppImportFindings,
  isMiniAppImportRuleId,
  resolveMiniAppImportRuleKeys,
} from './importReport';

/**
 * Mirrors `MINI_APP_HTML_MAX_BYTES` (4 MiB) in
 * `crates/backend/nomifun-miniapp/src/service.rs`.
 *
 * Used ONLY to pre-empt the inline flow: an oversized `html` body is refused by
 * the service as a plain BadRequest *before* the validator runs, so without this
 * the user would upload megabytes to receive an untranslated English sentence.
 * Reporting it as the `size_over_limit` finding the validator would have produced
 * keeps one explanation for one problem. The backend still has the last word.
 */
const MINI_APP_IMPORT_MAX_BYTES = 4 * 1024 * 1024;

/** What the user picked. `size` is only known for the byte flow. */
type MiniAppImportSource =
  | { kind: 'path'; path: string; folder: boolean }
  | { kind: 'html'; fileName: string; html: string; size: number };

interface SeverityStyle {
  headerKey: I18nKey;
  hintKey: I18nKey;
  /** Complete literal class strings — never assembled from fragments. */
  cardClass: string;
  iconClass: string;
  icon: React.ReactNode;
}

const SEVERITY_STYLE: Record<IApiMiniAppImportSeverity, SeverityStyle> = {
  fatal: {
    headerKey: 'miniApps.import.severity.fatal',
    hintKey: 'miniApps.import.severity.fatalHint',
    cardClass:
      'rounded-12px border border-solid border-[rgba(var(--danger-6),0.32)] bg-[rgba(var(--danger-6),0.06)] p-12px',
    iconClass: 'mt-2px shrink-0 text-danger-6',
    icon: <CloseOne theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />,
  },
  autofix: {
    headerKey: 'miniApps.import.severity.autofix',
    hintKey: 'miniApps.import.severity.autofixHint',
    cardClass:
      'rounded-12px border border-solid border-[rgba(var(--primary-6),0.32)] bg-[rgba(var(--primary-6),0.08)] p-12px',
    iconClass: 'mt-2px shrink-0 text-primary-6',
    icon: <CheckOne theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />,
  },
  warning: {
    headerKey: 'miniApps.import.severity.warning',
    hintKey: 'miniApps.import.severity.warningHint',
    cardClass:
      'rounded-12px border border-solid border-[rgba(var(--warning-6),0.32)] bg-[rgba(var(--warning-6),0.08)] p-12px',
    iconClass: 'mt-2px shrink-0 text-warning-6',
    icon: <Attention theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />,
  },
};

/** Neutral bucket for a severity a newer backend invented. */
const UNKNOWN_SEVERITY_STYLE: SeverityStyle = {
  headerKey: 'miniApps.import.severity.other',
  hintKey: 'miniApps.import.severity.otherHint',
  cardClass:
    'rounded-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-2)] p-12px',
  iconClass: 'mt-2px shrink-0 text-[var(--color-text-3)]',
  icon: <Attention theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />,
};

const styleFor = (severity: IApiMiniAppImportSeverity): SeverityStyle =>
  SEVERITY_STYLE[severity] ?? UNKNOWN_SEVERITY_STYLE;

const SOURCE_BUTTON_CLASS = [
  'flex flex-1 min-w-160px items-center gap-10px rounded-12px p-12px cursor-pointer text-left',
  'border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] font-[inherit]',
  'transition-colors hover:border-[var(--color-border-3)] hover:bg-[var(--color-fill-2)]',
].join(' ');

const DETAIL_CHIP_CLASS =
  'inline-block max-w-full break-all rounded-6px bg-[var(--color-fill-2)] px-6px py-1px font-mono text-11px text-[var(--color-text-2)]';

export interface MiniAppImportDialogProps {
  visible: boolean;
  onCancel: () => void;
  /** Ran once the backend adopted the app, with the record it echoed back. */
  onImported: (app: IApiMiniApp) => void;
}

const MiniAppImportDialog: React.FC<MiniAppImportDialogProps> = ({ visible, onCancel, onImported }) => {
  const { t } = useTranslation();
  const [message, messageHolder] = useArcoMessage();
  const { start: startConversion } = useNomiQuickStart();
  const fileInputRef = useRef<HTMLInputElement>(null);

  const [source, setSource] = useState<MiniAppImportSource | null>(null);
  const [result, setResult] = useState<IApiMiniAppImportResponse | null>(null);
  const [validating, setValidating] = useState(false);
  const [importing, setImporting] = useState(false);
  const [converting, setConverting] = useState(false);
  /** Transport / rejection text that is NOT a report — rendered as one line. */
  const [error, setError] = useState<string | null>(null);

  // The native picker is the only path-capable one, and it exists only in the
  // desktop shell. Probed rather than assumed: the same bundle serves both.
  const canPickPaths = useMemo(() => isTauriRuntime(), []);

  const resetAll = useCallback(() => {
    setSource(null);
    setResult(null);
    setError(null);
    setValidating(false);
    setImporting(false);
    setConverting(false);
  }, []);

  useEffect(() => {
    if (!visible) resetAll();
  }, [visible, resetAll]);

  const requestFor = useCallback(
    (picked: MiniAppImportSource): IApiMiniAppImportRequest =>
      picked.kind === 'path' ? { path: picked.path } : { html: picked.html },
    []
  );

  /**
   * A rejection that is not a report.
   *
   * One case still deserves the report treatment: a folder with no entry document
   * comes back as `BadRequest("no_root_document")` — the rule id itself, because
   * the entry is decided before any document exists to scan. Rendering it as the
   * fatal finding it is keeps the user in one explanation model.
   */
  const adoptRejection = useCallback((e: unknown): boolean => {
    if (!isBackendHttpError(e)) return false;
    const backendMessage = e.backendMessage.trim();
    if (!isMiniAppImportRuleId(backendMessage)) return false;
    setResult({
      report: { findings: [{ rule_id: backendMessage, severity: 'fatal' }], blocked: true },
      applied_fixes: [],
    });
    return true;
  }, []);

  const failureText = useCallback(
    (e: unknown): string => {
      if (isBackendHttpError(e) && e.backendMessage.trim()) return e.backendMessage;
      return e instanceof Error ? e.message : String(e);
    },
    []
  );

  const runValidate = useCallback(
    async (picked: MiniAppImportSource) => {
      setSource(picked);
      setResult(null);
      setError(null);

      // Oversized inline documents never reach the validator (see the constant).
      if (picked.kind === 'html' && picked.size > MINI_APP_IMPORT_MAX_BYTES) {
        setResult({
          report: {
            findings: [{ rule_id: 'size_over_limit', severity: 'fatal', detail: String(picked.size) }],
            blocked: true,
          },
          applied_fixes: [],
        });
        return;
      }

      setValidating(true);
      try {
        setResult(await ipcBridge.miniapps.validateImport.invoke(requestFor(picked)));
      } catch (e) {
        console.error('[miniapps] validating an import candidate failed', e);
        if (!adoptRejection(e)) setError(t('miniApps.import.errors.validateFailed', { message: failureText(e) }));
      } finally {
        setValidating(false);
      }
    },
    [requestFor, adoptRejection, failureText, t]
  );

  const pickPath = useCallback(
    async (folder: boolean) => {
      try {
        const picked = await ipcBridge.dialog.showOpen.invoke(
          folder
            ? { properties: ['openDirectory'] }
            : { properties: ['openFile'], filters: [{ name: 'HTML', extensions: ['html', 'htm'] }] }
        );
        const path = picked?.[0];
        if (!path) return;
        await runValidate({ kind: 'path', path, folder });
      } catch (e) {
        console.error('[miniapps] the native picker failed', e);
        setError(t('miniApps.import.errors.pickFailed'));
      }
    },
    [runValidate, t]
  );

  const onFileInputChange = useCallback(
    async (event: React.ChangeEvent<HTMLInputElement>) => {
      const input = event.target;
      const file = input.files?.[0];
      if (!file) return;
      try {
        const html = await file.text();
        await runValidate({ kind: 'html', fileName: file.name, html, size: file.size });
      } catch (e) {
        console.error('[miniapps] reading the picked file failed', e);
        setError(t('miniApps.import.errors.readFailed'));
      } finally {
        // Cleared only after the read: picking the same file twice must still
        // fire a change event, and clearing first would drop the FileList.
        input.value = '';
      }
    },
    [runValidate, t]
  );

  const runImport = useCallback(async () => {
    if (!source) return;
    setImporting(true);
    setError(null);
    try {
      const response = await ipcBridge.miniapps.importApp.invoke(requestFor(source));
      if (!response.app) {
        // 200 without a record should not happen; show what came back instead of
        // claiming success.
        setResult(response);
        return;
      }
      message.success(t('miniApps.import.success', { name: response.app.name }));
      // Repairs the backend can prove it made, named with the same copy the
      // report used — never the ids the catalogue merely hoped to repair.
      const fixed = response.applied_fixes.map((ruleId) => {
        const keys = resolveMiniAppImportRuleKeys(ruleId);
        return keys ? t(keys.title) : ruleId;
      });
      if (fixed.length > 0) message.info(t('miniApps.import.appliedFixes', { items: fixed.join('; ') }));
      onImported(response.app);
    } catch (e) {
      console.error('[miniapps] importing failed', e);
      // The 400 that refuses a blocked candidate still carries the full report.
      const recovered = miniAppImportReportFromError(e);
      if (recovered) {
        setResult(recovered);
        setError(t('miniApps.import.errors.blockedOnImport'));
      } else if (!adoptRejection(e)) {
        setError(t('miniApps.import.errors.importFailed', { message: failureText(e) }));
      }
    } finally {
      setImporting(false);
    }
  }, [source, requestFor, message, t, onImported, adoptRejection, failureText]);

  const startConversionSession = useCallback(async () => {
    if (!source) return;
    setConverting(true);
    try {
      // Best effort, path flow only: hand the model the entry document's text as
      // well as the path. The path is what lets it read a folder's siblings, the
      // text is what lets it start even if its file tools cannot reach outside the
      // session workspace. A failed read is not an error — the path still stands.
      let document: string | undefined;
      if (source.kind === 'path' && !source.folder) {
        try {
          document = (await ipcBridge.fs.readFile.invoke({ path: source.path })) ?? undefined;
        } catch (e) {
          console.error('[miniapps] could not pre-read the import source; sending the path only', e);
        }
      }
      const conversionSource: MiniAppImportConversionSource =
        source.kind === 'path'
          ? { kind: 'path', path: source.path, ...(document ? { document } : {}) }
          : { kind: 'html', fileName: source.fileName, html: source.html };
      const baseName = miniAppImportSourceBaseName(conversionSource);
      const started = await startConversion({
        name: t('miniApps.import.conversationName', {
          name: Array.from(baseName).slice(0, MINI_APP_NAME_SNIPPET_LENGTH).join(''),
        }),
        prompt: buildMiniAppImportConversionPrompt({
          source: conversionSource,
          findings: result?.report.findings ?? [],
        }),
        extra: {
          system_prompt: MINI_APP_IMPORT_CONVERSION_SYSTEM_PROMPT,
          // Marks the thread as a mini-app build so the produced `miniapp.html`
          // opens in the preview panel with 「发布为小程序」 on it — that is how the
          // rewritten app gets back into the library.
          [MINI_APP_EXTRA_FLAG]: true,
        },
      });
      // `start` already navigated on success; closing keeps the dialog from
      // reappearing over the conversation if the user comes back.
      if (started) onCancel();
    } finally {
      setConverting(false);
    }
  }, [source, result, startConversion, t, onCancel]);

  const report = result?.report ?? null;
  const groups = useMemo(() => groupMiniAppImportFindings(report?.findings ?? []), [report]);
  const blocked = report?.blocked === true;
  const busy = validating || importing || converting;

  const renderFinding = (finding: IApiMiniAppImportFinding, style: SeverityStyle) => {
    const keys = resolveMiniAppImportRuleKeys(finding.rule_id);
    const detail = formatMiniAppImportDetail(finding);
    return (
      <div key={`${finding.severity}-${finding.rule_id}`} className='flex gap-8px'>
        <span className={style.iconClass} aria-hidden='true'>
          {style.icon}
        </span>
        <div className='flex min-w-0 flex-1 flex-col gap-3px'>
          <span className='text-13px font-600 leading-18px text-[var(--color-text-1)]'>
            {keys ? t(keys.title) : t('miniApps.import.rules.unknown.title', { ruleId: finding.rule_id })}
          </span>
          {detail !== null && <span className={DETAIL_CHIP_CLASS}>{detail}</span>}
          <span className='text-12px leading-18px text-[var(--color-text-3)]'>
            {keys
              ? t(keys.fix, { detail: detail ?? t('miniApps.import.detailUnknown') })
              : t('miniApps.import.rules.unknown.fix')}
          </span>
        </div>
      </div>
    );
  };

  return (
    <>
      {messageHolder}
      <Modal
        title={t('miniApps.import.title')}
        visible={visible}
        onCancel={onCancel}
        autoFocus={false}
        unmountOnExit
        style={{ width: 620, maxWidth: '92vw' }}
        footer={
          <div className='flex items-center justify-between gap-8px'>
            <div className='flex items-center gap-8px'>
              {source !== null && (
                <Button disabled={busy} onClick={resetAll}>
                  {t('miniApps.import.reselect')}
                </Button>
              )}
            </div>
            <div className='flex items-center gap-8px'>
              <Button disabled={busy} onClick={onCancel}>
                {t('common.cancel')}
              </Button>
              {blocked && (
                <Button loading={converting} disabled={importing || validating} onClick={() => void startConversionSession()}>
                  <span className='inline-flex items-center gap-6px'>
                    <Comment theme='outline' size='14' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                    {t('miniApps.import.convert')}
                  </span>
                </Button>
              )}
              <Button
                type='primary'
                loading={importing}
                disabled={source === null || report === null || blocked || validating || converting}
                onClick={() => void runImport()}
              >
                {t('miniApps.import.action')}
              </Button>
            </div>
          </div>
        }
      >
        <div className='flex max-h-[56vh] flex-col gap-12px overflow-y-auto pr-2px'>
          <p className='m-0 text-12px leading-18px text-[var(--color-text-3)]'>{t('miniApps.import.intro')}</p>

          {/* Step 1 — source */}
          {source === null ? (
            <div className='flex flex-col gap-8px'>
              <div className='flex flex-wrap gap-10px'>
                {canPickPaths ? (
                  <>
                    <button type='button' className={SOURCE_BUTTON_CLASS} onClick={() => void pickPath(false)}>
                      <FileCode theme='outline' size='18' fill='currentColor' className='block text-primary-6' style={{ lineHeight: 0 }} />
                      <span className='flex min-w-0 flex-col gap-2px'>
                        <span className='text-13px font-600 text-[var(--color-text-1)]'>{t('miniApps.import.pickFile')}</span>
                        <span className='text-11px text-[var(--color-text-3)]'>{t('miniApps.import.pickFileHint')}</span>
                      </span>
                    </button>
                    <button type='button' className={SOURCE_BUTTON_CLASS} onClick={() => void pickPath(true)}>
                      <FolderClose theme='outline' size='18' fill='currentColor' className='block text-primary-6' style={{ lineHeight: 0 }} />
                      <span className='flex min-w-0 flex-col gap-2px'>
                        <span className='text-13px font-600 text-[var(--color-text-1)]'>{t('miniApps.import.pickFolder')}</span>
                        <span className='text-11px text-[var(--color-text-3)]'>{t('miniApps.import.pickFolderHint')}</span>
                      </span>
                    </button>
                  </>
                ) : (
                  <button type='button' className={SOURCE_BUTTON_CLASS} onClick={() => fileInputRef.current?.click()}>
                    <Upload theme='outline' size='18' fill='currentColor' className='block text-primary-6' style={{ lineHeight: 0 }} />
                    <span className='flex min-w-0 flex-col gap-2px'>
                      <span className='text-13px font-600 text-[var(--color-text-1)]'>{t('miniApps.import.pickUpload')}</span>
                      <span className='text-11px text-[var(--color-text-3)]'>{t('miniApps.import.pickUploadHint')}</span>
                    </span>
                  </button>
                )}
              </div>
              {!canPickPaths && (
                <span className='text-11px leading-16px text-[var(--color-text-3)]'>{t('miniApps.import.webUiOnlyFile')}</span>
              )}
            </div>
          ) : (
            <div className='flex items-center gap-8px rounded-10px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-2)] px-10px py-8px'>
              <span className='shrink-0 text-[var(--color-text-3)]' aria-hidden='true'>
                {source.kind === 'path' && source.folder ? (
                  <FolderClose theme='outline' size='14' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                ) : (
                  <FileCode theme='outline' size='14' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                )}
              </span>
              <span className='min-w-0 flex-1 break-all text-12px leading-17px text-[var(--color-text-2)]'>
                {source.kind === 'path'
                  ? source.path
                  : t('miniApps.import.sourceFile', {
                      name: source.fileName,
                      size: formatMiniAppImportBytes(source.size),
                    })}
              </span>
            </div>
          )}

          {/* Step 2 — the report */}
          {validating && (
            <div className='flex items-center gap-8px py-8px text-12px text-[var(--color-text-3)]'>
              <Spin size={14} />
              {t('miniApps.import.validating')}
            </div>
          )}

          {error !== null && (
            <div className='rounded-10px border border-solid border-[rgba(var(--danger-6),0.32)] bg-[rgba(var(--danger-6),0.06)] px-10px py-8px text-12px leading-18px text-danger-6'>
              {error}
            </div>
          )}

          {report !== null && groups.length === 0 && (
            <div className='flex items-center gap-8px rounded-10px border border-solid border-[rgba(var(--primary-6),0.32)] bg-[rgba(var(--primary-6),0.08)] px-10px py-8px text-12px text-[var(--color-text-2)]'>
              <CheckOne theme='outline' size='15' fill='currentColor' className='block text-primary-6' style={{ lineHeight: 0 }} />
              {t('miniApps.import.clean')}
            </div>
          )}

          {groups.map((group) => {
            const style = styleFor(group.severity);
            return (
              <div key={group.severity} className={style.cardClass}>
                <div className='mb-8px flex flex-col gap-2px'>
                  <span className='text-12px font-600 leading-17px text-[var(--color-text-1)]'>
                    {t(style.headerKey, { total: group.findings.length })}
                  </span>
                  <span className='text-11px leading-16px text-[var(--color-text-3)]'>{t(style.hintKey)}</span>
                </div>
                <div className='flex flex-col gap-10px'>
                  {group.findings.map((finding) => renderFinding(finding, style))}
                </div>
              </div>
            );
          })}

          {blocked && (
            <p className='m-0 text-12px leading-18px text-[var(--color-text-3)]'>{t('miniApps.import.convertHint')}</p>
          )}
        </div>

        <input ref={fileInputRef} type='file' accept='.html,.htm,text/html' hidden onChange={(e) => void onFileInputChange(e)} />
      </Modal>
    </>
  );
};

export default MiniAppImportDialog;
