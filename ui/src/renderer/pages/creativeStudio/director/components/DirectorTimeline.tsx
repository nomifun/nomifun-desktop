/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Add,
  Camera,
  Close,
  Cube,
  Delete,
  Diamond,
  Film,
  LoopOnce,
  Pause,
  People,
  Play,
  SettingTwo,
} from '@icon-park/react';
import { Button, InputNumber, Tooltip } from '@arco-design/web-react';
import React from 'react';

import styles from './DirectorWorkbenchShell.module.css';
import type {
  DirectorTimelineTrack,
  DirectorWorkbenchShellProps,
} from './types';

type DirectorTimelineProps = Pick<
  DirectorWorkbenchShellProps,
  | 'timeline'
  | 'disabled'
  | 'onTimelineOpenChange'
  | 'onTimelinePlayingChange'
  | 'onTimelineLoopChange'
  | 'onTimelineAutoKeyChange'
  | 'onTimelineTimeChange'
  | 'onTimelineDurationChange'
  | 'onTimelineTrackSelect'
  | 'onKeyframeSelect'
  | 'onKeyframeAdd'
  | 'onKeyframeDelete'
  | 'onTimelineExport'
>;

const clampTime = (value: number, duration: number): number =>
  Math.max(0, Math.min(Math.max(duration, 0), Number.isFinite(value) ? value : 0));

const percentAt = (time: number, duration: number): string =>
  `${duration > 0 ? (clampTime(time, duration) / duration) * 100 : 0}%`;

const TrackIcon: React.FC<{ track: DirectorTimelineTrack }> = ({ track }) => {
  const props = { size: 14, strokeWidth: 1.8, fill: 'currentColor' };
  if (track.kind === 'camera') return <Camera {...props} />;
  if (track.kind === 'character') return <People {...props} />;
  if (track.kind === 'object') return <Cube {...props} />;
  return <SettingTwo {...props} />;
};

