/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { expect, test } from 'bun:test';
import { ProseCursorStore } from './cursor';

function harness() {
  let now = 0;
  let frameId = 0;
  const frames = new Map<number, FrameRequestCallback>();
  const store = new ProseCursorStore({
    now: () => now,
    request: callback => { frames.set(++frameId, callback); return frameId; },
    cancel: id => { frames.delete(id); },
  });
  return { store, frames, elapse: (elapsed: number) => { now += elapsed; }, advance: (elapsed: number) => {
    now += elapsed;
    const pending = [...frames.values()];
    frames.clear();
    for (const frame of pending) frame(now);
  } };
}

test('an atomic live report is gradually revealed and catches up within a bounded interval', () => {
  const { store, frames, advance } = harness();
  const text = '这是一份已经校验完成的报告。'.repeat(100);
  store.update('report', text, true);
  expect(store.visible('report', text, true)).toBe(0);
  advance(16);
  expect(store.visible('report', text, true)).toBeGreaterThan(0);
  expect(store.visible('report', text, true)).toBeLessThan(text.length);
  advance(584);
  expect(store.visible('report', text, true)).toBe(text.length);
  expect(frames.size).toBe(0);
});

test('short appends after a provider gap start from the shown prefix without catching up stale elapsed time', () => {
  const { store, advance } = harness();
  store.update('reply', '甲', true);
  advance(32);
  expect(store.visible('reply', '甲', true)).toBe(1);
  advance(3000);
  store.update('reply', '甲乙丙丁戊己庚', true);
  expect(store.visible('reply', '甲乙丙丁戊己庚', true)).toBe(1);
  advance(16);
  expect(store.visible('reply', '甲乙丙丁戊己庚', true)).toBeLessThan(7);
  advance(584);
  expect(store.visible('reply', '甲乙丙丁戊己庚', true)).toBe(7);
});

test('a large delta near the old deadline gets smooth frames and finishes within its own catch-up window', () => {
  const { store, frames, elapse, advance } = harness();
  const first = '甲'.repeat(1000);
  store.update('reply', first, true);
  advance(580);
  const before = store.visible('reply', first, true);
  expect(before).toBeLessThan(first.length);
  elapse(10);
  const appended = first + '乙'.repeat(10000);
  store.update('reply', appended, true);
  advance(16); // Past the first burst's deadline, still early in the new delta.
  const shown = store.visible('reply', appended, true);
  expect(shown).toBeGreaterThan(before);
  expect(shown).toBeLessThan(appended.length / 2);
  expect(frames.size).toBe(1);
  advance(584);
  expect(store.visible('reply', appended, true)).toBe(appended.length);
  expect(frames.size).toBe(0);
});

test('adaptive pacing bounds sustained-stream backlog and drains within 600ms after the last delta', () => {
  const { store, frames, advance } = harness();
  let text = '';
  let previousVisible = 0;
  for (let round = 0; round < 100; round++) {
    text += '持续输出'.repeat(25); // 100 graphemes/100ms: faster than baseline.
    store.update('reply', text, true);
    advance(100);
    const shown = store.visible('reply', text, true);
    expect(shown).toBeGreaterThan(previousVisible);
    expect(text.length - shown).toBeLessThanOrEqual(600);
    previousVisible = shown;
  }
  advance(500);
  expect(store.visible('reply', text, true)).toBe(text.length);
  expect(frames.size).toBe(0);
});

test('CJK, combining marks, flags and joined emoji are never revealed at partial grapheme boundaries', () => {
  const { store, advance } = harness();
  const text = '中文e\u0301👩🏽‍💻🇨🇳👨‍👩‍👧‍👦'.repeat(5);
  const ends = new Set([0, ...Array.from(new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(text),
    part => part.index + part.segment.length)]);
  store.update('reply', text, true);
  for (let round = 0; round < 40; round++) {
    advance(16);
    expect(ends.has(store.visible('reply', text, true))).toBe(true);
  }
  expect(store.visible('reply', text, true)).toBe(text.length);
});

