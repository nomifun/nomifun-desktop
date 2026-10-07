/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLatestRef } from '@/renderer/hooks/ui/useLatestRef';
import { transcribeAudioBlob } from '@/renderer/services/SpeechToTextService';
import { isDesktopShell } from '@/renderer/utils/platform';
import {
  getSpeechInputAvailabilityForEnvironment,
  type SpeechInputEnvironment,
} from './speechInputAvailability';

export type { SpeechInputAvailability } from './speechInputAvailability';
export type SpeechInputStatus = 'idle' | 'recording' | 'transcribing' | 'error';
export type SpeechInputErrorCode =
  | 'aborted'
  | 'audio-capture'
  | 'empty-transcript'
  | 'file-too-large'
  | 'network'
  | 'not-configured'
  | 'permission-denied'
  | 'recording-unsupported'
  | 'transcription-failed'
  | 'unknown';

type UseSpeechInputOptions = {
  locale?: string;
  onTranscript: (transcript: string) => void;
};

/**
 * Browser-preference order: whichever the platform can actually encode wins.
 * Reordering this to suit one provider is a per-platform lottery — Chromium
 * cannot record Ogg/Opus and Linux/WebKitGTK cannot record AAC, so a
 * provider-driven order silently falls through to a different container per
 * platform. The provider-compatibility half of this belongs in the ASR
 * adapters, which normalize the recorded MIME (parameters included) instead.
 */
const RECORDING_MIME_TYPES = ['audio/webm;codecs=opus', 'audio/webm', 'audio/mp4', 'audio/ogg;codecs=opus'];
const SPEECH_WAVEFORM_SAMPLE_COUNT = 40;
const SPEECH_WAVEFORM_MIN_LEVEL = 0.015;
const SPEECH_WAVEFORM_MAX_LEVEL = 1;
const SPEECH_VISUALIZER_INTERVAL_MS = 80;

const createInitialWaveformLevels = (): number[] =>
  Array.from({ length: SPEECH_WAVEFORM_SAMPLE_COUNT }, (_, index) => ((index + 1) % 6 === 0 ? 0.04 : 0.015));

const clampWaveformLevel = (value: number): number =>
  Math.max(SPEECH_WAVEFORM_MIN_LEVEL, Math.min(SPEECH_WAVEFORM_MAX_LEVEL, value));

const createNextWaveformLevels = (previous: number[], nextLevel: number): number[] => [
  ...previous.slice(1),
  clampWaveformLevel(nextLevel),
];

export const appendSpeechTranscript = (base: string, transcript: string): string => {
  const normalizedTranscript = transcript.trim();
  if (!normalizedTranscript) {
    return base;
  }

  const normalizedBase = base.trimEnd();
  if (!normalizedBase) {
    return normalizedTranscript;
  }

  return `${normalizedBase}\n${normalizedTranscript}`;
};

const getSpeechInputEnvironment = (): SpeechInputEnvironment => {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return {
      hasMediaDevices: false,
      hasMediaRecorder: false,
      hostname: '',
      isDesktopShell: false,
      isSecureContext: false,
    };
  }

  return {
    hasMediaDevices: typeof navigator !== 'undefined' && Boolean(navigator.mediaDevices?.getUserMedia),
    hasMediaRecorder: typeof MediaRecorder !== 'undefined',
    hostname: window.location.hostname,
    isDesktopShell: isDesktopShell(),
    isSecureContext: window.isSecureContext,
  };
};

export const getSpeechInputAvailability = () => {
  return getSpeechInputAvailabilityForEnvironment(getSpeechInputEnvironment());
};

export const pickRecordingMimeType = (): string => {
  if (typeof MediaRecorder === 'undefined' || typeof MediaRecorder.isTypeSupported !== 'function') {
    return '';
  }

  return RECORDING_MIME_TYPES.find((mimeType) => MediaRecorder.isTypeSupported(mimeType)) || '';
};

const mapSpeechInputError = (error: unknown): SpeechInputErrorCode => {
  if (error instanceof DOMException) {
    switch (error.name) {
      case 'NotAllowedError':
      case 'SecurityError':
        return 'permission-denied';
      case 'NotFoundError':
      case 'DevicesNotFoundError':
        return 'audio-capture';
      case 'AbortError':
        return 'aborted';
      default:
        return 'unknown';
    }
  }

  const message = error instanceof Error ? error.message : String(error);

  if (
    message.includes('STT_DISABLED') ||
    message.includes('ASR_NOT_CONFIGURED') ||
    message.includes('ASR_NOT_READY')
  ) {
    return 'not-configured';
  }
  if (message.includes('STT_FILE_TOO_LARGE')) {
    return 'file-too-large';
  }
  if (message.includes('STT_NETWORK_ERROR')) {
    return 'network';
  }
  if (message.includes('STT_ABORTED')) {
    return 'aborted';
  }
  if (message.includes('STT_REQUEST_FAILED')) {
    return 'transcription-failed';
  }

  return 'unknown';
};

