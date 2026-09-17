import { expect, test } from 'bun:test';
import { asCapabilityId, asPackageId, type CapabilityCatalogItem } from '@/common/types/agentPlatform';
import { capabilityCategory } from './capabilityGroups';
import { capabilityProductCopy } from './model';

test('browser is one provider-neutral web module', () => {
  const capability = { id: asCapabilityId('browser'), version: '1.0.0' };
  expect(capabilityCategory(capability)).toBe('web');
  const item = { capability, display_name: 'browser', description: 'browser', source_package: { id: asPackageId('nomifun.browser'), version: '1.0.0' }, source_kind: 'bundled' } as CapabilityCatalogItem;
  expect(capabilityProductCopy(item, 'zh-CN').name).toBe('浏览器');
  expect(capabilityProductCopy(item, 'zh-CN').description).toContain('不会扩大');
  expect(capabilityProductCopy(item, 'en-US').name).toBe('Browser');
});

test('local browser search has its own web-category identity and product copy', () => {
  const capability = { id: asCapabilityId('nomi_local_websearch'), version: '1.0.0' };
  expect(capabilityCategory(capability)).toBe('web');
  expect(capabilityCategory({ id: asCapabilityId('web.search'), version: '1.0.0' })).toBe('web');
  const item = { capability, source_package: { id: asPackageId('nomifun.local-websearch'), version: '1.0.0' }, source_kind: 'bundled' } as CapabilityCatalogItem;
  const copy = capabilityProductCopy(item, 'zh-CN');
  expect(copy.name).toBe('Nomi 本地网页搜索');
  expect(copy.description).toContain('公开网页');
  expect(copy.description).toContain('可引用来源');
  expect(copy.description).toContain('不读取会话登录态');
});
