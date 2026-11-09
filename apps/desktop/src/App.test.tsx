// @vitest-environment jsdom
import { StrictMode } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { Snapshot, Profile } from './lib/types';
import { rules } from './lib/types';
const backend = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  state: null as Snapshot | null,
  sequence: 0,
  rejectConfig: false,
  release: null as (() => void) | null,
  holdScan: false,
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: backend.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    backend.listeners.set(name, handler);
    return () => {
      if (backend.listeners.get(name) === handler) backend.listeners.delete(name);
    };
  }),
}));
import { App } from './App';
function next() {
  return structuredClone({ ...backend.state!, sequence: ++backend.sequence });
}
beforeEach(() => {
  backend.sequence = 0;
  backend.rejectConfig = false;
  backend.holdScan = false;
  backend.release = null;
  backend.listeners.clear();
  backend.invoke.mockReset();
  backend.state = {
    preflightMode: 'proxy_preflight',
    sessionId: 'ui-fixture',
    sequence: 0,
    policyVersion: 1,
    platform: 'demo-offline',
    demo: true,
    launchAllowed: false,
    scanning: false,
    launching: false,
    checks: rules.map((r) => ({
      id: r.id,
      status: 'pass',
      detail: '离线模拟证据',
      elapsedMs: 0,
      blocksLaunch: false,
    })),
    profile: {
      schemaVersion: 1,
      proxyUrl: 'http://127.0.0.1:17890',
      expectedCountry: 'US',
      expectedProxyExecutable: '/fixture/proxy',
      nonMainlandTimezone: true,
    },
    clients: { cli: true, desktop: true },
    monitorActive: true,
    issue: null,
    lastSample: 1,
    requiresRecheck: false,
    canLaunch: false,
    events: [],
    launchMessage: null,
    nowMs: 1,
  };
  backend.invoke.mockImplementation(async (name: string, args?: Record<string, unknown>) => {
    if (name === 'notification_status') return '离线演示不发送通知';
    if (name === 'snapshot') return next();
    if (name === 'scan') {
      if (backend.holdScan) {
        const results = backend.state!.checks;
        backend.state!.checks = [];
        backend.state!.scanning = true;
        backend.listeners.get('guard://state')?.({ payload: next() });
        await new Promise<void>((resolve) => {
          backend.release = resolve;
        });
        backend.state!.checks = results;
      }
      backend.state!.scanning = false;
      return next();
    }
    if (name === 'configure') {
      if (backend.rejectConfig) throw '代理地址无效，未应用配置';
      backend.state!.profile = args!.profile as Profile;
      return next();
    }
    if (name === 'hide_main') return;
    throw new Error(`unexpected command: ${name}`);
  });
  Object.defineProperty(HTMLDialogElement.prototype, 'showModal', {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.setAttribute('open', '');
    },
  });
  Object.defineProperty(HTMLDialogElement.prototype, 'close', {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.removeAttribute('open');
    },
  });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