export const useSpeechInput = ({ locale, onTranscript }: UseSpeechInputOptions) => {
  const [status, setStatus] = useState<SpeechInputStatus>('idle');
  const [errorCode, setErrorCode] = useState<SpeechInputErrorCode | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [recordingDurationMs, setRecordingDurationMs] = useState(0);
  const [recordingLevels, setRecordingLevels] = useState<number[]>(() => createInitialWaveformLevels());
  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const mountedRef = useRef(true);
  const captureEpochRef = useRef(0);
  const transcriptionAbortRef = useRef<AbortController | null>(null);
  const recordingStartedAtRef = useRef<number | null>(null);
  const visualizerIntervalRef = useRef<number | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const mediaSourceRef = useRef<MediaStreamAudioSourceNode | null>(null);
  const analyserDataRef = useRef<Uint8Array<ArrayBuffer> | null>(null);
  const onTranscriptRef = useLatestRef(onTranscript);
  const availability = useMemo(() => getSpeechInputAvailability(), []);

  const recognitionLocale = locale?.trim() || undefined;

  const pauseSpeechVisualizer = useCallback(() => {
    if (visualizerIntervalRef.current !== null) {
      window.clearInterval(visualizerIntervalRef.current);
      visualizerIntervalRef.current = null;
    }
  }, []);

  const resetSpeechVisualizer = useCallback(() => {
    pauseSpeechVisualizer();
    recordingStartedAtRef.current = null;
    setRecordingDurationMs(0);
    setRecordingLevels(createInitialWaveformLevels());
  }, [pauseSpeechVisualizer]);

  const cleanupAudioAnalysis = useCallback(async () => {
    if (mediaSourceRef.current) {
      try {
        mediaSourceRef.current.disconnect();
      } catch {
        // Ignore disconnect failures during teardown.
      }
      mediaSourceRef.current = null;
    }

    if (analyserRef.current) {
      try {
        analyserRef.current.disconnect();
      } catch {
        // Ignore disconnect failures during teardown.
      }
      analyserRef.current = null;
    }

    analyserDataRef.current = null;

    if (audioContextRef.current) {
      const context = audioContextRef.current;
      audioContextRef.current = null;
      try {
        await context.close();
      } catch {
        // Ignore close failures during teardown.
      }
    }
  }, []);

  const startSpeechVisualizer = useCallback(
    async (stream: MediaStream) => {
      resetSpeechVisualizer();
      recordingStartedAtRef.current = Date.now();

      const AudioContextCtor =
        typeof AudioContext !== 'undefined'
          ? AudioContext
          : typeof window !== 'undefined'
            ? (window as Window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
            : undefined;

      if (AudioContextCtor) {
        try {
          const audioContext = new AudioContextCtor();
          const analyser = audioContext.createAnalyser();
          analyser.fftSize = 128;
          analyser.smoothingTimeConstant = 0.82;
          const source = audioContext.createMediaStreamSource(stream);
          source.connect(analyser);
          audioContextRef.current = audioContext;
          analyserRef.current = analyser;
          mediaSourceRef.current = source;
          analyserDataRef.current = new Uint8Array(analyser.fftSize);
        } catch {
          void cleanupAudioAnalysis();
        }
      }

      visualizerIntervalRef.current = window.setInterval(() => {
        const startedAt = recordingStartedAtRef.current;
        if (startedAt) {
          setRecordingDurationMs(Date.now() - startedAt);
        }

        const analyser = analyserRef.current;
        const analyserData = analyserDataRef.current;
        if (!analyser || !analyserData) {
          setRecordingLevels((previous) => createNextWaveformLevels(previous, SPEECH_WAVEFORM_MIN_LEVEL));
          return;
        }

        analyser.getByteTimeDomainData(analyserData);
        let sum = 0;
        for (const sample of analyserData) {
          const normalized = (sample - 128) / 128;
          sum += normalized * normalized;
        }

        const rms = Math.sqrt(sum / analyserData.length);
        const scaledLevel = clampWaveformLevel(rms * 5.6);
        setRecordingLevels((previous) => createNextWaveformLevels(previous, scaledLevel));
      }, SPEECH_VISUALIZER_INTERVAL_MS);
    },
    [cleanupAudioAnalysis, resetSpeechVisualizer]
  );

  const cleanupRecorder = useCallback(() => {
    pauseSpeechVisualizer();
    if (streamRef.current) {
      streamRef.current.getTracks().forEach((track) => track.stop());
      streamRef.current = null;
    }
    recorderRef.current = null;
    chunksRef.current = [];
    void cleanupAudioAnalysis();
  }, [cleanupAudioAnalysis, pauseSpeechVisualizer]);

  const clearError = useCallback(() => {
    setErrorCode(null);
    setErrorMessage(null);
    setStatus('idle');
    resetSpeechVisualizer();
  }, [resetSpeechVisualizer]);

  const cancel = useCallback(() => {
    captureEpochRef.current += 1;
    transcriptionAbortRef.current?.abort(); transcriptionAbortRef.current = null;
    const recorder = recorderRef.current;
    if (recorder) { recorder.ondataavailable = null; recorder.onerror = null; recorder.onstop = null; }
    if (recorder && recorder.state !== 'inactive') { try { recorder.stop(); } catch { /* Partially started recorder. */ } }
    cleanupRecorder();
    if (mountedRef.current) { setStatus('idle'); setErrorCode(null); setErrorMessage(null); resetSpeechVisualizer(); }
  }, [cleanupRecorder, resetSpeechVisualizer]);

  const transcribeBlob = useCallback(
    async (blob: Blob) => {
      const epoch = captureEpochRef.current;
      transcriptionAbortRef.current?.abort();
      const abort = new AbortController(); transcriptionAbortRef.current = abort;
      try {
        setStatus('transcribing');
        setErrorCode(null);
        setErrorMessage(null);
        const result = await transcribeAudioBlob(blob, recognitionLocale, abort.signal);
        if (!mountedRef.current || epoch !== captureEpochRef.current || abort.signal.aborted) return;
        const transcript = result.text.trim();
        if (!transcript) {
          setErrorCode('empty-transcript');
          setErrorMessage(null);
          setStatus('error');
          resetSpeechVisualizer();
          return;
        }
        onTranscriptRef.current(transcript);
        setStatus('idle');
        resetSpeechVisualizer();
      } catch (error) {
        if (!mountedRef.current || epoch !== captureEpochRef.current || abort.signal.aborted) return;
        setErrorCode(mapSpeechInputError(error));
        const message = error instanceof Error ? error.message : String(error);
        setErrorMessage(
          message.startsWith('STT_REQUEST_FAILED:') ? message.replace('STT_REQUEST_FAILED:', '').trim() : null
        );
        setStatus('error');
        resetSpeechVisualizer();
      } finally {
        if (transcriptionAbortRef.current === abort) transcriptionAbortRef.current = null;
      }
    },
    [onTranscriptRef, recognitionLocale, resetSpeechVisualizer]
  );

  const transcribeRecordedBlob = useCallback(
    async (blob: Blob) => {
      await transcribeBlob(blob);
    },
    [transcribeBlob]
  );

  const startRecording = useCallback(async () => {
    if (availability !== 'record') {
      setErrorCode('recording-unsupported');
      setStatus('error');
      return;
    }

    cancel();
    const epoch = captureEpochRef.current;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      if (!mountedRef.current || epoch !== captureEpochRef.current) { stream.getTracks().forEach(track => track.stop()); return; }
      const mimeType = pickRecordingMimeType();
      const recorder = mimeType ? new MediaRecorder(stream, { mimeType }) : new MediaRecorder(stream);

      streamRef.current = stream;
      recorderRef.current = recorder;
      chunksRef.current = [];
      await startSpeechVisualizer(stream);
      if (!mountedRef.current || epoch !== captureEpochRef.current) { stream.getTracks().forEach(track => track.stop()); return; }

      recorder.ondataavailable = (event) => {
        if (!mountedRef.current || epoch !== captureEpochRef.current) return;
        if (event.data.size > 0) {
          chunksRef.current.push(event.data);
        }
      };

      recorder.onerror = () => {
        if (!mountedRef.current || epoch !== captureEpochRef.current) return;
        cleanupRecorder();
        setErrorCode('unknown');
        setStatus('error');
      };

      recorder.onstop = () => {
        if (!mountedRef.current || epoch !== captureEpochRef.current) return;
        const audioBlob = new Blob(chunksRef.current, {
          type: recorder.mimeType || mimeType || 'audio/webm',
        });
        cleanupRecorder();
        void transcribeRecordedBlob(audioBlob);
      };

      setErrorCode(null);
      setErrorMessage(null);
      setStatus('recording');
      recorder.start();
    } catch (error) {
      if (!mountedRef.current || epoch !== captureEpochRef.current) return;
      cleanupRecorder();
      setErrorCode(mapSpeechInputError(error));
      setErrorMessage(null);
      setStatus('error');
      resetSpeechVisualizer();
    }
  }, [availability, cancel, cleanupRecorder, resetSpeechVisualizer, startSpeechVisualizer, transcribeRecordedBlob]);

  const stopRecording = useCallback(() => {
    const recorder = recorderRef.current;
    if (!recorder || status !== 'recording') {
      return;
    }

    setStatus('transcribing');
    recorder.stop();
  }, [status]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false; captureEpochRef.current += 1;
      transcriptionAbortRef.current?.abort(); transcriptionAbortRef.current = null;
      const recorder = recorderRef.current;
      if (recorder) {
        recorder.ondataavailable = null;
        recorder.onerror = null;
        recorder.onstop = null;
      }
      if (recorder && recorder.state !== 'inactive') {
        try {
          recorder.stop();
        } catch {
          // Ignore teardown failures from partially started recording sessions.
        }
      }
      cleanupRecorder();
    };
  }, [cleanupRecorder]);

  return {
    availability,
    cancel,
    clearError,
    errorCode,
    errorMessage,
    recordingDurationMs,
    recordingLevels,
    startRecording,
    status,
    stopRecording,
  };
};
