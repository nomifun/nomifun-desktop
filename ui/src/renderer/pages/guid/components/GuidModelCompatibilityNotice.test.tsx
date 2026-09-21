import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import messages from '@/renderer/services/i18n/locales/zh-CN/index';
import GuidModelCompatibilityNotice from './GuidModelCompatibilityNotice';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  resources: { 'zh-CN': { translation: messages } },
});

afterEach(cleanup);

test('names the current model and every missing capability, then exposes both recovery paths', () => {
  const actions: string[] = [];
  const page = render(
    <I18nextProvider i18n={i18n}>
      <GuidModelCompatibilityNotice
        modelLabel='普通聊天模型'
        providerLabel='测试供应商'
        missingCapabilities={['function_calling', 'reasoning']}
        compatibleModelCount={2}
        canConfigureCurrentModel
        onChooseCompatibleModel={() => actions.push('choose')}
        onOpenModelConfiguration={() => actions.push('configure')}
      />
    </I18nextProvider>,
  );

  expect(page.getByText('普通聊天模型')).toBeTruthy();
  expect(page.getByText(/测试供应商/)).toBeTruthy();
  expect(page.getByText('工具调用')).toBeTruthy();
  expect(page.getByText('推理')).toBeTruthy();
  expect(page.getByText(/输入、附件、项目与 Agent 选择都已保留/)).toBeTruthy();

  fireEvent.click(page.getByRole('button', { name: '选择兼容模型（2）' }));
  fireEvent.click(page.getByRole('button', { name: '检查模型与协议配置' }));
  expect(actions).toEqual(['choose', 'configure']);
});

test('disables the empty compatible-model path and routes managed models to the catalog', () => {
  const page = render(
    <I18nextProvider i18n={i18n}>
      <GuidModelCompatibilityNotice
        modelLabel='托管模型'
        providerLabel='NomiFun'
        missingCapabilities={['function_calling']}
        compatibleModelCount={0}
        canConfigureCurrentModel={false}
        onChooseCompatibleModel={() => undefined}
        onOpenModelConfiguration={() => undefined}
      />
    </I18nextProvider>,
  );

  expect(page.getByRole('button', { name: '选择兼容模型（0）' }).hasAttribute('disabled')).toBe(true);
  expect(page.getByRole('button', { name: '查看聊天模型' })).toBeTruthy();
  expect(page.getByText(/暂无其他兼容模型/)).toBeTruthy();
});
