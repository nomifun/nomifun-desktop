import '../../../test/setup-dom.ts';
import { afterEach, beforeEach, expect, test } from 'bun:test';
import { FakeSpeechXhr } from '../../../test/fake-speech-xhr';
import { transcribeAudioBlob } from './SpeechToTextService';
const original = globalThis.XMLHttpRequest;
beforeEach(() => { FakeSpeechXhr.instances = []; globalThis.XMLHttpRequest = FakeSpeechXhr as unknown as typeof XMLHttpRequest; });
afterEach(() => { globalThis.XMLHttpRequest = original; });
test('a pre-aborted dictation does not open an upload', async () => {
  const abort = new AbortController(); abort.abort();
  await expect(transcribeAudioBlob(new Blob(['audio']), 'zh', abort.signal)).rejects.toThrow('STT_ABORTED');
  expect(FakeSpeechXhr.instances.length).toBe(0);
});
test('cancel aborts the owned XHR and removes listeners before a late success', async () => {
  const abort = new AbortController();
  const request = transcribeAudioBlob(new Blob(['audio']), 'zh', abort.signal);
  const xhr = FakeSpeechXhr.instances[0]; expect(xhr.sent).toBe(true);
  abort.abort(); await expect(request).rejects.toThrow('STT_ABORTED');
  expect(xhr.aborted).toBe(true); expect(xhr.listenerCount).toBe(0);
  xhr.complete('late transcript'); expect(xhr.listenerCount).toBe(0);
});
test('ordinary configured batch transcription still resolves and releases listeners', async () => {
  const request = transcribeAudioBlob(new Blob(['audio']), 'zh');
  const xhr = FakeSpeechXhr.instances[0]; xhr.complete('ordinary transcript');
  expect((await request).text).toBe('ordinary transcript'); expect(xhr.listenerCount).toBe(0);
});