async function ready() {
  const user = userEvent.setup();
  render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
  await screen.findByRole('heading', { name: '离线演示已完成' });
  return user;
}
function scans() {
  return backend.invoke.mock.calls.filter(([name]) => name === 'scan').length;
}
it('navigates all pages and filters the offline catalog without requests', async () => {
  const user = await ready();
  await user.click(screen.getByRole('button', { name: /事件记录/ }));
  expect(screen.getByRole('heading', { name: '异常与恢复记录' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: '返回总览' }));
  await user.click(screen.getByRole('button', { name: /Claude 域名清单/ }));
  expect(screen.getByText('41 条')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'CLI' }));
  expect(screen.getByText('25 条')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: '返回总览' }));
  await user.click(screen.getByRole('button', { name: /系统 DNS/ }));
  expect(
    within(document.querySelector('.detail')!).getByRole('heading', { name: '系统 DNS' }),
  ).toBeTruthy();
});
it('applies settings then returns to overview with visible completion feedback', async () => {
  const user = await ready();
  await user.click(screen.getByRole('button', { name: '设置' }));
  const input = screen.getByRole('textbox', { name: /本机 HTTP 代理地址/ });
  await user.clear(input);
  await user.type(input, 'http://127.0.0.1:9999');
  await user.click(screen.getByRole('button', { name: '应用并查看模拟检查' }));
  await screen.findByRole('heading', { name: '检测流程' });
  expect((await screen.findByRole('status')).textContent).toContain(
    '演示配置已应用，离线检测已完成',
  );
  expect(backend.state!.profile.proxyUrl).toBe('http://127.0.0.1:9999');
  expect(scans()).toBe(2);
});
it('keeps invalid settings visible and never scans the old profile after failure', async () => {
  const user = await ready();
  await user.click(screen.getByRole('button', { name: '设置' }));
  backend.rejectConfig = true;
  await user.click(screen.getByRole('button', { name: '应用并查看模拟检查' }));
  expect((await within(screen.getByRole('dialog')).findByRole('alert')).textContent).toContain(
    '代理地址无效',
  );
  expect(screen.getByRole('heading', { name: '使用环境配置' })).toBeTruthy();
  expect(scans()).toBe(1);
});
it('shows progress and rejects double clicks while a scan is still pending', async () => {
  const user = await ready();
  backend.holdScan = true;
  await user.dblClick(screen.getByRole('button', { name: /重新检测/ }));
  expect(screen.getByRole('button', { name: '检测中…' }).hasAttribute('disabled')).toBe(true);
  expect(screen.getByRole('heading', { name: '正在逐项检查环境' })).toBeTruthy();
  expect(scans()).toBe(2);
  expect(within(document.querySelector('.detail')!).getByText('正在读取与验证…')).toBeTruthy();
  await act(async () => {
    backend.release?.();
  });
  expect((await screen.findByRole('status')).textContent).toContain('离线检测已完成');
});
it('explains both demo launch buttons without invoking the native launcher', async () => {
  const user = await ready();
  for (const name of ['CLI', 'Desktop']) {
    await user.click(screen.getByRole('button', { name: new RegExp(`${name} 启动说明`) }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/本次没有启动任何客户端/)).toBeTruthy();
    await user.click(within(dialog).getByRole('button', { name: '我知道了' }));
    expect(screen.queryByRole('dialog')).toBeNull();
  }
  expect(backend.invoke.mock.calls.some(([name]) => name === 'launch')).toBe(false);
  await user.click(screen.getByRole('button', { name: '查看通知说明' }));
  expect((await screen.findByRole('status')).textContent).toContain('演示模式不发送系统通知');
  expect(backend.invoke.mock.calls.some(([name]) => name === 'enable_notifications')).toBe(false);
});
it('copies a redacted report and confirms success', async () => {
  const user = await ready();
  const write = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue();
  await user.click(screen.getByRole('button', { name: '复制报告' }));
  expect((await screen.findByRole('status')).textContent).toContain('已复制到剪贴板');
  expect(write.mock.calls[0][0]).not.toContain('/fixture/proxy');
  expect(write.mock.calls[0][0]).not.toContain('canLaunch');
});
it('reports clipboard failure instead of leaving a silent button', async () => {
  const user = await ready();
  vi.spyOn(navigator.clipboard, 'writeText').mockRejectedValue(new Error('denied'));
  await user.click(screen.getByRole('button', { name: '复制报告' }));
  expect((await screen.findByRole('alert')).textContent).toContain('复制报告失败');
});
it('routes hide through the dedicated window command', async () => {
  const user = await ready();
  await user.click(screen.getByRole('button', { name: '收起窗口' }));
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('hide_main'));
});
it('can hide during a pending scan without cancelling or clearing its progress', async () => {
  const user = await ready();
  backend.holdScan = true;
  await user.click(screen.getByRole('button', { name: /重新检测/ }));
  await user.click(screen.getByRole('button', { name: '收起窗口' }));
  await waitFor(() => expect(backend.invoke).toHaveBeenCalledWith('hide_main'));
  expect(screen.getByRole('button', { name: '检测中…' }).hasAttribute('disabled')).toBe(true);
  expect(screen.queryByRole('alert')).toBeNull();
  expect(scans()).toBe(2);
  await act(async () => {
    backend.release?.();
  });
  await screen.findByRole('status');
});

