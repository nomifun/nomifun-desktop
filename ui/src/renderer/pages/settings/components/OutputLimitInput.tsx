/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { InputNumber, Select } from '@arco-design/web-react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

const PROVIDER_DEFAULT_VALUE = 'provider-default';
const CUSTOM_VALUE = 'custom';
const MAX_OUTPUT_LIMIT_TOKENS = 0xffff_ffff;

export const OUTPUT_LIMIT_PRESETS = [
  1_024,
  2_048,
  4_096,
  8_192,
  16_384,
  32_768,
  65_536,
  131_072,
] as const;

export const OUTPUT_LIMIT_UNIT_MULTIPLIERS = {
  tokens: 1,
  k: 1_000,
  m: 1_000_000,
} as const;

export type OutputLimitUnit = keyof typeof OUTPUT_LIMIT_UNIT_MULTIPLIERS;

interface OutputLimitInputProps {
  value?: number;
  onChange?: (value?: number) => void;
  /** Compact task editors keep conversion feedback only while a custom value is active. */
  compact?: boolean;
}

export const normalizeOutputLimit = (value: unknown): number | undefined => {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) return undefined;
  const normalized = Math.trunc(value);
  return normalized > 0 ? normalized : undefined;
};

export const outputLimitFromDisplayValue = (
  value: unknown,
  unit: OutputLimitUnit
): number | undefined => {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) return undefined;
  const normalized = normalizeOutputLimit(
    Math.round(value * OUTPUT_LIMIT_UNIT_MULTIPLIERS[unit])
  );
  return normalized !== undefined && normalized <= MAX_OUTPUT_LIMIT_TOKENS
    ? normalized
    : undefined;
};

export const displayValueFromOutputLimit = (
  value: unknown,
  unit: OutputLimitUnit
): number | undefined => {
  const normalized = normalizeOutputLimit(value);
  return normalized === undefined ? undefined : normalized / OUTPUT_LIMIT_UNIT_MULTIPLIERS[unit];
};

export const formatOutputLimit = (value: number): string => new Intl.NumberFormat().format(value);

const isOutputLimitPreset = (value: number | undefined): boolean =>
  value !== undefined && (OUTPUT_LIMIT_PRESETS as readonly number[]).includes(value);

const unitPrecision = (unit: OutputLimitUnit): number => {
  switch (unit) {
    case 'tokens':
      return 0;
    case 'k':
      return 3;
    case 'm':
      return 6;
  }
};

const unitStep = (unit: OutputLimitUnit): number => {
  switch (unit) {
    case 'tokens':
      return 1;
    case 'k':
      return 1;
    case 'm':
      return 0.1;
  }
};

export const OutputLimitInput: React.FC<OutputLimitInputProps> = ({ value, onChange, compact = false }) => {
  const { t } = useTranslation();
  const normalizedValue = normalizeOutputLimit(value);
  const [customActive, setCustomActive] = useState(
    () => normalizedValue !== undefined && !isOutputLimitPreset(normalizedValue)
  );
  const [unit, setUnit] = useState<OutputLimitUnit>('tokens');

  useEffect(() => {
    if (normalizedValue === undefined) {
      setCustomActive(false);
    } else if (!isOutputLimitPreset(normalizedValue)) {
      setCustomActive(true);
    }
  }, [normalizedValue]);

  const isCustom =
    customActive || (normalizedValue !== undefined && !isOutputLimitPreset(normalizedValue));
  const selectValue = isCustom
    ? CUSTOM_VALUE
    : (normalizedValue ?? PROVIDER_DEFAULT_VALUE);
  const displayValue = displayValueFromOutputLimit(normalizedValue, unit);

  const presetOptions = useMemo(
    () => [
      {
        value: PROVIDER_DEFAULT_VALUE,
        label: t('settings.outputLimitDefaultOption', {
          defaultValue: 'Default (provider decides)',
        }),
      },
      ...OUTPUT_LIMIT_PRESETS.map((preset) => ({
        value: preset,
        label: t('settings.outputLimitPresetOption', {
          value: formatOutputLimit(preset),
          defaultValue: `${formatOutputLimit(preset)} tokens`,
        }),
      })),
      {
        value: CUSTOM_VALUE,
        label: t('settings.outputLimitCustomOption', { defaultValue: 'Custom' }),
      },
    ],
    [t]
  );

  const unitOptions = useMemo(
    () => [
      {
        value: 'tokens',
        label: t('settings.outputLimitUnitTokens', { defaultValue: 'tokens' }),
      },
      {
        value: 'k',
        label: t('settings.outputLimitUnitK', { defaultValue: 'k (×1,000)' }),
      },
      {
        value: 'm',
        label: t('settings.outputLimitUnitM', { defaultValue: 'M (×1,000,000)' }),
      },
    ],
    [t]
  );

  return (
    <div className='space-y-6px' data-output-limit-input>
      <Select
        value={selectValue}
        options={presetOptions}
        style={{ width: '100%' }}
        getPopupContainer={() => document.body}
        onChange={(nextValue) => {
          if (nextValue === PROVIDER_DEFAULT_VALUE) {
            setCustomActive(false);
            onChange?.(undefined);
            return;
          }
          if (nextValue === CUSTOM_VALUE) {
            setCustomActive(true);
            return;
          }
          setCustomActive(false);
          onChange?.(normalizeOutputLimit(nextValue));
        }}
      />

      {isCustom && (
        <div
          className='grid grid-cols-1 gap-8px sm:grid-cols-[minmax(0,1fr)_168px]'
          data-output-limit-custom
        >
          <InputNumber
            value={displayValue}
            min={1 / OUTPUT_LIMIT_UNIT_MULTIPLIERS[unit]}
            max={MAX_OUTPUT_LIMIT_TOKENS / OUTPUT_LIMIT_UNIT_MULTIPLIERS[unit]}
            precision={unitPrecision(unit)}
            step={unitStep(unit)}
            style={{ width: '100%' }}
            suffix={unit === 'tokens' ? 'tokens' : unit}
            aria-label={t('settings.outputLimitCustomAmount', {
              defaultValue: 'Custom output limit',
            })}
            placeholder={t('settings.outputLimitCustomAmount', {
              defaultValue: 'Enter a custom value',
            })}
            onChange={(nextValue) => {
              const nextLimit = outputLimitFromDisplayValue(nextValue, unit);
              if (nextLimit === undefined) setCustomActive(false);
              onChange?.(nextLimit);
            }}
          />
          <Select
            value={unit}
            options={unitOptions}
            style={{ width: '100%' }}
            getPopupContainer={() => document.body}
            aria-label={t('settings.outputLimitCustomUnit', {
              defaultValue: 'Custom output limit unit',
            })}
            onChange={(nextUnit) => setUnit(nextUnit as OutputLimitUnit)}
          />
        </div>
      )}

      <div
        className='text-11px leading-4 text-t-tertiary'
        aria-live='polite'
        data-output-limit-conversion
      >
        {normalizedValue === undefined
          ? t('settings.outputLimitProviderDefault', {
              defaultValue: 'Leave unset to use the provider default.',
            })
          : t('settings.outputLimitConverted', {
              value: formatOutputLimit(normalizedValue),
              defaultValue: `Converted value: ${formatOutputLimit(normalizedValue)} tokens`,
            })}
      </div>
      {!compact && (
        <div className='text-11px leading-4 text-t-tertiary'>
          {t('settings.outputLimitCustomHint', {
            defaultValue: 'Choose a common limit, or select Custom to enter a value and unit.',
          })}
        </div>
      )}
    </div>
  );
};
