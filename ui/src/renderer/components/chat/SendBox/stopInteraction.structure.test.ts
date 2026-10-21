import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const composerSource = readFileSync(new URL('../Composer.tsx', import.meta.url), 'utf8');
const platformSendBoxes = [
  '../../../pages/conversation/platforms/nomi/NomiSendBox.tsx',
].map((path) => ({ path, source: readFileSync(new URL(path, import.meta.url), 'utf8') }));

describe('SendBox stop interaction', () => {
  test('names the icon-only stop and send controls for keyboard and screen-reader users', () => {
    expect(source.includes("aria-label={t(isStopping ? 'conversation.stop.stopping' : 'conversation.stop.button')}")).toBe(true);
    expect(source.includes("title={t(isStopping ? 'conversation.stop.stopping' : 'conversation.stop.button')}")).toBe(true);
    expect(composerSource.includes("aria-label={title ?? t('common.send')}")).toBe(true);
    expect(source.includes('<ComposerSendButton')).toBe(true);
    expect(source.includes('steerMessageHandler')).toBe(false);
  });

  test('deduplicates stop clicks and blocks send/steer until stop settles', () => {
    expect(source.includes('const [isStopping, setIsStopping] = useState(false)')).toBe(true);
    expect(source.includes('if (!onStop || isStoppingRef.current) return;')).toBe(true);
    expect(source.includes('isStoppingRef.current = true;')).toBe(true);
    expect(source.includes('isStoppingRef.current = false;')).toBe(true);
    expect(source.includes('setIsStopping(true);')).toBe(true);
    expect(source.includes('setIsStopping(false);')).toBe(true);
    expect(source.includes('if (isUploading || isStopping) return;')).toBe(true);
    expect(source.includes("data-testid='sendbox-stop-btn'")).toBe(true);
    expect(source.includes('disabled={isStopping}')).toBe(true);
    expect(source.includes('loading={isStopping}')).toBe(true);
  });

  test('remounts shared stop state when switching conversations', () => {
    for (const platform of platformSendBoxes) {
      expect(/<SendBox\r?\n\s+key=\{conversation_id\}/.test(platform.source)).toBe(true);
    }
  });

  test('checks authoritative start and completion generations before applying a stop result', () => {
    for (const platform of platformSendBoxes) {
      const stopResultIndex = platform.source.indexOf(
        'const result = await stopConversationAndConfirmRelease(conversation_id'
      );
      const statusIndex = platform.source.indexOf(
        'const stopAttemptStatus = getStopAttemptStatus(stopAttempt);',
        stopResultIndex
      );
      const confirmIndex = platform.source.indexOf('confirmStopped();', statusIndex);

      expect(stopResultIndex >= 0).toBe(true);
      expect(statusIndex > stopResultIndex).toBe(true);
      expect(confirmIndex > statusIndex).toBe(true);
      expect(platform.source.includes('getTurnCompletionGeneration')).toBe(true);
      expect(platform.source.includes('shouldReleaseStopInteraction(stopAttemptStatus)')).toBe(true);
    }
  });

  test('Stop delegates durable queue ownership to the Host', () => {
    for (const platform of platformSendBoxes) {
      expect(platform.source.includes('resetActiveExecution')).toBe(false);
      expect(platform.source.includes('useConversationCommandQueue')).toBe(false);
      expect(platform.source.includes('stopConversationAndConfirmRelease(conversation_id')).toBe(true);
    }
  });
});
