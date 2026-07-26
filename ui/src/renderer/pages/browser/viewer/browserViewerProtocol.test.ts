import { describe, expect, test } from 'bun:test';
import {
  browserViewerErrorKind,
  browserViewerStateAfterMetadata,
  buildBrowserViewerUrl,
  isBrowserViewerStreamFailure,
  LatestBrowserFrame,
  mapBrowserViewerPoint,
  parseBrowserViewerMetadata,
} from './browserViewerProtocol';

const expectToThrow = (callback: () => unknown): void => {
  let threw = false;
  try {
    callback();
  } catch {
    threw = true;
  }
  expect(threw).toBe(true);
};

describe('browser viewer protocol', () => {
  test('keeps exactly the newest frame', () => {
    const frames = new LatestBrowserFrame<number>();
    expect(frames.push(1)).toBeNull();
    expect(frames.push(2)).toBe(1);
    expect(frames.peek()).toBe(2);
    expect(frames.take()).toBe(2);
    expect(frames.peek()).toBeNull();
  });

  test('maps object-contain input and rejects its letterbox', () => {
    const rect = { left: 0, top: 0, width: 200, height: 200 };
    expect(mapBrowserViewerPoint(rect, { width: 200, height: 100 }, 100, 100)).toEqual({
      x: 100,
      y: 50,
    });
    expect(mapBrowserViewerPoint(rect, { width: 200, height: 100 }, 100, 20)).toBeNull();
  });

  test('accepts wrapped metadata and uses the freshly minted viewer token', () => {
    expect(
      parseBrowserViewerMetadata(
        JSON.stringify({
          event: 'viewer.metadata',
          data: { frame: { width: 800, height: 600 }, control_state: 'user' },
        })
      )
    ).toMatchObject({
      type: 'viewer.metadata',
      frame: { width: 800, height: 600 },
      control_state: 'user',
    });
    expect(buildBrowserViewerUrl('lane/1', 'a b')).toBe(
      '/api/browser/lanes/lane%2F1/view?token=a+b'
    );
    expect(
      buildBrowserViewerUrl(
        'lane',
        'new',
        '/api/browser/lanes/lane/view?token=stale&keep=yes'
      )
    ).toBe('/api/browser/lanes/lane/view?token=new&keep=yes');
  });

  test('distinguishes stream failures from command and protocol errors', () => {
    expect(
      isBrowserViewerStreamFailure({
        type: 'stream_error',
        code: 'viewer_stream_failed',
      })
    ).toBe(true);
    expect(
      isBrowserViewerStreamFailure({
        type: 'command_error',
        code: 'operation_not_allowed',
      })
    ).toBe(false);
    expect(
      isBrowserViewerStreamFailure({
        type: 'protocol_error',
        code: 'invalid_viewer_message',
      })
    ).toBe(false);
    expect(
      browserViewerErrorKind({
        type: 'command_error',
        code: 'viewer_stream_failed',
      })
    ).toBe('command');
    expect(
      browserViewerErrorKind({
        type: 'protocol_error',
        code: 'viewer_stream_failed',
      })
    ).toBe('protocol');
    expect(
      browserViewerErrorKind({
        type: 'error',
        code: 'viewer_stream_failed',
      })
    ).toBe('stream');
    expect(
      browserViewerStateAfterMetadata('streaming', {
        type: 'command_error',
        code: 'operation_not_allowed',
      })
    ).toBe('streaming');
    expect(
      browserViewerStateAfterMetadata('streaming', {
        type: 'protocol_error',
        code: 'invalid_viewer_message',
      })
    ).toBe('streaming');
    expect(
      browserViewerStateAfterMetadata('streaming', {
        type: 'stream_error',
        code: 'viewer_stream_failed',
      })
    ).toBe('failed');
  });

  test('rejects cross-lane viewer paths, credentials, and fragments', () => {
    expectToThrow(() =>
      buildBrowserViewerUrl('lane-a', 'token', '/api/browser/lanes/lane-b/view')
    );
    expectToThrow(() =>
      buildBrowserViewerUrl(
        'lane-a',
        'token',
        'https://user:password@example.test/api/browser/lanes/lane-a/view'
      )
    );
    expectToThrow(() =>
      buildBrowserViewerUrl('lane-a', 'token', '/api/browser/lanes/lane-a/view#token=stale')
    );
  });
});
