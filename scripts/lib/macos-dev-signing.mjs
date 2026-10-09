// Only an explicitly configured installed identity may replace ad-hoc signing.
// This module never enumerates certificates or changes Keychain access policy.
export function macosDevelopmentSigningIdentity(environment = process.env) {
  const configured = environment.NOMIFUN_MACOS_DEV_SIGN_IDENTITY;
  if (configured === undefined) return '-';
  if (typeof configured !== 'string' || !configured.trim()
    || /[\0\r\n]/.test(configured) || (configured.trim().startsWith('-') && configured.trim() !== '-')) {
    throw new Error('NOMIFUN_MACOS_DEV_SIGN_IDENTITY must name an installed codesign identity or be -');
  }
  return configured.trim();
}

export function macosDevelopmentSigningNotice(environment = process.env) {
  return macosDevelopmentSigningIdentity(environment) === '-'
    ? '[macOS dev] 使用 ad-hoc 签名。可显式设置 NOMIFUN_MACOS_DEV_SIGN_IDENTITY 使用已安装的稳定签名身份。'
    : null;
}

export function macosDevelopmentSealArguments(appPath, identity) {
  return ['--force', ...(identity === '-' ? [] : ['--options', 'runtime', '--timestamp']), '--sign', identity, appPath];
}
