import { createInstance } from 'i18next';
import { createRoot } from 'react-dom/client';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import en from '@/renderer/services/i18n/locales/en-US/settings.json';
import zh from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import KeyboardSubtitle from './KeyboardSubtitle';
import '@/renderer/styles/themes/index.css';

const i18n = createInstance();
document.documentElement.classList.add('keyboard-subtitle-window');
let language = navigator.language;
try { language = localStorage.getItem('i18nextLng') || language; } catch { /* use the OS preference */ }
void i18n.use(initReactI18next).init({
  lng: language.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': { translation: { settings: { keyboard: en.keyboard } } },
    'zh-CN': { translation: { settings: { keyboard: zh.keyboard } } },
  },
  interpolation: { escapeValue: false },
});
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><KeyboardSubtitle /></I18nextProvider>);
