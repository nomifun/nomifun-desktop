import { Select } from '@arco-design/web-react';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

const DEFAULT_CONTEXT_LIMIT_VALUE = 'default';

export const CONTEXT_WINDOW_OPTIONS = [
  {
    value: DEFAULT_CONTEXT_LIMIT_VALUE,
    labelKey: 'settings.contextLimitDefaultOption',
    defaultLabel: '默认 200k',
  },
  { value: 32_000, defaultLabel: '32k' },
  { value: 64_000, defaultLabel: '64k' },
  { value: 128_000, defaultLabel: '128k' },
  { value: 200_000, defaultLabel: '200k' },
  { value: 1_000_000, defaultLabel: '1M' },
] as const;

export const formatContextLimit = (tokens: number): string => {
  if (tokens >= 1_000_000 && tokens % 1_000_000 === 0) return `${tokens / 1_000_000}M`;
  if (tokens >= 1_000 && tokens % 1_000 === 0) return `${tokens / 1_000}k`;
  return new Intl.NumberFormat().format(tokens);
};

const isPresetContextLimit = (value: number): boolean =>
  CONTEXT_WINDOW_OPTIONS.some((option) => option.value === value);

const normalizeContextLimit = (value: unknown): number | undefined => {
  if (typeof value === 'number' && Number.isFinite(value) && value > 0) return value;
  return undefined;
};

const toSelectValue = (value: number | undefined): number | typeof DEFAULT_CONTEXT_LIMIT_VALUE =>
  normalizeContextLimit(value) ?? DEFAULT_CONTEXT_LIMIT_VALUE;

interface ContextLimitSelectProps {
  value?: number;
  onChange?: (value?: number) => void;
}

export const ContextLimitSelect: React.FC<ContextLimitSelectProps> = ({ value, onChange }) => {
  const { t } = useTranslation();
  const normalizedValue = normalizeContextLimit(value);

  const options = useMemo(() => {
    const presetOptions: Array<{ value: string | number; label: React.ReactNode }> = CONTEXT_WINDOW_OPTIONS.map(
      (option) => ({
        value: option.value,
        label:
          'labelKey' in option
            ? t(option.labelKey, { defaultValue: option.defaultLabel })
            : option.defaultLabel,
      })
    );

    if (normalizedValue && !isPresetContextLimit(normalizedValue)) {
      const formatted = formatContextLimit(normalizedValue);
      presetOptions.push({
        value: normalizedValue,
        label: t('settings.contextLimitCustomOption', {
          value: formatted,
          defaultValue: `${formatted} (custom)`,
        }),
      });
    }

    return presetOptions;
  }, [normalizedValue, t]);

  return (
    <Select
      value={toSelectValue(value)}
      options={options}
      style={{ width: '100%' }}
      getPopupContainer={() => document.body}
      placeholder={t('settings.contextLimitSelectPlaceholder', { defaultValue: '选择上下文窗口' })}
      onChange={(nextValue) => {
        if (nextValue === DEFAULT_CONTEXT_LIMIT_VALUE) {
          onChange?.(undefined);
          return;
        }
        onChange?.(normalizeContextLimit(nextValue));
      }}
    />
  );
};
