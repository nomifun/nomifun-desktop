import DingTalkLogo from '@/renderer/assets/channel-logos/dingtalk.svg';
import FeishuLogo from '@/renderer/assets/channel-logos/lark.svg';
import WeComLogo from '@/renderer/assets/channel-logos/wecom.svg';
import WeChatLogo from '@/renderer/assets/channel-logos/weixin.svg';
import { Dropdown, Menu } from '@arco-design/web-react';
import { Check, Down } from '@icon-park/react';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { useTranslation } from 'react-i18next';

const applications = [
  { id: 'feishu', name: 'office.feishu', logo: FeishuLogo },
  { id: 'wechat', name: 'office.appSwitcher.wechat', logo: WeChatLogo },
  { id: 'wecom', name: 'office.appSwitcher.wecom', logo: WeComLogo },
  { id: 'dingtalk', name: 'office.appSwitcher.dingtalk', logo: DingTalkLogo },
] as const;

/** The remaining applications are display-only until their integrations exist. */
export function OfficeAppSwitcher() {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const close = () => { setVisible(false); trigger.current?.focus(); };
  const dismiss = (event: KeyboardEvent) => {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); }
  };
  useEffect(() => {
    if (!visible) return;
    const frame = requestAnimationFrame(() => popup.current?.querySelector<HTMLElement>('[data-office-app="feishu"]')?.focus());
    return () => cancelAnimationFrame(frame);
  }, [visible]);
  return <Dropdown trigger='click' position='bl' popupVisible={visible} onVisibleChange={setVisible} getPopupContainer={() => document.body} droplist={
    <div ref={popup} className='office-app-popup' onKeyDown={dismiss}>
      <div className='office-app-popup-heading'><strong>{t('office.appSwitcher.title')}</strong><span>{t('office.appSwitcher.hint')}</span></div>
      <Menu className='office-app-menu' selectedKeys={['feishu']} onClickMenuItem={close}>
        {applications.map(application => <Menu.Item key={application.id} data-office-app={application.id} disabled={application.id !== 'feishu'} aria-disabled={application.id !== 'feishu'} aria-current={application.id === 'feishu' ? 'true' : undefined}>
          <div className='office-app-option'><img src={application.logo} alt='' aria-hidden='true' /><span>{t(application.name)}</span>{application.id === 'feishu' ? <Check size={16} aria-hidden='true' /> : <small>{t('office.appSwitcher.unavailable')}</small>}</div>
        </Menu.Item>)}
      </Menu>
    </div>
  }>
    <button ref={trigger} type='button' className='office-app-trigger' aria-label={t('office.appSwitcher.label')} aria-haspopup='menu' aria-expanded={visible} onKeyDown={event => { dismiss(event); if (event.key === 'ArrowDown') { event.preventDefault(); setVisible(true); } }}><img src={FeishuLogo} alt='' aria-hidden='true' /><span>{t('office.feishu')}</span><Down size={12} aria-hidden='true' /></button>
  </Dropdown>;
}
