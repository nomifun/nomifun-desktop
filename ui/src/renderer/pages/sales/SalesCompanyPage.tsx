import { Button, Input } from '@arco-design/web-react';
import { CheckOne, Save } from '@icon-park/react';
import React, { useEffect, useState } from 'react';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { useSalesWorkspace } from './SalesWorkspaceContext';
import { SalesPageHeader, SalesSection } from './SalesUi';
import type { SalesCompanyProfile } from './salesWorkspace';

const { TextArea } = Input;

const SalesCompanyPage: React.FC = () => {
  const { workspace, saveCompanyProfile } = useSalesWorkspace();
  const [draft, setDraft] = useState<SalesCompanyProfile>(workspace.companyProfile);
  const [message, contextHolder] = useArcoMessage({ maxCount: 2 });

  useEffect(() => setDraft(workspace.companyProfile), [workspace.companyProfile]);

  const update = (key: keyof SalesCompanyProfile, value: string) =>
    setDraft((current) => ({ ...current, [key]: value }));

  const handleSave = (event: React.FormEvent) => {
    event.preventDefault();
    if (!draft.companyName.trim() || !draft.businessSummary.trim() || !draft.valueProposition.trim()) {
      message.warning('请至少填写公司名称、业务简介和价值主张。');
      return;
    }
    saveCompanyProfile({
      ...draft,
      companyName: draft.companyName.trim(),
      website: draft.website.trim(),
      senderName: draft.senderName.trim(),
      senderEmail: draft.senderEmail.trim(),
    });
    message.success('公司资料已保存。');
  };

  return (
    <div className='sales-page-scroll'>
      {contextHolder}
      <form className='sales-page sales-form-page' onSubmit={handleSave}>
        <SalesPageHeader
          title='我的公司'
          description='这些信息将用于筛选目标公司和准备个性化联络内容。'
          action={
            <Button htmlType='submit' type='primary' icon={<Save size={15} />}>
              保存资料
            </Button>
          }
        />

        <SalesSection title='基础信息' description='让系统知道你代表谁，以及客户可以在哪里了解你。'>
          <div className='sales-form-grid'>
            <label className='sales-field'>
              <span>公司名称 <em>必填</em></span>
              <Input value={draft.companyName} onChange={(value) => update('companyName', value)} placeholder='例如：Example Trading 株式会社' />
            </label>
            <label className='sales-field'>
              <span>公司网站</span>
              <Input value={draft.website} onChange={(value) => update('website', value)} placeholder='https://example.com' />
            </label>
            <label className='sales-field'>
              <span>默认联系人姓名</span>
              <Input value={draft.senderName} onChange={(value) => update('senderName', value)} placeholder='用于联系表单中的姓名字段' />
            </label>
            <label className='sales-field'>
              <span>默认联系邮箱</span>
              <Input type='email' value={draft.senderEmail} onChange={(value) => update('senderEmail', value)} placeholder='sales@example.com' />
            </label>
          </div>
        </SalesSection>

        <SalesSection title='业务定位' description='写清楚事实和差异点，避免 AI 生成空泛或夸张的销售文案。'>
          <div className='sales-form-stack'>
            <label className='sales-field'>
              <span>业务简介 <em>必填</em></span>
              <TextArea
                value={draft.businessSummary}
                onChange={(value) => update('businessSummary', value)}
                placeholder='你们提供什么产品或服务，主要面向哪些市场？'
                autoSize={{ minRows: 3, maxRows: 6 }}
              />
            </label>
            <label className='sales-field'>
              <span>价值主张 <em>必填</em></span>
              <TextArea
                value={draft.valueProposition}
                onChange={(value) => update('valueProposition', value)}
                placeholder='为什么目标公司应该与你联系？请写具体优势、能力或合作条件。'
                autoSize={{ minRows: 3, maxRows: 6 }}
              />
            </label>
            <label className='sales-field'>
              <span>理想客户</span>
              <TextArea
                value={draft.targetCustomer}
                onChange={(value) => update('targetCustomer', value)}
                placeholder='例如：日本生活杂货零售商、10–50 家门店、有进口经验。'
                autoSize={{ minRows: 2, maxRows: 5 }}
              />
            </label>
          </div>
        </SalesSection>

        <div className='sales-form-footer'>
          <span><CheckOne size={14} />资料仅保存在当前登录账号的销售空间中</span>
          <Button htmlType='submit' type='primary' icon={<Save size={15} />}>
            保存资料
          </Button>
        </div>
      </form>
    </div>
  );
};

export default SalesCompanyPage;