test('a delta extending the last shown grapheme never exposes a trailing joiner', () => {
  const { store, advance } = harness();
  store.update('reply', '👩', true);
  advance(600);
  store.update('reply', '👩‍💻后续', true);
  expect(store.visible('reply', '👩‍💻后续', true)).toBe('👩‍💻'.length);
  advance(600);
  expect(store.visible('reply', '👩‍💻后续', true)).toBe('👩‍💻后续'.length);
});

test('history appears immediately and only a subsequent live suffix is animated', () => {
  const { store, advance } = harness();
  const history = '已经加载的历史正文。';
  store.update('reply', history, false);
  expect(store.visible('reply', history, false)).toBe(history.length);
  const live = history + '新收到的实时正文。';
  expect(store.visible('reply', live, true)).toBe(history.length);
  store.update('reply', live, true);
  expect(store.visible('reply', live, true)).toBe(history.length);
  advance(600);
  expect(store.visible('reply', live, true)).toBe(live.length);
});

test('replacements and shortening synchronously invalidate old progress and cancel pending animation', () => {
  const { store, frames, advance } = harness();
  store.update('reply', '旧的流式内容'.repeat(20), true);
  advance(16);
  expect(store.visible('reply', '已更正', true)).toBe(3);
  store.update('reply', '已更正', true);
  expect(store.visible('reply', '已更正', true)).toBe(3);
  expect(frames.size).toBe(0);
  store.update('reply', '已', true);
  expect(store.visible('reply', '已', true)).toBe(1);
  expect(frames.size).toBe(0);
});

test('revocation flushes pending prose and removes live eligibility without changing its authoritative source', () => {
  const { store, frames } = harness();
  const text = '已完成的文字';
  store.markLive('reply');
  store.update('reply', text, true);
  expect(store.isLive('reply')).toBe(true);
  store.revoke('reply');
  expect(store.isLive('reply')).toBe(false);
  expect(store.visible('reply', text, true)).toBe(text.length);
  expect(frames.size).toBe(0);
});

test('normal completion keeps admitted text draining while authoritative cancellation flushes and cannot resurrect', () => {
  const { store, frames, advance } = harness();
  const text = '已经收到但尚未完全呈现的正文'.repeat(10);
  store.update('reply', text, true);
  advance(16);
  const shown = store.visible('reply', text, true);
  store.update('reply', text, false);
  expect(store.visible('reply', text, false)).toBe(shown);
  expect(frames.size).toBe(1);
  store.update('reply', text, false, true);
  expect(store.visible('reply', text, false, true)).toBe(text.length);
  expect(frames.size).toBe(0);
  advance(16);
  expect(store.visible('reply', text, true)).toBe(text.length);
  expect(frames.size).toBe(0);
});

test('a process/public remount with the same exact source identity keeps its cursor', () => {
  const { store, advance } = harness();
  const text = '相同 canonical 正文'.repeat(10);
  const unsubscribe = store.subscribe('turn/source', () => {});
  store.update('turn/source', text, true);
  advance(80);
  const before = store.visible('turn/source', text, true);
  unsubscribe();
  const nextUnsubscribe = store.subscribe('turn/source', () => {});
  store.update('turn/source', text, true);
  expect(store.visible('turn/source', text, true)).toBe(before);
  advance(520);
  expect(store.visible('turn/source', text, true)).toBe(text.length);
  nextUnsubscribe();
});

test('pending process/public handoff survives cache pressure from a long mounted transcript', () => {
  const { store, advance } = harness();
  const cleanups: Array<() => void> = [];
  for (let index = 0; index < 160; index++) {
    const identity = `history-${index}`;
    cleanups.push(store.subscribe(identity, () => {}));
    store.update(identity, '已经显示的历史', false);
  }
  const text = '当前流式正文'.repeat(20);
  const unsubscribe = store.subscribe('current', () => {});
  store.update('current', text, true);
  advance(32);
  const before = store.visible('current', text, true);
  unsubscribe();
  store.subscribe('current', () => {});
  expect(store.visible('current', text, true)).toBe(before);
  cleanups.forEach(cleanup => cleanup());
});

test('reduced motion and background visibility flush pending text and do not replay it on return', () => {
  const { store, frames } = harness();
  const text = '不需要动画的正文';
  store.update('reply', text, true);
  store.setImmediate(true);
  expect(store.visible('reply', text, true)).toBe(text.length);
  expect(frames.size).toBe(0);
  store.setImmediate(false);
  expect(store.visible('reply', text, true)).toBe(text.length);
});

