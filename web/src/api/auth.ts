import request from '@/utils/request';
import type {
  LoginStepOneResponse,
  TotpVerifyRequest,
  TotpSetupResponse,
  TotpStatusResponse,
  TotpEnableRequest,
} from '@/types/totp';

export const authApi = {
  // 检查是否需要初始设置
  checkInitialSetup: () =>
    request.get<{ needs_setup: boolean }>('/auth/setup/status'),

  // 初始设置
  initialSetup: (username: string, password: string) =>
    request.post<{ success: boolean }>('/auth/setup', { username, password }),

  // 修改密码
  changePassword: (oldPassword: string, newPassword: string) =>
    request.post<{ success: boolean }>('/auth/password', {
      old_password: oldPassword,
      new_password: newPassword,
    }),

  // 第一步登录：验证用户名密码
  login: (username: string, password: string) =>
    request.post<LoginStepOneResponse>('/auth/login', { username, password }),

  // 第二步登录：验证TOTP码
  verifyTotp: (data: TotpVerifyRequest) =>
    request.post<{ token: string; expires_in: number }>('/auth/totp/verify', data),

  // 获取TOTP状态
  getTotpStatus: () =>
    request.get<TotpStatusResponse>('/auth/totp/status'),

  // 初始化TOTP设置
  setupTotp: () =>
    request.post<TotpSetupResponse>('/auth/totp/setup'),

  // 启用TOTP
  enableTotp: (data: TotpEnableRequest) =>
    request.post<{ success: boolean }>('/auth/totp/enable', data),

  // 禁用TOTP
  disableTotp: (code: string) =>
    request.post<{ success: boolean }>('/auth/totp/disable', { code }),

  // 重新生成备用码
  regenerateBackupCodes: (code: string) =>
    request.post<{ backup_codes: string[] }>('/auth/totp/regenerate-backup-codes', { code }),
};