it('restores the original single-window layout and keeps diagnostics mounted under sheets', async () => {
  const user = await ready();
  expect(document.querySelector('aside')).toBeNull();
  expect(document.querySelector('.hero .hero-status')).toBeTruthy();
  expect(document.querySelector('.hero .counts')).toBeTruthy();
  expect(document.querySelector('.diagnostics .steps')).toBeTruthy();
  const original = document.querySelector('.detail');
  await user.click(screen.getByRole('button', { name: '设置' }));
  expect(screen.getByRole('dialog').getAttribute('aria-modal')).toBe('true');
  expect(document.querySelector('.detail')).toBe(original);
  await user.click(screen.getByRole('button', { name: '返回总览' }));
  expect(screen.queryByRole('dialog')).toBeNull();
});
it('never enables client launch in real observation mode and does not show stale green success', async () => {
  backend.state!.demo = false;
  backend.state!.launchAllowed = false;
  backend.state!.canLaunch = false;
  backend.state!.requiresRecheck = true;
  render(<App />);
  await screen.findByRole('heading', { name: '启动前需要完整复检' });
  expect(document.querySelector('.hero.warning')).toBeTruthy();
  for (const name of ['启动 CLI', '启动 Desktop']) {
    expect(screen.getByRole('button', { name }).hasAttribute('disabled')).toBe(true);
  }
  expect(backend.invoke.mock.calls.some(([name]) => name === 'launch')).toBe(false);
});

it('discarding invalid settings clears its error and preserves the effective profile', async () => {
  const user = await ready();
  await user.click(screen.getByRole('button', { name: '设置' }));
  backend.rejectConfig = true;
  await user.click(screen.getByRole('button', { name: '应用并查看模拟检查' }));
  await within(screen.getByRole('dialog')).findByRole('alert');
  await user.click(screen.getByRole('button', { name: '返回总览' }));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(backend.state!.profile.proxyUrl).toBe('http://127.0.0.1:17890');
  expect(scans()).toBe(1);
});

it('shows refresh rather than enable when system notification permission is already granted', async () => {
  backend.state!.demo = false;
  backend.state!.notificationStatus = '系统通知已授权（横幅和声音由系统设置控制）';
  backend.state!.notificationAction = 'refresh';
  backend.invoke.mockImplementation(async (name: string) => {
    if (name === 'snapshot' || name === 'scan') return next();
    if (name === 'notification_status') return backend.state!.notificationStatus;
    if (name === 'refresh_notifications') return '正在刷新系统通知状态…';
    throw new Error(`unexpected command: ${name}`);
  });
  const user = userEvent.setup();
  render(<App />);
  await screen.findByText(/系统通知已授权/);
  expect(screen.queryByRole('button', { name: '开启通知' })).toBeNull();
  await user.click(screen.getByRole('button', { name: '刷新状态' }));
  expect(backend.invoke).toHaveBeenCalledWith('refresh_notifications', undefined);
  expect(backend.invoke.mock.calls.some(([name]) => name === 'enable_notifications')).toBe(false);
});

