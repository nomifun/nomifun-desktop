import { Button, Popover, Switch } from '@arco-design/web-react';
import { EveryUser } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { TDecisionPolicy, TDelegationPolicy } from '@/common/types/agentExecution/agentExecutionTypes';

export type CollaborationPolicyValue = {
  delegationPolicy: TDelegationPolicy;
  decisionPolicy: TDecisionPolicy;
};

type CollaborationPolicyControlProps = CollaborationPolicyValue & {
  runtimeType?: string;
  onChange: (next: CollaborationPolicyValue) => void | Promise<void>;
  compact?: boolean;
  className?: string;
};

const DELEGATION_OPTIONS: TDelegationPolicy[] = ['disabled', 'automatic', 'prefer_parallel'];

const CollaborationPolicyControl: React.FC<CollaborationPolicyControlProps> = ({
  runtimeType,
  delegationPolicy,
  decisionPolicy,
  onChange,
  compact = false,
  className,
}) => {
  const { t } = useTranslation();
  if (runtimeType !== 'nomi') return null;

  const content = (
    <div className='flex w-260px flex-col gap-14px py-2px'>
      <div className='flex flex-col gap-7px'>
        <div>
          <div className='text-13px font-600 text-t-primary'>{t('collaboration.policy.title', { defaultValue: '协作策略' })}</div>
          <div className='mt-2px text-11px leading-16px text-t-tertiary'>
            {t('collaboration.policy.description', {
              defaultValue: '决定当前对话是否拆分任务，以及是否优先并行推进。',
            })}
          </div>
        </div>
        <div className='grid grid-cols-3 gap-5px'>
          {DELEGATION_OPTIONS.map((option) => {
            const active = option === delegationPolicy;
            return (
              <button
                key={option}
                type='button'
                className='rd-8px border px-7px py-7px text-11px transition-colors'
                style={{
                  color: active ? 'rgb(var(--primary-6))' : 'var(--color-text-2)',
                  borderColor: active ? 'rgb(var(--primary-6))' : 'var(--color-border-2)',
                  background: active ? 'color-mix(in srgb, rgb(var(--primary-6)) 10%, transparent)' : 'transparent',
                }}
                onClick={() =>
                  void onChange({
                    delegationPolicy: option,
                    decisionPolicy,
                  })
                }
              >
                {t(`collaboration.policy.delegation.${option}`, {
                  defaultValue: option === 'disabled' ? '关闭' : option === 'prefer_parallel' ? '优先并行' : '自动',
                })}
              </button>
            );
          })}
        </div>
      </div>

      <div className='flex items-start justify-between gap-12px'>
        <div className='min-w-0'>
          <div className='text-13px font-600 text-t-primary'>
            {t('collaboration.policy.askUser', {
              defaultValue: '关键决策时询问我',
            })}
          </div>
          <div className='mt-2px text-11px leading-16px text-t-tertiary'>
            {t('collaboration.policy.askUserDescription', {
              defaultValue: '协作者遇到无法安全判断的选择时暂停并询问。',
            })}
          </div>
        </div>
        <Switch
          size='small'
          checked={decisionPolicy === 'ask_user'}
          disabled={delegationPolicy === 'disabled'}
          onChange={(checked) =>
            void onChange({
              delegationPolicy,
              decisionPolicy: checked ? 'ask_user' : 'automatic',
            })
          }
        />
      </div>
    </div>
  );

  const active = delegationPolicy !== 'disabled';
  return (
    <Popover content={content} trigger='click' position='top' unmountOnExit>
      <Button
        type={compact ? 'text' : 'secondary'}
        shape={compact ? 'circle' : 'round'}
        size='small'
        className={className}
        aria-label={t('collaboration.policy.open', {
          defaultValue: '协作策略',
        })}
        aria-pressed={active}
        data-testid='collaboration-policy-control'
      >
        <span className='inline-flex items-center gap-5px'>
          <EveryUser theme='outline' size='15' fill={active ? 'rgb(var(--primary-6))' : 'currentColor'} strokeWidth={3} />
          {!compact && (
            <span>
              {t('collaboration.policy.button', {
                defaultValue: active ? '协作已启用' : '协作已关闭',
              })}
            </span>
          )}
          {compact && active && <span className='size-5px rd-full bg-primary-6' aria-hidden='true' />}
        </span>
      </Button>
    </Popover>
  );
};

export default CollaborationPolicyControl;
