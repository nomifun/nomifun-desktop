import { InputNumber, Select } from '@arco-design/web-react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

const DEFAULT_CONTEXT_LIMIT_VALUE = 'default';
const CUSTOM_CONTEXT_LIMIT_VALUE = 'custom';

const CONTEXT_WINDOW_OPTIONS = [
  {
    value: DEFAULT_CONTEXT_LIMIT_VALUE,
    labelKey: 'settings.contextLimitDefaultOption',
    defaultLabel: '自动（运行时默认）',
  },
  { value: 32_000, defaultLabel: '32k' },
  { value: 64_000, defaultLabel: '64k' },
  { value: 128_000, defaultLabel: '128k' },
  { value: 200_000, defaultLabel: '200k' },
  { value: 1_000_000, defaultLabel: '1M' },
] as const;

const isPresetContextLimit = (value: number): boolean =>
  CONTEXT_WINDOW_OPTIONS.some((option) => option.value === value);

const normalizeContextLimit = (value: unknown): number | undefined => {
  if (typeof value === 'number' && Number.isFinite(value) && value > 0) return value;
  return undefined;
};

interface ContextLimitSelectProps {
  value?: number;
  onChange?: (value?: number) => void;
}

export const ContextLimitSelect: React.FC<ContextLimitSelectProps> = ({ value, onChange }) => {
  const { t } = useTranslation();
  const normalizedValue = normalizeContextLimit(value);
  const [customOpen, setCustomOpen] = useState(
    () => normalizedValue !== undefined && !isPresetContextLimit(normalizedValue)
  );

  useEffect(() => {
    if (normalizedValue !== undefined && !isPresetContextLimit(normalizedValue)) setCustomOpen(true);
  }, [normalizedValue]);

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

    presetOptions.push({
      value: CUSTOM_CONTEXT_LIMIT_VALUE,
      label: t('settings.outputLimitCustomOption', { defaultValue: '自定义' }),
    });

    return presetOptions;
  }, [t]);

  const selectedValue = customOpen || (normalizedValue !== undefined && !isPresetContextLimit(normalizedValue))
    ? CUSTOM_CONTEXT_LIMIT_VALUE
    : (normalizedValue ?? DEFAULT_CONTEXT_LIMIT_VALUE);

  return (
    <div className='space-y-6px'>
      <Select
        value={selectedValue}
        options={options}
        style={{ width: '100%' }}
        getPopupContainer={() => document.body}
        aria-label={t('settings.contextLimit', { defaultValue: '上下文窗口（tokens）' })}
        placeholder={t('settings.contextLimitSelectPlaceholder', { defaultValue: '选择上下文窗口' })}
        onChange={(nextValue) => {
          if (nextValue === CUSTOM_CONTEXT_LIMIT_VALUE) {
            setCustomOpen(true);
            return;
          }
          setCustomOpen(false);
          onChange?.(nextValue === DEFAULT_CONTEXT_LIMIT_VALUE ? undefined : normalizeContextLimit(nextValue));
        }}
      />
      {selectedValue === CUSTOM_CONTEXT_LIMIT_VALUE && (
        <InputNumber
          value={normalizedValue}
          min={1}
          max={0xffff_ffff}
          precision={0}
          placeholder={t('settings.contextLimitCustomPlaceholder', { defaultValue: '输入 tokens 数量' })}
          style={{ width: '100%' }}
          aria-label={t('settings.contextLimitCustomPlaceholder', { defaultValue: '输入 tokens 数量' })}
          onChange={(nextValue) => onChange?.(normalizeContextLimit(nextValue))}
        />
      )}
    </div>
  );
};