it('sends exactly one explicitly requested test notification without launch or diagnostic calls', async () => {
  backend.state!.demo = false;
  backend.state!.notificationAction = 'refresh';
  backend.invoke.mockImplementation(async (name: string) => {
    if (name === 'snapshot' || name === 'scan') return next();
    if (name === 'notification_status') return '系统通知已授权';
    if (name === 'test_notification') return '测试通知已交给系统，请以实际收到为准。';
    throw Error(`unexpected command ${name}`);
  });
  const user = userEvent.setup();
  render(<App />);
  const button = await screen.findByRole('button', { name: '发送测试通知' });
  const before = backend.invoke.mock.calls.length;
  await user.click(button);
  expect((await screen.findByRole('status')).textContent).toContain('测试通知已交给系统');
  expect(backend.invoke.mock.calls.slice(before).map(([name]) => name)).toEqual([
    'test_notification',
  ]);
});
function notificationDiagnostics() {
  return {
    appEnabled: true,
    serviceActive: true,
    demo: false,
    applicationName: 'ClaudeGuard',
    applicationId: 'fixture.only',
    system: {
      authorization: 'authorized' as const,
      alert: 'disabled' as const,
      sound: 'disabled' as const,
      notificationCenter: 'enabled' as const,
      lockScreen: 'unknown' as const,
      alertStyle: 'none' as const,
      queriedAt: 1000,
    },
    lastTest: {
      sequence: 7,
      requestedAt: 1000,
      phase: 'accepted' as const,
      receipt: 'unconfirmed' as const,
      inNotificationCenter: true,
    },
    suggestions: ['系统允许通知，但横幅显示已关闭；请在系统设置中选择横幅。'],
  };
}
it('notification diagnostics separates system acceptance, center record and human receipt without sending', async () => {
  backend.state!.demo = false;
  backend.state!.notificationDiagnostics = notificationDiagnostics();
  const invoke = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(async (name: string, args?: Record<string, unknown>) => {
    if (name === 'confirm_test_notification') {
      expect(args).toEqual({ sequence: 7, received: false });
      backend.state!.notificationDiagnostics!.lastTest!.receipt = 'not_seen';
      return next();
    }
    return invoke(name, args);
  });
  const user = userEvent.setup();
  render(<App />);
  await user.click(await screen.findByRole('button', { name: '通知诊断' }));
  const dialog = screen.getByRole('dialog');
  expect(within(dialog).getByText('系统已接收，尚不能证明横幅已显示')).toBeTruthy();
  expect(within(dialog).getByText('已找到这条测试记录')).toBeTruthy();
  expect(within(dialog).getByText('等待你确认')).toBeTruthy();
  expect(within(dialog).getByText(/横幅显示已关闭/)).toBeTruthy();
  expect(backend.invoke.mock.calls.some(([name]) => name === 'test_notification')).toBe(false);
  await user.click(within(dialog).getByRole('button', { name: '没看到' }));
  await within(dialog).findByText('你已标记没看到');
  expect(
    backend.invoke.mock.calls.some(
      ([name]) =>
        name === 'launch' || name === 'test_notification' || name === 'enable_notifications',
    ),
  ).toBe(false);
});
it('shows recovery observation and concrete guidance without green or launch permission', async () => {
  backend.state!.demo = false;
  backend.state!.recovering = true;
  backend.state!.issue = '出口 IP 发生变化';
  backend.state!.requiresRecheck = true;
  backend.state!.issueGuidance = {
    title: '代理出口发生变化',
    explanation: '采样出口与稳定观察不同',
    steps: ['核对代理节点', '启动前仍需完整复检'],
  };
  const user = userEvent.setup();
  render(<App />);
  await screen.findByText('代理采样恢复观察中 · 等待连续稳定确认');
  expect(document.querySelector('.hero.good')).toBeNull();
  await user.click(screen.getByRole('button', { name: '查看处理建议' }));
  expect(within(screen.getByRole('dialog')).getByText('启动前仍需完整复检')).toBeTruthy();
  expect(backend.invoke.mock.calls.some(([name]) => name === 'launch')).toBe(false);
});