const DirectorTimeline: React.FC<DirectorTimelineProps> = ({
  timeline,
  disabled = false,
  onTimelineOpenChange,
  onTimelinePlayingChange,
  onTimelineLoopChange,
  onTimelineAutoKeyChange,
  onTimelineTimeChange,
  onTimelineDurationChange,
  onTimelineTrackSelect,
  onKeyframeSelect,
  onKeyframeAdd,
  onKeyframeDelete,
  onTimelineExport,
}) => {
  if (!timeline.open) return null;

  const selectedTrack = timeline.tracks.find(
    (track) => track.id === timeline.selectedTrackId || track.selected
  );
  const selectedKeyframe = selectedTrack?.keyframes.find(
    (keyframe) => keyframe.id === timeline.selectedKeyframeId || keyframe.selected
  );
  const rulerTicks = Array.from({ length: 6 }, (_, index) =>
    Number(((timeline.durationSeconds * index) / 5).toFixed(2))
  );

  return (
    <section
      className={styles.timeline}
      aria-label='时间轴'
      data-director-timeline
      data-timeline-playing={timeline.playing}
    >
      <header className={styles.timelineToolbar}>
        <div className={styles.timelinePlayback}>
          <Tooltip content={timeline.playing ? '暂停' : '播放'}>
            <Button
              type='text'
              shape='circle'
              size='small'
              aria-label={timeline.playing ? '暂停' : '播放'}
              icon={timeline.playing ? <Pause /> : <Play />}
              disabled={disabled}
              onClick={() => onTimelinePlayingChange(!timeline.playing)}
            />
          </Tooltip>
          <Tooltip content='循环播放'>
            <Button
              type='text'
              shape='circle'
              size='small'
              aria-label='循环播放'
              aria-pressed={timeline.loop}
              icon={<LoopOnce />}
              disabled={disabled}
              onClick={() => onTimelineLoopChange(!timeline.loop)}
            />
          </Tooltip>
          <Tooltip content='自动帧'>
            <Button
              type='text'
              shape='circle'
              size='small'
              aria-label='自动帧'
              aria-pressed={timeline.autoKey}
              icon={<Diamond />}
              disabled={disabled}
              onClick={() => onTimelineAutoKeyChange(!timeline.autoKey)}
            />
          </Tooltip>
        </div>

        <div className={styles.timelineTimeFields}>
          <InputNumber
            aria-label='当前时间'
            value={timeline.currentTimeSeconds}
            min={0}
            max={timeline.durationSeconds}
            step={1 / Math.max(1, timeline.fps)}
            precision={2}
            disabled={disabled}
            onChange={(next) =>
              onTimelineTimeChange(clampTime(next ?? 0, timeline.durationSeconds))
            }
          />
          <span>/</span>
          <InputNumber
            aria-label='时间轴时长'
            value={timeline.durationSeconds}
            min={1}
            step={1}
            precision={0}
            disabled={disabled}
            onChange={(next) => {
              if (typeof next === 'number' && next > 0) onTimelineDurationChange(next);
            }}
          />
          <small>{timeline.fps} FPS</small>
        </div>

        <div className={styles.timelineEditActions}>
          <Button
            type='text'
            size='small'
            icon={<Add />}
            disabled={disabled || !selectedTrack || !onKeyframeAdd}
            onClick={() =>
              selectedTrack && onKeyframeAdd?.(selectedTrack.id, timeline.currentTimeSeconds)
            }
          >
            添加关键帧
          </Button>
          <Button
            type='text'
            size='small'
            icon={<Delete />}
            disabled={disabled || !selectedTrack || !selectedKeyframe || !onKeyframeDelete}
            onClick={() =>
              selectedTrack &&
              selectedKeyframe &&
              onKeyframeDelete?.(selectedTrack.id, selectedKeyframe.id)
            }
          >
            删除
          </Button>
        </div>

        <Button
          className={styles.timelineExport}
          size='small'
          icon={<Film />}
          disabled={disabled || !onTimelineExport}
          onClick={onTimelineExport}
        >
          导出机位
        </Button>
        <Button
          type='text'
          shape='circle'
          size='small'
          aria-label='关闭时间轴'
          icon={<Close />}
          onClick={() => onTimelineOpenChange(false)}
        />
      </header>

      <div className={styles.timelineGrid}>
        <div className={styles.timelineListHeading}>场景轨道</div>
        <div className={styles.timelineRuler}>
          {rulerTicks.map((tick, index) => (
            <span
              key={`ruler-${index}-${tick}`}
              className={styles.timelineTick}
              style={{ left: percentAt(tick, timeline.durationSeconds) }}
            >
              {tick.toFixed(tick % 1 === 0 ? 0 : 1)}s
            </span>
          ))}
          <input
            className={styles.timelineScrubber}
            aria-label='时间轴播放头'
            type='range'
            min={0}
            max={timeline.durationSeconds}
            step={1 / Math.max(1, timeline.fps)}
            value={timeline.currentTimeSeconds}
            disabled={disabled}
            onChange={(event) => onTimelineTimeChange(Number(event.currentTarget.value))}
          />
        </div>

        <div className={styles.timelineTrackLabels}>
          {timeline.tracks.map((track) => (
            <button
              key={track.id}
              type='button'
              className={track.selected ? styles.timelineTrackLabelSelected : undefined}
              aria-pressed={track.selected || timeline.selectedTrackId === track.id}
              disabled={disabled}
              onClick={() => onTimelineTrackSelect(track.id)}
            >
              <TrackIcon track={track} />
              <span title={track.label}>{track.label}</span>
              <small>{track.keyframes.length}</small>
            </button>
          ))}
        </div>

        <div className={styles.timelineTracks}>
          {timeline.tracks.length === 0 ? (
            <div className={styles.timelineEmpty} role='status'>
              添加角色、模型或机位后即可制作动画
            </div>
          ) : (
            timeline.tracks.map((track) => (
              <div
                key={track.id}
                className={styles.timelineTrackRow}
                data-timeline-track-kind={track.kind}
              >
                {rulerTicks.map((tick, index) => (
                  <span
                    key={`guide-${index}-${tick}`}
                    className={styles.timelineGuide}
                    style={{ left: percentAt(tick, timeline.durationSeconds) }}
                  />
                ))}
                {track.keyframes.map((keyframe) => (
                  <Tooltip key={keyframe.id} content={`${keyframe.timeSeconds.toFixed(2)} 秒关键帧`}>
                    <button
                      type='button'
                      className={styles.keyframeMarker}
                      data-keyframe-selected={
                        keyframe.selected || timeline.selectedKeyframeId === keyframe.id
                      }
                      style={{ left: percentAt(keyframe.timeSeconds, timeline.durationSeconds) }}
                      aria-label={`${keyframe.timeSeconds.toFixed(2)} 秒关键帧`}
                      disabled={disabled}
                      onClick={() => onKeyframeSelect(track.id, keyframe.id)}
                    >
                      <Diamond aria-hidden='true' size={10} strokeWidth={2} />
                    </button>
                  </Tooltip>
                ))}
              </div>
            ))
          )}
          <span
            className={styles.timelinePlayhead}
            aria-hidden='true'
            style={{ left: percentAt(timeline.currentTimeSeconds, timeline.durationSeconds) }}
          />
        </div>
      </div>
    </section>
  );
};

export default DirectorTimeline;