test('two canonical source identities progress independently and disposal cancels all pending work', () => {
  const { store, frames, advance } = harness();
  store.update('turn-a/source', '甲'.repeat(100), true);
  store.update('turn-b/source', '乙'.repeat(30), true);
  advance(32);
  expect(store.visible('turn-a/source', '甲'.repeat(100), true)).toBeGreaterThan(0);
  expect(store.visible('turn-b/source', '乙'.repeat(30), true)).toBeGreaterThan(0);
  store.dispose();
  expect(frames.size).toBe(0);
  expect(store.isLive('turn-a/source')).toBe(false);
});

test('retained inactive cursors and live provenance are bounded', () => {
  const { store } = harness();
  for (let index = 0; index < 400; index++) {
    store.markLive(String(index));
    store.update(String(index), 'history', false);
  }
  expect(store.isLive('0')).toBe(false);
  expect(store.isLive('399')).toBe(true);
  expect(store.revision('0')).toBe(0);
  expect(store.revision('399')).toBeGreaterThan(0);
});

test('hidden live prose advances independently of mounted views and never replays when reopened later', () => {
  const { store, frames, advance } = harness();
  const history = '此前已经展示的过程说明。';
  store.commitSources(new Map([['source', history]]), undefined);
  expect(store.revision('source')).toBe(0);
  store.markLive('source');
  const live = history + '收起分段后仍收到的实时增量。'.repeat(20);
  store.commitSources(new Map([['source', live]]), store.generation());
  expect(store.isLive('source')).toBe(false);
  expect(store.visible('source', live, false)).toBe(history.length);
  advance(600);
  advance(60000);
  expect(store.visible('source', live, false)).toBe(live.length);
  store.update('source', live, false);
  expect(frames.size).toBe(0);
});

test('history refresh after an earlier live delivery never treats the old source identity as new ingress', () => {
  const { store, frames, advance } = harness();
  const first = '真实收到的第一段。';
  store.markLive('source');
  store.commitSources(new Map([['source', first]]), store.generation());
  advance(600);
  const history = first + '从历史读取补齐的正文。';
  store.commitSources(new Map([['source', history]]), undefined);
  expect(store.isLive('source')).toBe(false);
  expect(store.visible('source', history, false)).toBe(history.length);
  expect(frames.size).toBe(0);
});

test('a tagged older commit consumes only its own ingress and cannot overwrite a newer committed source', () => {
  const { store, advance } = harness();
  const initial = '已经显示。';
  store.commitSources(new Map([['source', initial]]), undefined);
  store.markLive('source');
  const olderGeneration = store.generation();
  store.markLive('source');
  const newerGeneration = store.generation();
  const older = initial + '第一批正文'.repeat(20);
  const newer = older + '第二批正文'.repeat(20);
  store.commitSources(new Map([['source', older]]), olderGeneration);
  expect(store.isLive('source', olderGeneration)).toBe(false);
  expect(store.isLive('source', newerGeneration)).toBe(true);
  store.commitSources(new Map([['source', newer]]), newerGeneration);
  expect(store.isLive('source')).toBe(false);
  store.commitSources(new Map([['source', older]]), olderGeneration);
  advance(600);
  expect(store.visible('source', newer, false)).toBe(newer.length);
});

test('history and unrelated render snapshots never consume a queued live marker', () => {
  const { store, frames } = harness();
  const history = '此前加载的历史正文。';
  const oldSnapshot: unknown[] = [];
  store.noteSnapshot(oldSnapshot, 0);
  store.commitSources(new Map([['source', history]]), undefined);
  store.markLive('source');
  const pendingGeneration = store.generation();
  store.commitSources(new Map([['source', history]]), store.generationFor(oldSnapshot));
  expect(store.isLive('source', pendingGeneration)).toBe(true);
  store.commitSources(new Map([['source', history]]), pendingGeneration);
  expect(store.isLive('source', pendingGeneration)).toBe(true);
  expect(store.isLive('source', undefined)).toBe(false);
  expect(store.visible('source', history, true)).toBe(history.length);
  expect(frames.size).toBe(0);
  const liveSnapshot: unknown[] = [];
  store.noteSnapshot(liveSnapshot, pendingGeneration);
  const live = history + '真正合入列表的增量。';
  store.commitSources(new Map([['source', live]]), store.generationFor(liveSnapshot));
  expect(store.isLive('source')).toBe(false);
  expect(store.visible('source', live, false)).toBe(history.length);
  expect(frames.size).toBe(1);
});

