/** Thin native startup entry: no HTTP, settings, auth, database or application routes. */
import { createRoot } from 'react-dom/client';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import '@arco-design/web-react/es/_util/react-19-adapter';
import '@arco-design/web-react/dist/css/arco.css';
import 'uno.css';
import './styles/themes/index.css';
import './styles/arco-override.css';
import en from './services/i18n/locales/en-US/common.json';
import zh from './services/i18n/locales/zh-CN/common.json';
import { desktopStartup } from '@/common/adapter/tauriStartup';
import DesktopStartup from './components/startup/DesktopStartup';

const i18n = createInstance();
let language = navigator.language.startsWith('zh') ? 'zh-CN' : 'en-US';
try {
  const saved = localStorage.getItem('i18nextLng');
  if (saved === 'zh-CN' || saved === 'en-US') language = saved;
} catch {}
void i18n
  .use(initReactI18next)
  .init({ lng: language, resources: { 'en-US': { translation: { common: en } }, 'zh-CN': { translation: { common: zh } } } })
  .then(() => {
    createRoot(document.getElementById('root')!).render(
      <I18nextProvider i18n={i18n}>
        <DesktopStartup io={desktopStartup} />
      </I18nextProvider>
    );
  });