it('first run discovers locally but never scans, saves or launches before confirmation', async () => {
  Object.assign(backend.state!, {
    demo: false,
    setupRequired: true,
    checks: [],
    monitorActive: false,
    canLaunch: true,
    launchAllowed: true,
    profile: {
      schemaVersion: 1,
      proxyUrl: '',
      expectedProxyExecutable: '',
      expectedCountry: null,
      nonMainlandTimezone: false,
    },
  });
  const original = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(async (name: string, args?: Record<string, unknown>) => {
    if (name === 'discover_proxy')
      return {
        supported: true,
        message: '本地候选，等待确认；尚未发起外网请求。',
        candidates: [
          {
            profile: {
              ...backend.state!.profile,
              proxyUrl: 'http://127.0.0.1:8899',
              expectedProxyExecutable: '/fixture/another-proxy',
            },
            source: '系统代理与监听进程',
          },
        ],
      };
    if (name === 'configure') backend.state!.setupRequired = false;
    return original(name, args);
  });
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('heading', { name: '首次使用 · 确认本机代理' });
  await waitFor(() =>
    expect(
      (screen.getByRole('textbox', { name: /本机 HTTP 代理地址/ }) as HTMLInputElement).value,
    ).toBe('http://127.0.0.1:8899'),
  );
  expect(scans()).toBe(0);
  expect(
    backend.invoke.mock.calls.some(([name]) => name === 'configure' || name === 'launch'),
  ).toBe(false);
  expect(screen.getByRole('button', { name: '启动 Desktop' }).hasAttribute('disabled')).toBe(true);
  await user.click(screen.getByRole('button', { name: '返回总览' }));
  expect(scans()).toBe(0);
  expect(backend.state!.profile.proxyUrl).toBe('');
  await user.click(screen.getByRole('button', { name: '设置' }));
  const submit = await screen.findByRole('button', { name: '确认代理并开始检测' });
  await waitFor(() => expect(submit.hasAttribute('disabled')).toBe(false));
  await user.click(submit);
  await waitFor(() => expect(scans()).toBe(1));
  expect(backend.state!.profile.proxyUrl).toBe('http://127.0.0.1:8899');
  expect(backend.state!.profile.expectedCountry).toBeNull();
  expect(backend.state!.profile.nonMainlandTimezone).toBe(false);
  expect(backend.invoke.mock.calls.some(([name]) => name === 'launch')).toBe(false);
});
it('existing profile remains unchanged until explicit rediscovery and saving', async () => {
  backend.state!.profile.expectedCountry = 'SG';
  const original = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(async (name: string, args?: Record<string, unknown>) =>
    name === 'discover_proxy'
      ? {
          supported: true,
          message: '仅读本地',
          candidates: [
            {
              profile: {
                ...backend.state!.profile,
                proxyUrl: 'http://127.0.0.1:8899',
                expectedProxyExecutable: '/fixture/new-proxy',
                expectedCountry: null,
                nonMainlandTimezone: false,
              },
              source: '系统代理',
            },
          ],
        }
      : original(name, args),
  );
  const user = await ready();
  await user.click(screen.getByRole('button', { name: '设置' }));
  expect(backend.invoke.mock.calls.some(([name]) => name === 'discover_proxy')).toBe(false);
  await user.click(screen.getByRole('button', { name: '重新识别' }));
  await waitFor(() =>
    expect(
      (screen.getByRole('textbox', { name: /本机 HTTP 代理地址/ }) as HTMLInputElement).value,
    ).toBe('http://127.0.0.1:8899'),
  );
  expect((screen.getByRole('textbox', { name: /出口地区偏好/ }) as HTMLInputElement).value).toBe(
    'SG',
  );
  expect((screen.getByRole('checkbox') as HTMLInputElement).checked).toBe(true);
  expect(backend.state!.profile.proxyUrl).toBe('http://127.0.0.1:17890');
  await user.click(screen.getByRole('button', { name: '返回总览' }));
  expect(backend.state!.profile.proxyUrl).toBe('http://127.0.0.1:17890');
});
it('first-run PAC-only discovery offers manual entry without inventing a proxy', async () => {
  Object.assign(backend.state!, {
    demo: false,
    setupRequired: true,
    checks: [],
    monitorActive: false,
    profile: {
      schemaVersion: 1,
      proxyUrl: '',
      expectedProxyExecutable: '',
      expectedCountry: null,
      nonMainlandTimezone: false,
    },
  });
  const original = backend.invoke.getMockImplementation()!;
  backend.invoke.mockImplementation(async (name: string, args?: Record<string, unknown>) =>
    name === 'discover_proxy'
      ? { supported: true, candidates: [], message: '只有 PAC，请人工填写明确的本机代理' }
      : original(name, args),
  );
  render(<App />);
  await screen.findByText('只有 PAC，请人工填写明确的本机代理');
  expect(
    (screen.getByRole('textbox', { name: /本机 HTTP 代理地址/ }) as HTMLInputElement).value,
  ).toBe('');
  expect(scans()).toBe(0);
});

