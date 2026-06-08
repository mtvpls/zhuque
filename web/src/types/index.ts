// 任务类型
export interface Task {
  id: number;
  name: string;
  command: string;
  cron: string | string[]; // 支持单个或多个 cron 表达式
  type: 'cron' | 'manual' | 'startup';
  enabled: boolean;
  env?: string;
  pre_command?: string;
  post_command?: string;
  group_id?: number;
  working_dir?: string;
  timeout?: number; // 执行超时（秒），0 表示不超时
  target_type?: 'local' | 'remote';
  target_agent_id?: number;
  notification?: string | null; // JSON 序列化的 TaskNotificationConfig
  last_run_at?: string;
  last_run_duration?: number;
  next_run_at?: string;
  created_at: string;
  updated_at: string;
}

// 脚本类型
export interface Script {
  name: string;
  path: string;
  size: number;
  modified: string;
  is_dir: boolean;
}

// 环境变量类型
export interface EnvVar {
  id: number;
  key: string;
  value: string;
  remark?: string;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

// 依赖类型
export interface Dependence {
  id: number;
  name: string;
  dep_type: number; // 0: nodejs, 1: python, 2: linux
  status: number; // 0: installing, 1: installed, 2: failed, 3: removing, 4: removed
  log?: string; // JSON格式的日志数组
  remark?: string;
  created_at: string;
  updated_at: string;
}

// 订阅类型
export interface Subscription {
  id: number;
  name: string;
  url: string;
  branch: string;
  schedule: string;
  enabled: boolean;
  last_run_time?: string;
  last_run_status?: string;
  last_run_log?: string;
  created_at: string;
  updated_at: string;
}

// 日志类型
export interface Log {
  id: number;
  task_id: number;
  output?: string; // 列表接口不返回，详情接口才返回
  status: string;
  duration?: number; // 执行耗时（毫秒）
  created_at: string;
}

// 任务分组类型
export interface TaskGroup {
  id: number;
  name: string;
  description?: string;
  created_at: string;
  updated_at: string;
}

// 执行记录类型
export interface Execution {
  execution_id: string;
  task_id: number;
  task_name: string;
  status: 'running' | 'completed' | 'failed';
  started_at: string;
}

// 用户类型
export interface User {
  username: string;
  token: string;
}

// API响应类型
export interface ApiResponse<T = any> {
  data?: T;
  error?: string;
  message?: string;
}

// ─── 通知配置类型 ─────────────────────────────────────────────────────────────

export interface TelegramConfig {
  bot_token: string;
  chat_id: string;
  proxy?: string;
}

export interface PushPlusConfig {
  token: string;
  topic?: string;
}

export interface SmtpConfig {
  host: string;
  port: number;
  username: string;
  password: string;
  from: string;
  to: string[];
  use_tls: boolean;
}

export interface ResendConfig {
  api_key: string;
  from: string;
  to: string[];
}

export interface WeComConfig {
  key: string;
}

export interface WebhookConfig {
  url: string;
  method: string;
  headers: Record<string, string>;
  body_template: string;
}

export interface DingTalkConfig {
  access_token: string;
  secret?: string;
}

export interface FeishuConfig {
  webhook_url: string;
  sign_key?: string;
}

export interface BarkConfig {
  server_url: string;
  device_key: string;
  sound?: string;
  group?: string;
}

export interface NtfyConfig {
  server_url: string;
  topic: string;
  token?: string;
  priority: number;
}

export interface ChannelConfig {
  id: string;
  name: string;
  type: string;
  enabled: boolean;
  config: TelegramConfig | PushPlusConfig | SmtpConfig | ResendConfig | WeComConfig | WebhookConfig | DingTalkConfig | FeishuConfig | BarkConfig | NtfyConfig | Record<string, unknown>;
}

export interface NotificationConfig {
  enabled: boolean;
  on_success: boolean;
  on_failure: boolean;
  on_killed: boolean;
  on_login: boolean;
  channels: ChannelConfig[];
}

export interface TestChannelRequest {
  channel_type: string;
  config: Record<string, unknown>;
}

// ─── 任务级通知配置（覆盖全局通知管理）────────────────────────────────────────

export interface TaskNotificationConfig {
  /** 是否启用任务级别覆盖 */
  enabled: boolean;
  /** 覆盖：成功时是否通知（undefined = 跟随全局） */
  on_success?: boolean;
  /** 覆盖：失败时是否通知（undefined = 跟随全局） */
  on_failure?: boolean;
  /** 覆盖：终止时是否通知（undefined = 跟随全局） */
  on_killed?: boolean;
  /** 覆盖：指定生效的渠道 ID 列表（undefined 或空数组 = 使用全局所有已启用渠道） */
  channel_ids?: string[];
}
