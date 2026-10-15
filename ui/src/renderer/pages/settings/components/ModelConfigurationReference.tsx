import type { ModelConfigurationDefaults } from '@/common/protocolBindings/ModelConfigurationDefaults';
import { openExternalUrl } from '@/renderer/utils/platform';
import { Button, Popover } from '@arco-design/web-react';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

function documentationLinks(sources: string[]) {
  const links = new Map<string, { url: string; host: string; path: string }>();
  for (const source of sources) {
    try {
      const url = new URL(source);
      if (url.protocol !== 'https:' || url.username || url.password) continue;
      let path = url.pathname.replace(/^\/+|\/+$/g, '');
      try { path = decodeURIComponent(path); } catch { /* Keep a malformed escaped path readable. */ }
      links.set(url.href, { url: url.href, host: url.host, path });
    } catch { /* Documentation metadata must not break the editor. */ }
  }
  return [...links.values()];
}

/** Optional reference material, separate from the user's current settings. */
export default function ModelConfigurationReference({ defaults }: { defaults: ModelConfigurationDefaults }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const links = documentationLinks(defaults.sources);
  const effort = defaults.provider_params.reasoning_effort;
  const protocol = defaults.protocol_id === 'openai.responses' ? 'Responses'
    : defaults.protocol_id === 'openai.chat_text' ? 'Chat Completions' : defaults.protocol_id;
  return <Popover trigger='click' position='left' popupVisible={open} onVisibleChange={setOpen}
    getPopupContainer={() => document.body} unmountOnExit escToClose style={{ width: 380 }}
    content={<section id={panelId} aria-label={t('settings.modelAdvanced.referenceTitle')} className='max-h-[calc(100vh-64px)] space-y-10px overflow-y-auto text-12px leading-18px'>
      <div>
        <div className='break-words font-600 text-t-primary'>{defaults.model}</div>
        <div className='text-11px text-t-tertiary'>
          {t('settings.modelAdvanced.documentedDefaults', { date: defaults.checked_at })}
        </div>
      </div>
      <div className='space-y-4px text-t-secondary'>
        <div>{t('settings.modelAdvanced.referenceProtocol', { protocol })}</div>
        {defaults.context_limit !== undefined && <div>{t('settings.modelAdvanced.documentedContext', { tokens: defaults.context_limit.toLocaleString() })}</div>}
        {defaults.output_limit !== undefined && <div>{t('settings.modelAdvanced.documentedOutput', { tokens: defaults.output_limit.toLocaleString() })}</div>}
        {typeof effort === 'string' && <div>{t('settings.modelAdvanced.recommendedReasoning', {
          effort: t(`settings.modelAdvanced.reasoningEffortOptions.${effort}`, { defaultValue: effort }),
        })}</div>}
      </div>
      {!!links.length && <div className='max-h-220px space-y-4px overflow-y-auto' aria-label={t('settings.modelAdvanced.referenceDocuments')}>
        {links.map(link => <a key={link.url} href={link.url} target='_blank' rel='noreferrer' title={link.url}
          className='block rounded-4px px-6px py-5px text-primary-6 hover:bg-fill-2 focus-visible:outline-auto'
          onClick={event => {
            event.preventDefault();
            void openExternalUrl(link.url).catch(error => console.error('Failed to open model documentation:', error));
          }}>
          <span className='block break-all'>{link.path || link.host}</span>
          {link.path && <span className='block text-11px text-t-tertiary'>{link.host}</span>}
        </a>)}
      </div>}
      <div className='text-11px text-t-tertiary'>{t('settings.modelAdvanced.referenceHint')}</div>
    </section>}>
    <Button type='text' size='mini' aria-expanded={open} aria-controls={open ? panelId : undefined}>
      {t('settings.modelAdvanced.referenceTitle')}
    </Button>
  </Popover>;
}