it('allows proxy preflight with an explicit nonblocking IPv6 coverage warning', async () => {
  backend.state!.checks = backend.state!.checks.map((c) =>
    c.id === 'ipv6'
      ? {
          ...c,
          status: 'warning',
          detail:
            '公网 IPv6 路径：活动接口存在可用地址。\n代理覆盖：尚未验证；代理预检仅提醒，不要求关闭 IPv6。未证实直连或泄漏。',
        }
      : c,
  );
  backend.state!.checkGuidance = [
    {
      id: 'ipv6',
      advice: {
        title: 'IPv6 覆盖尚未确认',
        explanation: '公网 IPv6 是正常网络能力，当前证据不能证明代理覆盖。',
        steps: ['核对代理的 IPv6 支持；本工具不会修改系统配置。', '处理后重新检测。'],
      },
    },
  ];
  backend.state!.demo = false;
  backend.state!.platform = 'recorded-ui-fixture';
  backend.state!.launchAllowed = true;
  backend.state!.requiresRecheck = false;
  backend.state!.canLaunch = true;
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('button', { name: /IPv6 本地配置/ });
  await user.click(screen.getByRole('button', { name: /IPv6 本地配置/ }));
  const detail = within(document.querySelector('.detail')!);
  expect(detail.getByText('覆盖提醒')).toBeTruthy();
  expect(detail.getByText('覆盖未验证')).toBeTruthy();
  expect(detail.getByText(/未证实直连或泄漏/)).toBeTruthy();
  expect(detail.getByText(/处理后重新检测/)).toBeTruthy();
  expect(detail.getAllByText(/不要求关闭 IPv6/).length).toBeGreaterThan(0);
  expect(screen.getByRole('heading', { name: '代理预检通过，可手动启动' })).toBeTruthy();
  expect(screen.getByText(/IPv4 \/ IPv6 实际客户端流量覆盖未验证/)).toBeTruthy();
  for (const name of ['启动 CLI', '启动 Desktop']) {
    const button = screen.getByRole('button', { name });
    expect(button.hasAttribute('disabled')).toBe(false);
  }
  expect(backend.invoke.mock.calls.some(([name]) => name === 'launch')).toBe(false);
});

it('focuses the actual blocking check ahead of an IPv6 advisory and refuses launch', async () => {
  backend.state!.demo = false;
  backend.state!.launchAllowed = true;
  backend.state!.requiresRecheck = true;
  backend.state!.canLaunch = true; // Blocking evidence still disables the UI, even in an inconsistent snapshot.
  backend.state!.checks = backend.state!.checks.map((c) =>
    c.id === 'ipv6'
      ? { ...c, status: 'warning', blocksLaunch: false, detail: 'IPv6 覆盖未验证，仅提醒' }
      : c.id === 'final_recheck'
        ? { ...c, status: 'fail', blocksLaunch: true, detail: '代理配置在最终复核时改变' }
        : c,
  );
  render(<App />);
  await screen.findByText('代理配置在最终复核时改变');
  expect(document.querySelector('.rule-row.selected')?.getAttribute('data-rule')).toBe(
    'final_recheck',
  );
  for (const name of ['启动 CLI', '启动 Desktop'])
    expect(screen.getByRole('button', { name }).hasAttribute('disabled')).toBe(true);
  expect(backend.invoke.mock.calls.some(([name]) => name === 'launch')).toBe(false);
});
