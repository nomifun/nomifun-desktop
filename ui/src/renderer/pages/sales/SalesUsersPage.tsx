import { Button, Input, Spin } from '@arco-design/web-react';
import { Lock, Plus, Refresh, Shield } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { SalesPageHeader, SalesSection } from './SalesUi';
import {
  createManagedSalesUser,
  listManagedSalesUsers,
  resetManagedSalesUserPassword,
  type ManagedSalesUser,
} from './salesTenantApi';

const formatAccountDate = (timestamp: number | null) => {
  if (!timestamp) return '尚未登录';
  return new Intl.DateTimeFormat('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(timestamp));
};

const SalesUsersPage: React.FC = () => {
  const [users, setUsers] = useState<ManagedSalesUser[]>([]);
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [resetUser, setResetUser] = useState<ManagedSalesUser | null>(null);
  const [resetPassword, setResetPassword] = useState('');
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [forbidden, setForbidden] = useState(false);
  const [message, contextHolder] = useArcoMessage({ maxCount: 2 });

  const refreshUsers = useCallback(async () => {
    setLoading(true);
    try {
      setUsers(await listManagedSalesUsers());
      setForbidden(false);
    } catch (error) {
      if (isBackendHttpError(error) && error.status === 403) {
        setForbidden(true);
      } else {
        message.error('账号列表加载失败，请稍后重试。');
      }
    } finally {
      setLoading(false);
    }
  }, [message]);

  useEffect(() => {
    void refreshUsers();
  }, [refreshUsers]);

  const createAccount = async (event: React.FormEvent) => {
    event.preventDefault();
    if (username.trim().length < 3) {
      message.warning('账号名至少需要 3 个字符。');
      return;
    }
    if (password.length < 8) {
      message.warning('密码至少需要 8 个字符。');
      return;
    }

    setSubmitting(true);
    try {
      const user = await createManagedSalesUser(username.trim(), password);
      setUsers((current) => [...current, user]);
      setUsername('');
      setPassword('');
      message.success(`已创建账号 ${user.username}。`);
    } catch (error) {
      const detail = isBackendHttpError(error) ? error.backendMessage : '';
      message.error(detail || '账号创建失败。');
    } finally {
      setSubmitting(false);
    }
  };

  const resetPasswordForUser = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!resetUser || resetPassword.length < 8) {
      message.warning('新密码至少需要 8 个字符。');
      return;
    }

    setSubmitting(true);
    try {
      await resetManagedSalesUserPassword(resetUser.userId, resetPassword);
      message.success(`已更新 ${resetUser.username} 的密码。`);
      setResetUser(null);
      setResetPassword('');
    } catch (error) {
      const detail = isBackendHttpError(error) ? error.backendMessage : '';
      message.error(detail || '密码更新失败。');
    } finally {
      setSubmitting(false);
    }
  };

  const sortedUsers = useMemo(
    () => [...users].sort((a, b) => Number(b.isOwner) - Number(a.isOwner) || a.createdAt - b.createdAt),
    [users]
  );

  if (forbidden) {
    return (
      <div className='sales-page-scroll'>
        {contextHolder}
        <div className='sales-page'>
          <SalesPageHeader title='账号与隔离' description='只有安装管理员可以创建和管理销售工作台账号。' />
          <SalesSection>
            <div className='sales-access-denied'>
              <Shield size={28} aria-hidden='true' />
              <strong>当前账号没有管理权限</strong>
              <span>你的公司资料、任务、目标公司和结果仍只属于当前账号。</span>
            </div>
          </SalesSection>
        </div>
      </div>
    );
  }

  return (
    <div className='sales-page-scroll'>
      {contextHolder}
      <div className='sales-page'>
        <SalesPageHeader
          title='账号与隔离'
          description='为每位销售人员创建独立账号。公司资料、计划、Agent 会话和结果按登录账号隔离。'
          action={<Button icon={<Refresh />} loading={loading} onClick={() => void refreshUsers()}>刷新</Button>}
        />

        <div className='sales-account-grid'>
          <SalesSection title='新建账号' description='账号名使用字母、数字、连字符或下划线；密码至少 8 个字符。'>
            <form className='sales-form-stack' onSubmit={createAccount}>
              <label className='sales-field'>
                <span>账号名 <em>必填</em></span>
                <Input
                  value={username}
                  onChange={setUsername}
                  maxLength={32}
                  autoComplete='off'
                  placeholder='例如：sales-japan'
                />
              </label>
              <label className='sales-field'>
                <span>初始密码 <em>必填</em></span>
                <Input.Password
                  value={password}
                  onChange={setPassword}
                  maxLength={128}
                  autoComplete='new-password'
                  placeholder='至少 8 个字符'
                  visibilityToggle
                />
              </label>
              <Button type='primary' htmlType='submit' icon={<Plus />} loading={submitting}>
                创建独立工作台
              </Button>
            </form>
          </SalesSection>

          <SalesSection title='隔离边界' description='本阶段采用同一套本地服务、多个独立账号。'>
            <ul className='sales-boundary-list'>
              <li><Shield size={16} /><span><strong>独立数据</strong>公司资料、任务、公司列表与结果不跨账号。</span></li>
              <li><Shield size={16} /><span><strong>独立会话</strong>Agent 会话与进度只允许所属账号读取。</span></li>
              <li><Lock size={16} /><span><strong>共享系统配置</strong>模型、Skills 和 Agent 由管理员统一维护。</span></li>
            </ul>
          </SalesSection>
        </div>

        <SalesSection
          title='现有账号'
          description={`共 ${users.length} 个账号。安装管理员负责共享 Agent、模型与系统配置。`}
        >
          {loading && users.length === 0 ? (
            <div className='sales-table-loading'><Spin size={24} /></div>
          ) : (
            <div className='sales-account-table-wrap'>
              <table className='sales-account-table'>
                <thead>
                  <tr><th>账号</th><th>权限</th><th>最近登录</th><th>创建时间</th><th><span className='sr-only'>操作</span></th></tr>
                </thead>
                <tbody>
                  {sortedUsers.map((user) => (
                    <tr key={user.userId}>
                      <td><strong>{user.username}</strong><small>{user.userId.slice(0, 8)}…</small></td>
                      <td>{user.isOwner ? <span className='sales-account-role'>安装管理员</span> : '销售账号'}</td>
                      <td>{formatAccountDate(user.lastLogin)}</td>
                      <td>{formatAccountDate(user.createdAt)}</td>
                      <td>
                        <Button
                          size='mini'
                          type='text'
                          onClick={() => { setResetUser(user); setResetPassword(''); }}
                        >
                          重设密码
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </SalesSection>

        {resetUser ? (
          <SalesSection title={`重设 ${resetUser.username} 的密码`} description='更新后请把新密码安全地交给账号使用者。'>
            <form className='sales-password-reset' onSubmit={resetPasswordForUser}>
              <Input.Password
                value={resetPassword}
                onChange={setResetPassword}
                maxLength={128}
                autoComplete='new-password'
                placeholder='输入至少 8 个字符的新密码'
                visibilityToggle
              />
              <Button type='primary' htmlType='submit' loading={submitting}>保存新密码</Button>
              <Button onClick={() => { setResetUser(null); setResetPassword(''); }}>取消</Button>
            </form>
          </SalesSection>
        ) : null}
      </div>
    </div>
  );
};

export default SalesUsersPage;