test('an initial history snapshot cannot consume live ingress queued before its first mount', () => {
  const { store, frames } = harness();
  const history = '初次读取的历史正文。';
  store.markLive('source');
  store.commitSources(new Map([['source', history]]), undefined);
  expect(store.isLive('source')).toBe(true);
  expect(store.visible('source', history, false)).toBe(history.length);
  expect(store.revision('source')).toBe(0);
  const live = history + '后续实时增量。';
  store.commitSources(new Map([['source', live]]), store.generation());
  expect(store.visible('source', live, false)).toBe(history.length);
  expect(store.isLive('source')).toBe(false);
  expect(frames.size).toBe(1);
});

test('removing a source from the committed list cancels hidden presentation work', () => {
  const { store, frames } = harness();
  store.markLive('source');
  store.commitSources(new Map([['source', '正在推进的已接收正文'.repeat(20)]]), store.generation());
  expect(frames.size).toBe(1);
  store.commitSources(new Map(), store.generation());
  expect(frames.size).toBe(0);
  expect(store.revision('source')).toBe(0);
});

test('several append revisions stage one immutable source reference until the shared presentation frame', () => {
  const { store, frames, advance } = harness();
  const history = '已经可见。';
  store.update('source', history, false);
  store.update('source', history + '甲', true);
  store.update('source', history + '甲乙', true);
  const latest = history + '甲乙丙';
  store.update('source', latest, true);
  expect(store.frameSource('source', latest, true)).toBe(history);
  expect(frames.size).toBe(1);
  let notifications = 0;
  store.subscribe('source', () => { notifications++; });
  advance(16); // Less than one grapheme's credit; parsing still gets the latest source.
  expect(store.frameSource('source', latest, true)).toBe(latest);
  expect(store.visible('source', latest, true)).toBe(history.length);
  expect(notifications).toBe(1);
});

test('source correction and forced immediacy synchronously flush the staged reference as well as the cursor', () => {
  const { store, frames } = harness();
  store.update('source', '已显示', false);
  store.update('source', '已显示后续增量', true);
  expect(store.frameSource('source', '已显示后续增量', true)).toBe('已显示');
  store.update('source', '更正后的正文', true);
  expect(store.frameSource('source', '更正后的正文', true)).toBe('更正后的正文');
  expect(frames.size).toBe(0);
  store.update('source', '更正后的正文还有增量', true);
  store.update('source', '更正后的正文还有增量', true, true);
  expect(store.frameSource('source', '更正后的正文还有增量', true)).toBe('更正后的正文还有增量');
  expect(frames.size).toBe(0);
});

test('ordinary completion handoff preserves the staged source until the existing next frame', () => {
  const { store, frames, advance } = harness();
  store.update('source', '此前可见', false);
  const latest = '此前可见最后到达的增量';
  store.update('source', latest, true);
  store.update('source', latest, false);
  expect(store.frameSource('source', latest, false)).toBe('此前可见');
  expect(frames.size).toBe(1);
  advance(16);
  expect(store.frameSource('source', latest, false)).toBe(latest);
});

test('extending an already-shown emoji schedules a source frame even when the aligned cursor is complete', () => {
  const { store, frames, advance } = harness();
  store.update('source', '👩', false);
  store.update('source', '👩‍💻', true);
  expect(store.visible('source', '👩‍💻', true)).toBe('👩‍💻'.length);
  expect(store.frameSource('source', '👩‍💻', true)).toBe('👩');
  expect(frames.size).toBe(1);
  advance(16);
  expect(store.frameSource('source', '👩‍💻', true)).toBe('👩‍💻');
  expect(frames.size).toBe(0);
});
