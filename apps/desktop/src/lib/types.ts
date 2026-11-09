export type RuleId =
  | 'system_proxy'
  | 'proxy_bypass'
  | 'proxy_owner'
  | 'https_tls'
  | 'exit_region'
  | 'exit_stability'
  | 'dns'
  | 'route'
  | 'ipv6'
  | 'timezone'
  | 'clock'
  | 'client_signature'
  | 'environment'
  | 'existing_clients'
  | 'final_recheck';
export type Status = 'pass' | 'info' | 'warning' | 'fail' | 'skipped';
export interface Check {
  id: RuleId;
  status: Status;
  detail: string;
  elapsedMs: number;
  blocksLaunch: boolean;
}
export interface Profile {
  schemaVersion: number;
  proxyUrl: string;
  expectedCountry: string | null;
  expectedProxyExecutable: string;
  nonMainlandTimezone: boolean;
}
export interface Discovery {
  supported: boolean;
  candidates: { profile: Profile; source: string }[];
  message: string;
}
export type Notice = { kind: 'problem'; message: string } | { kind: 'recovered' };
export interface Guidance {
  title: string;
  explanation: string;
  steps: string[];
}
export type NotificationSetting = 'unknown' | 'unsupported' | 'disabled' | 'enabled';
export interface NotificationDiagnostics {
  appEnabled: boolean;
  serviceActive: boolean;
  demo: boolean;
  applicationName: string;
  applicationId: string;
  system: {
    authorization: 'unknown' | 'not_determined' | 'denied' | 'authorized' | 'provisional';
    alert: NotificationSetting;
    sound: NotificationSetting;
    notificationCenter: NotificationSetting;
    lockScreen: NotificationSetting;
    alertStyle: 'unknown' | 'none' | 'banner' | 'alert';
    queriedAt: number | null;
  };
  lastTest: {
    sequence: number;
    requestedAt: number;
    phase: 'pending' | 'submitted' | 'accepted' | 'rejected' | 'cancelled' | 'unconfirmed';
    receipt: 'unconfirmed' | 'received' | 'not_seen';
    inNotificationCenter: boolean | null;
    centerQueriedAt?: number | null;
  } | null;
  suggestions: string[];
}
export interface Snapshot {
  preflightMode: 'proxy_preflight';
  sessionId: string;
  sequence: number;
  policyVersion: number;
  platform: string;
  demo: boolean;
  launchAllowed: boolean;
  setupRequired?: boolean;
  notificationStatus?: string | null;
  notificationDiagnostics?: NotificationDiagnostics | null;
  recovering?: boolean;
  issueGuidance?: Guidance | null;
  checkGuidance?: { id: RuleId; advice: Guidance }[];
  notificationAction?: 'enable' | 'enable_reminders' | 'refresh' | null;
  scanning: boolean;
  launching: boolean;
  checks: Check[];
  profile: Profile;
  clients: { cli: boolean; desktop: boolean };
  monitorActive: boolean;
  issue: string | null;
  lastSample: number | null;
  requiresRecheck: boolean;
  canLaunch: boolean;
  events: { atMs: number; notice: Notice }[];
  launchMessage: string | null;
  nowMs: number;
}
export const rules: { id: RuleId; title: string; note: string }[] = [
  { id: 'system_proxy', title: '系统代理', note: 'HTTP / HTTPS 的实际设置' },
  { id: 'proxy_bypass', title: '代理旁路规则', note: 'PAC、自动发现与绕过列表' },
  { id: 'proxy_owner', title: '代理端口身份', note: '端口监听与进程身份' },
  { id: 'https_tls', title: 'HTTPS 与证书', note: '指定代理上的加密连接' },
  { id: 'exit_region', title: '代理出口', note: '出口地区与当前国家基线' },
  { id: 'exit_stability', title: '短时出口稳定性', note: '连续三次采样的一致性' },
  { id: 'dns', title: '系统 DNS', note: '本地解析器配置' },
  { id: 'route', title: '路由 / TUN', note: 'IPv4 默认路由与内网隧道' },
  { id: 'ipv6', title: 'IPv6 本地配置', note: '活动接口、地址状态与外网路由' },
  { id: 'timezone', title: '系统与进程时区', note: '时区有效性、一致性与个人偏好' },
  { id: 'clock', title: '时钟参考', note: '与 Cloudflare 时间进行粗略比较' },
  { id: 'client_signature', title: '官方程序签名', note: '静态验证 Apple / Anthropic 签名' },
  { id: 'environment', title: '启动环境冲突', note: '上游、TLS 与进程包装变量' },
  { id: 'existing_clients', title: '已有客户端进程', note: '旧进程与后台代理覆盖边界' },
  { id: 'final_recheck', title: '检测后复核', note: '确认代理未在检测期间改变' },
];
export const statusText: Record<Status, string> = {
  pass: '通过',
  info: '信息',
  warning: '提醒',
  fail: '异常',
  skipped: '未执行',
};

export function checkStatusText(check: Check): string {
  return check.id === 'ipv6' && check.status === 'warning'
    ? '覆盖未验证'
    : statusText[check.status];
}
