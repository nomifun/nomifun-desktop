import { describe, expect, test } from 'bun:test';
import { NOMIFUN_FILES_MARKER } from '@/common/config/constants';
import { buildDisplayMessage } from './messageFiles';

const paths = (files: string[], workspace: string) =>
  buildDisplayMessage('Review these files', files, workspace).split(`${NOMIFUN_FILES_MARKER}\n`)[1]?.split('\n');

describe('new message attachment locations', () => {
  test('stores nested managed Windows files relative to their exact workspace', () => {
    expect(paths(['C:\\old\\workspace\\uploads\\photo.png', 'C:/old/workspace/docs/report.pdf'], 'C:\\old\\workspace\\'))
      .toEqual(['uploads/photo.png', 'docs/report.pdf']);
  });

  test('keeps relative files relative and retains existing upload timestamp display cleanup', () => {
    expect(paths(['uploads/photo_nomifun_1234567890123.png'], '/managed/session'))
      .toEqual(['uploads/photo.png']);
    expect(paths(['/managed/session/uploads/photo.png'], '/managed/session')).toEqual(['uploads/photo.png']);
  });

  test('preserves external absolute files and sibling directories sharing the workspace prefix', () => {
    expect(paths(['D:/personal/image.png', 'C:/managed/session-other/image.png', 'C:/managed/session'], 'C:/managed/session'))
      .toEqual(['D:/personal/image.png', 'C:/managed/session-other/image.png', 'C:/managed/session']);
  });

  test('does not classify paths containing parent traversal as managed workspace files', () => {
    expect(paths(['C:/managed/session/../external/image.png', '../external/image.png'], 'C:/managed/session'))
      .toEqual(['C:/managed/session/../external/image.png', 'C:/managed/session/../external/image.png']);
  });

  test('preserves external UNC paths and makes files inside the exact UNC workspace relative', () => {
    expect(paths(['\\\\server\\share\\session\\photo.png', '\\\\other\\share\\photo.png'], '\\\\server\\share\\session'))
      .toEqual(['photo.png', '\\\\other\\share\\photo.png']);
  });

  test('keeps attachment paths without a managed workspace and leaves text without files unchanged', () => {
    expect(paths(['D:/personal/photo.png'], '')).toEqual(['D:/personal/photo.png']);
    expect(buildDisplayMessage('Original text', [], 'C:/managed/session')).toBe('Original text');
  });
});
