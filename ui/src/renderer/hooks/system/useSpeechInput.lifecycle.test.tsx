import '../../../../test/setup-dom.ts';
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, test } from 'bun:test';
import { FakeSpeechXhr } from '../../../../test/fake-speech-xhr';
import { useSpeechInput } from './useSpeechInput';

const xhrOriginal = globalThis.XMLHttpRequest;
const recorderOriginal = globalThis.MediaRecorder;
const devicesOriginal = Object.getOwnPropertyDescriptor(navigator, 'mediaDevices');
class Recorder {
  static instances: Recorder[] = [];
  static isTypeSupported() { return true; }
  mimeType = 'audio/webm'; state = 'inactive';
  ondataavailable: ((event: { data: Blob }) => void) | null = null;
  onstop: (() => void) | null = null; onerror: (() => void) | null = null;
  constructor() { Recorder.instances.push(this); }
  start() { this.state = 'recording'; }
  stop() { this.state = 'inactive'; this.ondataavailable?.({ data: new Blob(['audio']) }); this.onstop?.(); }
}
beforeEach(() => {
  Recorder.instances = []; FakeSpeechXhr.instances = [];
  globalThis.XMLHttpRequest = FakeSpeechXhr as unknown as typeof XMLHttpRequest;
  globalThis.MediaRecorder = Recorder as unknown as typeof MediaRecorder;
  Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia: async () => ({ getTracks: () => [{ stop: mock() }] }) } });
});
afterEach(() => {
  cleanup(); globalThis.XMLHttpRequest = xhrOriginal; globalThis.MediaRecorder = recorderOriginal;
  if (devicesOriginal) Object.defineProperty(navigator, 'mediaDevices', devicesOriginal); else Reflect.deleteProperty(navigator, 'mediaDevices');
});
function mount() {
  const transcript = mock();
  let hook!: ReturnType<typeof useSpeechInput>;
  function Harness() { hook = useSpeechInput({ locale: 'zh', onTranscript: transcript }); return null; }
  const page = render(<Harness />);
  return { page, transcript, get: () => hook };
}
test('unmount during ASR aborts the upload and excludes a late transcript', async () => {
  const value = mount(); await act(async () => { await value.get().startRecording(); });
  act(() => value.get().stopRecording());
  expect(FakeSpeechXhr.instances.length).toBe(1);
  value.page.unmount(); expect(FakeSpeechXhr.instances[0].aborted).toBe(true);
  FakeSpeechXhr.instances[0].complete('must not be applied'); await act(async () => {});
  expect(value.transcript).not.toHaveBeenCalled();
});
test('a new recording replaces the old ASR epoch without applying its late result', async () => {
  const value = mount(); await act(async () => { await value.get().startRecording(); });
  act(() => value.get().stopRecording());
  const old = FakeSpeechXhr.instances[0];
  await act(async () => { await value.get().startRecording(); });
  expect(old.aborted).toBe(true);
  old.complete('obsolete words'); await act(async () => {});
  expect(value.transcript).not.toHaveBeenCalled(); expect(value.get().status).toBe('recording');
  act(() => value.get().stopRecording());
  await act(async () => { FakeSpeechXhr.instances[1].complete('new words'); });
  expect(value.transcript).toHaveBeenCalledWith('new words');
});
test('cancel during microphone permission acquisition closes a late device without recording', async () => {
  let resolve!: (stream: unknown) => void;
  const pending = new Promise<unknown>(done => { resolve = done; });
  Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia: () => pending } });
  const value = mount(); let start!: Promise<void>;
  act(() => { start = value.get().startRecording(); });
  act(() => value.get().cancel());
  const stop = mock();
  await act(async () => { resolve({ getTracks: () => [{ stop }] }); await start; });
  expect(stop).toHaveBeenCalled(); expect(Recorder.instances.length).toBe(0); expect(FakeSpeechXhr.instances.length).toBe(0);
  expect(value.get().status).toBe('idle');
});
