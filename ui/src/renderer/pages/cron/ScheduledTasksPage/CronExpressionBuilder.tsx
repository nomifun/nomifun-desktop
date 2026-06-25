/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';
import { Input, Tag } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { Cron } from 'croner';
import { getCurrentCronTimeZone } from '@renderer/pages/cron/cronUtils';

/**
 * Visual + raw editor for a 6-field, seconds-first cron expression
 * (`秒 分 时 日 月 周`), matching the Quartz-style dialect the Nomicore backend
 * parses with the `cron` crate.
 *
 * The raw expression is the single source of truth (`value`); the six
 * per-field inputs are derived from it and rewrite it on edit. Validation and
 * the "next runs" preview are computed client-side with `croner` purely as a
 * UX aid — the authoritative schedule is validated server-side on save.
 *
 * Two dialect rules keep the UI in lockstep with the backend:
 *  - The seconds field is shown but defaults to `0`; a non-zero seconds field
 *    means a sub-minute task and is flagged with a warning (high OS overhead).
 *  - Quartz extras `L` / `W` / `#` are rejected here because the backend
 *    `cron` crate does not support them (croner does — hence the explicit guard).
 */

const FIELD_KEYS = ['second', 'minute', 'hour', 'dayOfMonth', 'month', 'dayOfWeek'] as const;

const FIELD_LABEL_DEFAULTS: Record<(typeof FIELD_KEYS)[number], string> = {
  second: '秒',
  minute: '分钟',
  hour: '小时',
  dayOfMonth: '日',
  month: '月',
  dayOfWeek: '星期',
};

const FIELD_PLACEHOLDERS: Record<(typeof FIELD_KEYS)[number], string> = {
  second: '0-59',
  minute: '0-59',
  hour: '0-23',
  dayOfMonth: '1-31',
  month: '1-12',
  dayOfWeek: '0-6',
};

const PRESETS: { key: string; expr: string }[] = [
  { key: 'everyMin', expr: '0 * * * * ?' },
  { key: 'every5min', expr: '0 */5 * * * ?' },
  { key: 'every30min', expr: '0 */30 * * * ?' },
  { key: 'hourly', expr: '0 0 * * * ?' },
  { key: 'everyDay9', expr: '0 0 9 * * ?' },
  { key: 'everyMon9', expr: '0 0 9 ? * MON' },
  { key: 'firstOfMonth', expr: '0 0 9 1 * ?' },
];

const PREVIEW_COUNT = 3;

/**
 * Normalize an expression to exactly six fields. A 5-field (legacy Unix) form
 * is promoted by prepending the seconds field `0`; shorter forms are padded
 * with `*` (and `0` for seconds).
 */
export function splitExpr(expr: string): string[] {
  let parts = (expr || '').trim().split(/\s+/).filter(Boolean);
  if (parts.length === 5) parts = ['0', ...parts];
  while (parts.length < 6) parts.push(parts.length === 0 ? '0' : '*');
  return parts.slice(0, 6);
}

/** True when the expression fires more than once per minute (seconds ≠ `0`). */
export function isSubMinute(expr: string): boolean {
  const [seconds] = splitExpr(expr);
  return seconds.trim() !== '0';
}

