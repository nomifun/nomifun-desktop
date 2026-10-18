/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Alert, Button, Input, Modal, Progress, Slider, Spin, Steps } from '@arco-design/web-react';
import { Upload } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { IFigureMeta } from '@/common/adapter/ipcBridge';
import { uploadFileViaHttp } from '@renderer/services/FileService';
import { MattingClient, createBitmapClamped } from '@renderer/services/matting/mattingClient';
import type { MatteFallbackReason, MatteMethod, MatteResponse } from '@renderer/services/matting/mattingProtocol';
import FrameStep, { clampHeadBox, type HeadBox } from './FrameStep';

/**
 * CustomFigureWizard — DIY figure three-step modal: pick (file/paste) →
 * matte (Worker ML-first cutout with heuristic redo) → frame (head-box + size
 * tier + name), finishing by uploading the cutout to the shared **figure
 * library** (`POST /api/companion/figures`). Decoupled from any companion: `onDone` hands
 * back the new figure so the caller can select it for a companion (or just keep it
 * in the library).
 */

type Step = 'pick' | 'matte' | 'frame';

interface MatteOutcome {
  blob: Blob;
  url: string;
  aspect: number;
  method: MatteMethod;
}

const ACCEPT = '.jpg,.jpeg,.png,.webp,.gif';
const ACCEPT_EXT = ['jpg', 'jpeg', 'png', 'webp', 'gif'];
const ACCEPT_MIME = ['image/jpeg', 'image/png', 'image/webp', 'image/gif'];
/** "Use original" path mirrors the worker's output clamp. */
const MAX_OUTPUT_EDGE = 2048;
const DEFAULT_TOLERANCE = 11;
/** Below this cutout aspect (w/h) the figure is too narrow — warn, don't block. */
const NARROW_ASPECT = 0.3;

const FALLBACK_MESSAGES = {
  model_unavailable: 'nomi.customFigure.modelUnavailable',
  inference_failed: 'nomi.customFigure.inferenceFailed',
  inference_timeout: 'nomi.customFigure.inferenceTimeout',
  unsuitable_image: 'nomi.customFigure.unsuitableImage',
} as const;

const isAcceptedImage = (f: File): boolean => {
  if (ACCEPT_MIME.includes(f.type)) return true;
  const ext = f.name.split('.').pop()?.toLowerCase() ?? '';
  return ACCEPT_EXT.includes(ext);
};

/** "Use original" — encode the unmatted image (long edge ≤2048) as the figure. */
async function encodeOriginal(file: File): Promise<{ blob: Blob; aspect: number }> {
  const bitmap = await createBitmapClamped(file, MAX_OUTPUT_EDGE);
  const canvas = document.createElement('canvas');
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  const ctx = canvas.getContext('2d');
  const aspect = bitmap.width / bitmap.height;
  try {
    if (!ctx) throw new Error('canvas 2d context unavailable');
    ctx.drawImage(bitmap, 0, 0);
  } finally {
    bitmap.close();
  }
  // Safari can't encode webp — it silently falls back to png, which the
  // backend ingest accepts too (magic-sniffed, extension stays .webp).
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/webp', 0.9));
  if (!blob) throw new Error('image encode failed');
  return { blob, aspect };
}

const defaultHeadBoxFor = (aspect: number): HeadBox => clampHeadBox((1 - 0.3) / 2, 0, 0.3, 0.3 * aspect);

const round4 = (v: number): number => Math.round(v * 10000) / 10000;

export interface CustomFigureWizardProps {
  open: boolean;
  onClose: () => void;
  /** A new library figure was created (uploaded to /api/companion/figures). */
  onDone: (figure: IFigureMeta) => void;
}

