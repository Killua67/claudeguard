import type { Snapshot } from './types';
/** Explicit projection. No profiles, addresses, session IDs, tickets or baseline. */
export function reportText(state: Snapshot): string {
  return JSON.stringify(
    {
      mode: state.demo ? 'offline-demo' : 'proxy-preflight',
      policyVersion: state.policyVersion,
      preflightMode: state.preflightMode ?? 'proxy_preflight',
      coverage: 'IPv4/IPv6 actual client traffic unverified',
      checks: state.checks,
      runtime: {
        active: state.monitorActive === true,
        requiresFullRecheck: state.requiresRecheck !== false,
        issue: state.issue ?? null,
        recovering: state.recovering === true,
        lastSample: sampleTime(state),
      },
      events: (state.events ?? [])
        .slice(0, 8)
        .map((e) => ({ at: eventTime(state, e.atMs), notice: e.notice })),
      notifications: state.notificationDiagnostics
        ? {
            appEnabled: state.notificationDiagnostics.appEnabled,
            system: {
              authorization: state.notificationDiagnostics.system.authorization,
              alert: state.notificationDiagnostics.system.alert,
              alertStyle: state.notificationDiagnostics.system.alertStyle,
              sound: state.notificationDiagnostics.system.sound,
              notificationCenter: state.notificationDiagnostics.system.notificationCenter,
              lockScreen: state.notificationDiagnostics.system.lockScreen,
            },
            lastTest: state.notificationDiagnostics.lastTest
              ? {
                  phase: state.notificationDiagnostics.lastTest.phase,
                  receipt: state.notificationDiagnostics.lastTest.receipt,
                  inNotificationCenter: state.notificationDiagnostics.lastTest.inNotificationCenter,
                }
              : null,
          }
        : null,
      scope:
        '仅代理链路预检，IPv4/IPv6 实际客户端覆盖未验证，不提供防直连保护。报告不能作为启动许可。',
    },
    null,
    2,
  );
}
export function eventTime(state: Snapshot, at: number): string {
  if (!Number.isFinite(at) || !Number.isFinite(state.nowMs)) return '时间未知';
  return new Date(Date.now() - Math.max(0, state.nowMs - at)).toLocaleTimeString('zh-CN', {
    hour12: false,
  });
}
export function sampleTime(state: Snapshot): string {
  return state.lastSample == null ? '尚未采样' : eventTime(state, state.lastSample);
}