/** Reject Quartz extras the backend `cron` crate cannot parse (`L`/`W`/`#`). */
function hasUnsupportedQuartzTokens(expr: string): boolean {
  const [, , , dayOfMonth = '', , dayOfWeek = ''] = splitExpr(expr);
  // `WED` legitimately contains `W`, so only the day-of-month field is checked
  // for `L`/`W`, and only day-of-week for `#`/`L`.
  return /[LW]/i.test(dayOfMonth) || /[#]/.test(dayOfWeek) || /\dL\b|^L$/i.test(dayOfWeek);
}

export type CronValidation = { valid: boolean; error?: string; nextRuns: Date[]; subMinute: boolean };

/** Validate a 5/6-field cron expression and compute upcoming runs in `tz`. */
export function validateCronExpression(expr: string, tz?: string, count = PREVIEW_COUNT): CronValidation {
  const trimmed = (expr || '').trim();
  const subMinute = isSubMinute(trimmed);
  if (!trimmed) return { valid: false, nextRuns: [], subMinute: false };
  if (hasUnsupportedQuartzTokens(trimmed)) {
    return { valid: false, error: 'unsupported_token', nextRuns: [], subMinute };
  }
  try {
    const timezone = tz || getCurrentCronTimeZone();
    const cron = new Cron(splitExpr(trimmed).join(' '), { timezone });
    const nextRuns: Date[] = [];
    let cursor: Date | null = new Date();
    for (let i = 0; i < count; i++) {
      const next: Date | null = cron.nextRun(cursor);
      if (!next) break;
      nextRuns.push(next);
      cursor = next;
    }
    if (nextRuns.length === 0) {
      return { valid: false, error: 'no_upcoming_run', nextRuns: [], subMinute };
    }
    return { valid: true, nextRuns, subMinute };
  } catch (error) {
    return { valid: false, error: error instanceof Error ? error.message : String(error), nextRuns: [], subMinute };
  }
}

export interface CronExpressionBuilderProps {
  value: string;
  onChange: (expr: string) => void;
  /** IANA timezone used for the next-run preview (defaults to the local zone). */
  tz?: string;
}

const CronExpressionBuilder: React.FC<CronExpressionBuilderProps> = ({ value, onChange, tz }) => {
  const { t } = useTranslation();
  const fields = useMemo(() => splitExpr(value), [value]);
  const validation = useMemo(() => validateCronExpression(value, tz), [value, tz]);

  const setField = (index: number, raw: string) => {
    const next = [...fields];
    next[index] = raw.trim() === '' ? (index === 0 ? '0' : '*') : raw.trim();
    onChange(next.join(' '));
  };

  return (
    <div className='flex flex-col gap-10px'>
      {/* Raw expression */}
      <Input
        value={value}
        onChange={onChange}
        placeholder='0 */5 * * * ?'
        status={value.trim() && !validation.valid ? 'error' : undefined}
        className='font-mono'
      />

      {/* Per-field visual editor */}
      <div className='grid grid-cols-6 gap-6px'>
        {FIELD_KEYS.map((key, index) => (
          <div key={key} className='min-w-0'>
            <label className='mb-4px block text-11px text-t-tertiary'>
              {t(`cron.page.cronExpression.${key}`, { defaultValue: FIELD_LABEL_DEFAULTS[key] })}
            </label>
            <Input
              size='small'
              value={fields[index]}
              onChange={(v) => setField(index, v)}
              placeholder={FIELD_PLACEHOLDERS[key]}
              className='text-center font-mono'
            />
          </div>
        ))}
      </div>

      {/* Presets */}
      <div className='flex flex-wrap items-center gap-6px'>
        <span className='text-11px text-t-tertiary'>{t('cron.page.cronExpression.presets')}</span>
        {PRESETS.map((preset) => (
          <Tag
            key={preset.key}
            checkable
            checked={splitExpr(value).join(' ') === preset.expr}
            onCheck={() => onChange(preset.expr)}
            className='cursor-pointer'
          >
            {t(`cron.page.cronExpression.preset.${preset.key}`, {
              defaultValue: preset.key === 'everyMin' ? '每分钟' : preset.key,
            })}
          </Tag>
        ))}
      </div>

      {/* Sub-minute warning — high OS overhead, discouraged. */}
      {validation.subMinute && (
        <div className='text-12px text-warning'>
          ⚠{' '}
          {t('cron.page.cronExpression.subMinuteWarning', {
            defaultValue: '秒级任务会高频触发，对系统开销极大，不建议配置。',
          })}
        </div>
      )}

      {/* Validation + next-run preview */}
      {value.trim() &&
        (validation.valid ? (
          <div className='text-12px text-t-secondary'>
            <span className='text-success'>{t('cron.page.cronExpression.nextRuns')}</span>
            <span className='ml-6px'>
              {validation.nextRuns.map((date) => date.toLocaleString()).join(' · ') || '-'}
            </span>
          </div>
        ) : (
          <div className='text-12px text-error'>
            {validation.error === 'unsupported_token'
              ? t('cron.page.cronExpression.unsupportedToken', {
                  defaultValue: '不支持 L / W / # 等高级符号，请使用 * , - / ? 与星期名。',
                })
              : t('cron.page.cronExpression.invalid')}
          </div>
        ))}
    </div>
  );
};

export default CronExpressionBuilder;