const CustomFigureWizard: React.FC<CustomFigureWizardProps> = ({ open, onClose, onDone }) => {
  const { t } = useTranslation();

  const [step, setStep] = useState<Step>('pick');
  const [pickError, setPickError] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ phase: 'download' | 'infer' | 'process'; loaded?: number; total?: number } | null>(null);
  const [matteFailed, setMatteFailed] = useState(false);
  const [processingFailed, setProcessingFailed] = useState(false);
  const [fallbackReason, setFallbackReason] = useState<MatteFallbackReason | null>(null);
  const [result, setResult] = useState<MatteOutcome | null>(null);
  const [tolerance, setTolerance] = useState(DEFAULT_TOLERANCE);
  const [headBox, setHeadBox] = useState<HeadBox>({ x: 0.35, y: 0, w: 0.3, h: 0.3 });
  // Default to the smallest tier: a desktop pet should start compact and not
  // hog the desktop; the user can size up to 中/大 in the picker if they want.
  const [sizeTier, setSizeTier] = useState<'s' | 'm' | 'l'>('s');
  const [name, setName] = useState('');
  const [frameError, setFrameError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [originalProcessing, setOriginalProcessing] = useState(false);
  const originalBusyRef = useRef(false);

  const fileRef = useRef<File | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const clientRef = useRef<MattingClient | null>(null);
  /** Monotonic request id — stale worker responses and aborted decodes are ignored. */
  const reqIdRef = useRef(0);

  /** Swap the matte outcome, revoking the previous preview object URL. */
  const swapResult = useCallback((next: MatteOutcome | null) => {
    setResult((prev) => {
      if (prev && prev.url !== next?.url) URL.revokeObjectURL(prev.url);
      return next;
    });
  }, []);

  const failBackToPick = useCallback(
    (detail: unknown) => {
      setStep('pick');
      setProgress(null);
      setPickError(`${t('nomi.customFigure.errFailed')}${detail ? `: ${String(detail)}` : ''}`);
    },
    [t]
  );

  const handleWorkerMessage = useCallback(
    (msg: MatteResponse) => {
      if (msg.id !== reqIdRef.current) return; // stale run
      if (msg.type === 'progress') {
        setProgress({ phase: msg.phase, loaded: msg.loaded, total: msg.total });
        return;
      }
      if (msg.type === 'fallback') {
        setFallbackReason(msg.reason);
        if (msg.detail) console.warn(`[matting] ${msg.reason}:`, msg.detail);
        return;
      }
      if (msg.type === 'done') {
        setProgress(null);
        const url = URL.createObjectURL(msg.blob);
        swapResult({ blob: msg.blob, url, aspect: msg.aspect, method: msg.method });
        // The matting worker suggests a square box ({x,y,w}); seed h as the
        // square height (w·aspect) — the user can then stretch it to any rectangle.
        setHeadBox(clampHeadBox(msg.headBox.x, msg.headBox.y, msg.headBox.w, msg.headBox.w * msg.aspect));
        // Transparent sources skip the matte review entirely.
        if (msg.method === 'passthrough') setStep('frame');
        return;
      }
      // error
      if (msg.message === 'MATTE_FAILED') {
        setProgress(null);
        setMatteFailed(true); // stay on the matte step: re-matte or pick another image
      } else {
        console.warn('[matting] Processing failed:', msg.message);
        setProgress(null);
        setProcessingFailed(true);
      }
    },
    [swapResult]
  );
  const handleWorkerMessageRef = useRef(handleWorkerMessage);
  handleWorkerMessageRef.current = handleWorkerMessage;

  const runMatte = useCallback(
    (file: File, mode: 'auto' | 'heuristic', tol: number) => {
      setStep('matte');
      setMatteFailed(false);
      setProcessingFailed(false);
      setFallbackReason(null);
      swapResult(null);
      setProgress({ phase: 'process' });
      const id = ++reqIdRef.current;
      clientRef.current ??= new MattingClient();
      clientRef.current.run({ id, file, mode, tolerance: tol }, msg => handleWorkerMessageRef.current(msg));
    },
    [swapResult]
  );

  const handleFile = useCallback(
    (file: File) => {
      if (!isAcceptedImage(file)) {
        setPickError(t('nomi.customFigure.errFailed'));
        return;
      }
      setPickError(null);
      fileRef.current = file;
      void runMatte(file, 'auto', tolerance);
    },
    [runMatte, t, tolerance]
  );

  const redoHeuristic = useCallback(() => {
    const file = fileRef.current;
    if (file) void runMatte(file, 'heuristic', tolerance);
  }, [runMatte, tolerance]);

  const useOriginal = useCallback(async () => {
    const file = fileRef.current;
    if (!file || originalBusyRef.current) return;
    originalBusyRef.current = true;
    setOriginalProcessing(true);
    clientRef.current?.cancel();
    setMatteFailed(false);
    setProcessingFailed(false);
    setFallbackReason(null);
    setProgress({ phase: 'process' });
    const id = ++reqIdRef.current;
    try {
      const { blob, aspect } = await encodeOriginal(file);
      if (id !== reqIdRef.current) return;
      setProgress(null);
      swapResult({ blob, url: URL.createObjectURL(blob), aspect, method: 'passthrough' });
      setHeadBox(defaultHeadBoxFor(aspect));
      setStep('frame');
    } catch (err) {
      if (id === reqIdRef.current) failBackToPick(err);
    } finally {
      originalBusyRef.current = false;
      setOriginalProcessing(false);
    }
  }, [failBackToPick, swapResult]);

  const backToPick = useCallback(() => {
    reqIdRef.current++;
    clientRef.current?.cancel();
    swapResult(null);
    setProgress(null);
    setMatteFailed(false);
    setProcessingFailed(false);
    setFallbackReason(null);
    setPickError(null);
    setStep('pick');
  }, [swapResult]);

  /** Confirm: upload webp → create a reusable library figure. Failure keeps the frame step retryable. */
  const confirm = useCallback(async () => {
    if (!result || submitting) return;
    setSubmitting(true);
    setFrameError(null);
    try {
      const sourcePath = await uploadFileViaHttp(new File([result.blob], 'figure.webp', { type: result.blob.type || 'image/webp' }));
      const figure = await ipcBridge.companion.createFigure.invoke({
        source_path: sourcePath,
        name: name.trim(),
        aspect: round4(result.aspect),
        head_box: { x: round4(headBox.x), y: round4(headBox.y), w: round4(headBox.w), h: round4(headBox.h) },
        size_tier: sizeTier,
      });
      onDone(figure);
    } catch (err) {
      setFrameError(
        err instanceof Error && err.message === 'FILE_TOO_LARGE'
          ? t('nomi.customFigure.errTooLarge')
          : `${t('nomi.customFigure.errFailed')}: ${String(err)}`
      );
    } finally {
      setSubmitting(false);
    }
  }, [headBox, name, onDone, result, sizeTier, submitting, t]);

  // Reset to a clean pick step on every open.
  useEffect(() => {
    if (!open) return;
    setStep('pick');
    setPickError(null);
    setProgress(null);
    setMatteFailed(false);
    setProcessingFailed(false);
    setFallbackReason(null);
    swapResult(null);
    setTolerance(DEFAULT_TOLERANCE);
    setSizeTier('m');
    setName('');
    setFrameError(null);
    setSubmitting(false);
    fileRef.current = null;
  }, [open, swapResult]);

  // Closing invalidates in-flight work and frees the worker (wasm memory).
  useEffect(() => {
    if (open) return;
    reqIdRef.current++;
    clientRef.current?.dispose();
    clientRef.current = null;
    // Free the matte blob + object URL right away — the wizard component stays
    // mounted in its host, so waiting for the next open-reset leaks MBs.
    swapResult(null);
  }, [open, swapResult]);
  useEffect(
    () => () => {
      clientRef.current?.dispose();
      clientRef.current = null;
      setResult((prev) => {
        if (prev) URL.revokeObjectURL(prev.url);
        return null;
      });
    },
    []
  );

  // Paste capture while the pick step is up — capture phase + stopImmediatePropagation
  // keeps the global PasteService (chat sendbox uploads) from also consuming it.
  useEffect(() => {
    if (!open || step !== 'pick') return;
    const onPaste = (e: ClipboardEvent) => {
      const file = e.clipboardData?.files?.[0];
      if (!file || !isAcceptedImage(file)) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      handleFile(file);
    };
    document.addEventListener('paste', onPaste, true);
    return () => document.removeEventListener('paste', onPaste, true);
  }, [open, step, handleFile]);

  const stepIndex = step === 'pick' ? 1 : step === 'matte' ? 2 : 3;
  const progressText =
    progress?.phase === 'download'
      ? t('nomi.customFigure.downloadingModel')
      : progress?.phase === 'infer'
        ? t('nomi.customFigure.matting')
        : t('nomi.customFigure.processing');
  const downloadPercent =
    progress?.phase === 'download' && progress.total
      ? Math.min(100, Math.round(((progress.loaded ?? 0) / progress.total) * 100))
      : null;

  const renderPick = (): React.ReactNode => (
    <div className='flex flex-col gap-12px'>
      <div
        onClick={() => inputRef.current?.click()}
        className='group flex flex-col items-center justify-center gap-12px h-260px rd-16px cursor-pointer transition-all duration-200 border-2px border-solid border-[var(--color-border-2)] bg-gradient-to-b from-[var(--color-fill-1)] to-[var(--color-fill-2)] hover:border-[var(--color-primary)]'
      >
        <span
          className='flex items-center justify-center w-64px h-64px rd-full text-30px transition-all duration-200 bg-primary-1 text-primary-6 group-hover:scale-105'
        >
          <Upload theme='outline' size='14' fill='currentColor' />
        </span>
        <div className='flex flex-col items-center gap-3px'>
          <span className='text-14px font-600 text-t-primary'>{t('nomi.customFigure.dropHint')}</span>
          <span className='text-12px text-t-tertiary'>{t('nomi.customFigure.pasteHint')}</span>
        </div>
      </div>
      <input
        ref={inputRef}
        type='file'
        accept={ACCEPT}
        className='hidden'
        onChange={(e) => {
          const file = e.target.files?.[0];
          if (file) handleFile(file);
          e.target.value = '';
        }}
      />
      {pickError && <Alert type='error' content={pickError} />}
      <span className='text-12px text-t-tertiary text-center'>{t('nomi.customFigure.copyrightHint')}</span>
    </div>
  );

  const renderMatte = (): React.ReactNode => (
    <div className='flex flex-col gap-16px'>
      {progress && (
        <div className='flex flex-col items-center gap-12px py-48px rd-16px bg-fill-1'>
          <Spin size={28} />
          <span className='text-13px font-500 text-t-secondary'>{progressText}</span>
          {downloadPercent != null && (
            <div className='flex flex-col items-center gap-4px w-280px'>
              <Progress percent={downloadPercent} color='var(--color-primary)' trailColor='var(--color-fill-3)' />
              <span className='text-11px text-t-tertiary'>{t('nomi.customFigure.downloadingModel')}</span>
            </div>
          )}
        </div>
      )}
      {!progress && matteFailed && <Alert type='error' content={t('nomi.customFigure.errMatteFailed')} />}
      {!progress && processingFailed && <Alert type='error' content={t('nomi.customFigure.errFailed')} />}
      {!progress && fallbackReason && <Alert type='warning' content={t(FALLBACK_MESSAGES[fallbackReason])} />}
      {!progress && result && (
        <div className='flex gap-16px justify-center flex-wrap'>
          {/* light / dark twin previews — judge the cutout edge on both grounds */}
          <div className='flex flex-col items-center gap-6px'>
            <div className='flex items-center justify-center w-200px h-200px rd-12px bg-[#f4f4f7] border border-solid border-[var(--color-border-2)] overflow-hidden shadow-[0_4px_16px_rgba(0,0,0,0.06)]'>
              <img src={result.url} alt='' draggable={false} className='max-w-[90%] max-h-[90%] object-contain' />
            </div>
            <span className='text-11px text-t-tertiary'>{t('nomi.customFigure.previewLight')}</span>
          </div>
          <div className='flex flex-col items-center gap-6px'>
            <div className='flex items-center justify-center w-200px h-200px rd-12px bg-[#26262b] overflow-hidden shadow-[0_4px_16px_rgba(0,0,0,0.18)]'>
              <img src={result.url} alt='' draggable={false} className='max-w-[90%] max-h-[90%] object-contain' />
            </div>
            <span className='text-11px text-t-tertiary'>{t('nomi.customFigure.previewDark')}</span>
          </div>
        </div>
      )}
      {fileRef.current && (
        <div className='flex items-center gap-12px flex-wrap rd-12px bg-fill-2 px-14px py-10px'>
          <span className='text-13px text-t-secondary shrink-0'>{t('nomi.customFigure.tolerance')}</span>
          <Slider disabled={!!progress} min={4} max={32} value={tolerance} onChange={(v) => setTolerance(v as number)} style={{ width: 150 }} />
          <div className='flex-1' />
          <Button size='small' disabled={!!progress} onClick={() => {
            const file = fileRef.current;
            if (file) runMatte(file, 'auto', tolerance);
          }}>
            {t('nomi.customFigure.retryAi')}
          </Button>
          <Button size='small' disabled={!!progress} onClick={redoHeuristic}>
            {t('nomi.customFigure.redo')}
          </Button>
          <Button size='small' disabled={originalProcessing} loading={originalProcessing} onClick={() => void useOriginal()}>
            {t('nomi.customFigure.useOriginal')}
          </Button>
        </div>
      )}
    </div>
  );

  const renderFrame = (): React.ReactNode =>
    result && (
      <div className='flex flex-col gap-14px'>
        {result.aspect < NARROW_ASPECT && <Alert type='warning' content={t('nomi.customFigure.narrowWarn')} />}
        {frameError && <Alert type='error' content={frameError} />}
        <FrameStep
          imageUrl={result.url}
          aspect={result.aspect}
          headBox={headBox}
          onHeadBoxChange={setHeadBox}
          sizeTier={sizeTier}
          onSizeTierChange={setSizeTier}
        />
        <div className='flex items-center gap-10px rd-12px bg-fill-2 px-14px py-10px'>
          <span className='text-13px text-t-secondary shrink-0'>{t('nomi.customFigure.nameLabel')}</span>
          <Input
            value={name}
            onChange={setName}
            placeholder={t('nomi.customFigure.namePlaceholder')}
            maxLength={40}
            allowClear
            style={{ flex: 1 }}
          />
        </div>
      </div>
    );

  /** Unified footer: compact right-aligned modal actions. */
  const renderFooter = (): React.ReactNode => (
    <div className='figure-modal-footer flex items-center justify-end gap-8px pt-10px'>
      <Button type='text' status='danger' onClick={onClose} disabled={submitting}>
        {t('nomi.customFigure.discard')}
      </Button>
      {step === 'matte' && (
        <>
          <Button onClick={backToPick} disabled={!!progress}>
            {t('nomi.customFigure.repick')}
          </Button>
          <Button type='primary' disabled={!result || !!progress} onClick={() => setStep('frame')}>
            {t('nomi.customFigure.next')}
          </Button>
        </>
      )}
      {step === 'frame' && (
        <>
          <Button onClick={backToPick} disabled={submitting}>
            {t('nomi.customFigure.repick')}
          </Button>
          <Button type='primary' loading={submitting} onClick={() => void confirm()}>
            {submitting ? t('nomi.customFigure.uploading') : t('nomi.customFigure.done')}
          </Button>
        </>
      )}
    </div>
  );

  return (
    <Modal
      title={t('nomi.customFigure.wizardTitle')}
      visible={open}
      onCancel={onClose}
      footer={null}
      maskClosable={false}
      unmountOnExit
      style={{ width: 680 }}
    >
      <div className='flex flex-col gap-18px'>
        <Steps current={stepIndex} size='small'>
          <Steps.Step title={t('nomi.customFigure.stepPick')} />
          <Steps.Step title={t('nomi.customFigure.stepMatte')} />
          <Steps.Step title={t('nomi.customFigure.stepFrame')} />
        </Steps>
        {step === 'pick' && renderPick()}
        {step === 'matte' && renderMatte()}
        {step === 'frame' && renderFrame()}
        {renderFooter()}
      </div>
    </Modal>
  );
};

export default CustomFigureWizard;
