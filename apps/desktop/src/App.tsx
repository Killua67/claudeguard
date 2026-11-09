import { useEffect, useState } from 'react';
import { saveProfileAndCheck } from './lib/actions';
import { useGuard } from './lib/useGuard';
import { rules, statusText, checkStatusText, type RuleId } from './lib/types';
import { reportText, sampleTime, eventTime } from './lib/report';
import { RuleList } from './components/RuleList';
import { DependencyCatalog } from './components/DependencyCatalog';
import { Settings } from './components/Settings';
import { LaunchHelp } from './components/LaunchHelp';
import { Sheet } from './components/Sheet';
import { Icon } from './components/Icon';
import { NotificationPanel } from './components/NotificationPanel';
import { GuidanceCard } from './components/GuidanceCard';
export function App() {
  const guard = useGuard();
  const { state, error } = guard;
  const [sheet, setSheet] = useState<'events' | 'catalog' | 'settings' | 'notifications' | null>(
    null,
  );
  const [selected, setSelected] = useState<RuleId>('exit_region');
  const [notification, setNotification] = useState('正在读取系统通知权限…');
  const [copied, setCopied] = useState(false);
  const [sendingTest, setSendingTest] = useState(false);
  const [help, setHelp] = useState<'cli' | 'desktop' | null>(null);
  const [finishedAt, setFinishedAt] = useState<Date | null>(null);
  useEffect(() => {
    void guard.notificationStatus().then((s) => {
      if (s) setNotification(s);
    });
  }, []);
  useEffect(() => {
    if (state?.checks.length === 15 && !state.scanning) setFinishedAt(new Date());
  }, [state?.checks.length, state?.scanning]);
  useEffect(() => {
    if (!state) return;
    if (state.scanning) setSelected(rules[Math.min(state.checks.length, rules.length - 1)].id);
    else if (state.checks.length)
      setSelected(
        state.checks.find((check) => check.blocksLaunch !== false)?.id ??
          state.checks.find((check) => check.status === 'warning')?.id ??
          'exit_region',
      );
  }, [state?.scanning, state?.checks.length]);
  useEffect(() => {
    if (state?.setupRequired) setSheet('settings');
  }, [state?.setupRequired]);
  if (!state)
    return (
      <main className="loading">
        <span className="spinner" />
        <h1>ClaudeGuard</h1>
        <p>{error || '正在连接本地检测核心…'}</p>
        <small>不会自动启动 Claude，不请求 Anthropic。</small>
      </main>
    );
  const busy = state.scanning || state.launching || guard.pending !== null;
  const completed = state.checks.length;
  const blocked = state.checks.some((c) => c.blocksLaunch !== false);
  const failureCount = state.checks.filter((c) => c.status === 'fail').length;
  const selectedRunning =
    state.scanning && rules[Math.min(completed, rules.length - 1)].id === selected;
  const warningCount = state.checks.filter(
    (c) => c.status === 'warning' && c.blocksLaunch !== false,
  ).length;
  const item = state.checks.find((c) => c.id === selected);
  const definition = rules.find((r) => r.id === selected)!;
  const tone = state.setupRequired
    ? 'neutral'
    : busy
      ? 'checking'
      : failureCount
        ? 'danger'
        : state.issue || blocked || (completed > 0 && state.requiresRecheck)
          ? 'warning'
          : completed === 15
            ? 'good'
            : 'neutral';
  const title = state.setupRequired
    ? '首次使用，请确认本机代理'
    : busy
      ? '正在逐项检查环境'
      : state.demo
        ? '离线演示已完成'
        : failureCount
          ? `检测完成，发现 ${failureCount} 项异常`
          : warningCount
            ? `检测完成，${warningCount} 项提醒阻止启动`
            : blocked
              ? '代理预检尚未通过'
              : state.issue
                ? '监测结果已失效，请重新检测'
                : state.requiresRecheck
                  ? '启动前需要完整复检'
                  : state.launchAllowed
                    ? '代理预检通过，可手动启动'
                    : '代理预检通过';
  const monitorGood = state.monitorActive && !state.issue && !state.recovering;
  const checkGuidance = state.checkGuidance?.find((entry) => entry.id === selected)?.advice;
  async function sendTest() {
    if (sendingTest) return;
    setSendingTest(true);
    try {
      const message = await guard.testNotification();
      if (message) guard.notify(message);
    } finally {
      setSendingTest(false);
    }
  }
  const latest = state.events.at(0);
  async function copy() {
    try {
      await navigator.clipboard.writeText(reportText(state!));
      setCopied(true);
      guard.notify('脱敏报告已复制到剪贴板');
      setTimeout(() => setCopied(false), 2500);
    } catch {
      setCopied(false);
      guard.fail('复制报告失败，请确认系统允许剪贴板访问后重试。');
    }
  }
  return (
    <main className="shell">
      <header className={`hero ${tone}`}>
        <div className="hero-main">
          <div
            className="hero-status"
            aria-label={tone === 'good' ? '代理预检通过' : '本机检查尚未放行'}
          >
            {busy ? (
              <span className="spinner" />
            ) : tone === 'good' ? (
              <Icon name="check" />
            ) : tone === 'warning' || tone === 'danger' ? (
              <span>!</span>
            ) : (
              <span>○</span>
            )}
          </div>
          <div className="hero-copy">
            <small>CLAUDEGUARD / 代理预检</small>
            <h1>{title}</h1>
            <p>
              {state.demo
                ? '离线模拟数据；不会联网或执行客户端。'
                : 'IPv4 / IPv6 实际客户端流量覆盖未验证；预检不提供防直连保护。'}
            </p>
            <p className="scope-note">代理预检模式 · 仅检查与通知 · 不主动断网</p>
          </div>
          <div className="launch-actions">
            {state.setupRequired && (
              <button className="primary" onClick={() => setSheet('settings')}>
                识别并确认代理
              </button>
            )}
            <div>
              <button
                className="launch-button"
                disabled={
                  state.setupRequired ||
                  busy ||
                  (!state.demo && blocked) ||
                  (!state.demo && (!state.launchAllowed || !state.canLaunch || !state.clients.cli))
                }
                onClick={() => (state.demo ? setHelp('cli') : void guard.launch('cli'))}
              >
                <Icon name="cli" />
                {state.demo ? 'CLI 启动说明' : '启动 CLI'}
              </button>
              <button
                className="launch-button"
                disabled={
                  state.setupRequired ||
                  busy ||
                  (!state.demo && blocked) ||
                  (!state.demo &&
                    (!state.launchAllowed || !state.canLaunch || !state.clients.desktop))
                }
                onClick={() => (state.demo ? setHelp('desktop') : void guard.launch('desktop'))}
              >
                <Icon name="desktop" />
                {state.demo ? 'Desktop 启动说明' : '启动 Desktop'}
              </button>
            </div>
            <small>
              {state.demo
                ? '离线演示 · 点击仅查看说明'
                : !state.launchAllowed
                  ? '本次仅检测 · 后端禁止启动客户端'
                  : !state.clients.cli
                    ? 'CLI 未安装 · 不会自动安装'
                    : '仅手动点击启动 · 点击后再次复检'}
            </small>
          </div>
        </div>
        <div className="summary-line">
          <div className="counts">
            {(
              [
                'pass',
                'warning',
                'fail',
                'info',
                ...(state.checks.some((c) => c.status === 'skipped') ? (['skipped'] as const) : []),
              ] as const
            ).map((status) => (
              <span key={status}>
                <i className={status} />
                {statusText[status]} <b>{state.checks.filter((c) => c.status === status).length}</b>
              </span>
            ))}
          </div>
          <time className="finished-at">
            {busy
              ? `正在执行第 ${Math.min(completed + 1, 15)} 项`
              : finishedAt?.toLocaleTimeString('zh-CN', {
                  hour: 'numeric',
                  minute: '2-digit',
                  hour12: true,
                })}
          </time>
        </div>
        <progress className="scan-progress" max={15} value={completed} aria-label="检测进度" />
      </header>
      <section className={`monitor-strip ${monitorGood ? 'healthy' : 'attention'}`}>
        <div className="monitor-heading">
          <Icon name="pulse" />
          <strong>
            {state.setupRequired
              ? '等待确认代理 · 尚未发起外网检测'
              : state.recovering
                ? '代理采样恢复观察中 · 等待连续稳定确认'
                : (state.issue ??
                  (state.monitorActive
                    ? state.requiresRecheck
                      ? '后台采样已恢复 · 启动前仍需完整复检'
                      : '后台监测中 · 本地每 5 秒 / 出口每 30 秒'
                    : '等待完整检测建立监测基线'))}
          </strong>
          <span className="sample">最近采样　{sampleTime(state)}</span>
        </div>
        <p>
          后台自动检测，异常后继续采样；同一异常只提醒一次。收起窗口后继续，退出工具后停止。不抓包、不主动断网。
        </p>
        <div className="notification-line">
          <span>
            {state.demo ? '离线演示不会发送系统通知' : (state.notificationStatus ?? notification)}
          </span>
          <button
            onClick={() =>
              state.demo
                ? guard.notify('演示模式不发送系统通知。正式模式开启后，同一异常只提醒一次。')
                : void (
                    state.notificationAction === 'refresh'
                      ? guard.refreshNotifications()
                      : guard.enableNotifications()
                  ).then((s) => {
                    if (s) setNotification(s);
                  })
            }
          >
            {state.demo
              ? '查看通知说明'
              : state.notificationAction === 'refresh'
                ? '刷新状态'
                : state.notificationAction === 'enable_reminders'
                  ? '启用提醒'
                  : '开启通知'}
          </button>
          {!state.demo && (
            <button
              disabled={sendingTest}
              onClick={() => {
                setSheet('notifications');
                void sendTest();
              }}
            >
              {sendingTest ? '正在发送…' : '发送测试通知'}
            </button>
          )}
          {!state.demo && <button onClick={() => setSheet('notifications')}>通知诊断</button>}
          {state.issue && <button onClick={() => setSheet('events')}>查看处理建议</button>}
        </div>
        <button className="history-line" aria-label="事件记录" onClick={() => setSheet('events')}>
          {latest
            ? `历史告警：${eventTime(state, latest.atMs)} ${latest.notice.kind === 'problem' ? latest.notice.message : '代理采样已恢复；启动放行以完整复检为准'}`
            : '历史告警：暂无异常记录'}
        </button>
      </section>
      {error && !sheet && (
        <div role="alert" className="error-banner">
          {error}
        </div>
      )}
      {guard.feedback && (
        <div role="status" className="feedback-banner">
          ✓　{guard.feedback}
        </div>
      )}
      {state.launchMessage && <p className="launch-message">{state.launchMessage}</p>}
      <section className="diagnostics">
        <div className="steps">
          <div className="section-heading">
            <h2>检测流程</h2>
            <small>{completed} / 15</small>
          </div>
          <RuleList
            checks={state.checks}
            running={busy}
            selected={selected}
            onSelect={setSelected}
          />
        </div>
        <article className="detail">
          <div className="detail-heading">
            <small>检测详情</small>
            <small>
              {selected === 'ipv6'
                ? '覆盖提醒'
                : ['dns', 'route', 'clock', 'existing_clients'].includes(selected)
                  ? '参考信息'
                  : '核心检查'}
            </small>
          </div>
          <h2>{definition.title}</h2>
          <div
            className={`detail-status ${selectedRunning ? 'running' : (item?.status ?? 'pending')}`}
          >
            <span className="detail-label">
              {selectedRunning ? (
                <span className="spinner small" />
              ) : (
                <span className="status-glyph">
                  {item?.status === 'pass'
                    ? '✓'
                    : item?.status === 'warning'
                      ? '!'
                      : item?.status === 'fail'
                        ? '×'
                        : 'i'}
                </span>
              )}
              {selectedRunning ? '正在读取与验证…' : item ? checkStatusText(item) : '等待检测'}
            </span>
            <span>{item ? `${(item.elapsedMs / 1000).toFixed(2)} 秒` : ''}</span>
          </div>
          <p>{item?.detail ?? '本项将按流程顺序执行，结果会实时显示在这里。'}</p>
          <div className="evidence">
            <strong>检查依据与范围</strong>
            <p>
              {selected === 'exit_region'
                ? '以本次 Cloudflare 请求返回的地区为依据。出口地区只是当前配置基线，不代表实际所在地或平台使用资格。'
                : selected === 'ipv6'
                  ? '本项只读观察 IPv6，不验证实际客户端流量覆盖；覆盖未知仅提醒，不阻止代理预检启动，不要求关闭 IPv6。代理、旁路、监听身份及 HTTPS 异常仍会阻止启动。'
                  : `${definition.note}。此检查不验证客户端全部流量，不判断账号状态或平台资格。`}
            </p>
          </div>
          {checkGuidance && <GuidanceCard guidance={checkGuidance} />}
          <small className="detail-footnote">
            <Icon name="info" />
            <span>结果仅代表本次采样，不能保证账号安全或未来网络状态。</span>
          </small>
        </article>
      </section>
      <footer>
        <div className="footer-notes">
          <span>
            <Icon name="hand" />
            仅手动启动 · 启动前复检 · 不修改系统配置
          </span>
          <small>监测只通过固定代理请求 Cloudflare；IP 仅在内存比较。无强制断网保护。</small>
        </div>
        <div className="footer-actions">
          <button
            className="settings-trigger"
            aria-label="设置"
            title="设置"
            onClick={() => setSheet('settings')}
          >
            <Icon name="settings" />
          </button>
          <button onClick={() => setSheet('catalog')}>Claude 域名清单</button>
          <button onClick={() => void copy()}>{copied ? '已复制' : '复制报告'}</button>
          <button disabled={busy || state.setupRequired} onClick={() => void guard.scan()}>
            {busy ? (
              '检测中…'
            ) : (
              <>
                <Icon name="refresh" />
                重新检测
              </>
            )}
          </button>
          <button onClick={() => void guard.hide()}>收起窗口</button>
        </div>
      </footer>
      {sheet && (
        <Sheet
          title={
            sheet === 'events'
              ? '事件记录'
              : sheet === 'catalog'
                ? 'Claude 域名清单'
                : sheet === 'notifications'
                  ? '通知诊断'
                  : '设置'
          }
          onClose={() => {
            setSheet(null);
            guard.dismissError();
          }}
        >
          {error && (
            <div role="alert" className="sheet-error">
              {error}
            </div>
          )}
          {sheet === 'notifications' && (
            <NotificationPanel
              diagnostics={state.notificationDiagnostics}
              sending={sendingTest}
              onRefresh={() => {
                void guard.refreshNotifications();
              }}
              onTest={() => {
                void sendTest();
              }}
              onConfirm={(sequence, received) => {
                void guard.confirmTestNotification(sequence, received);
              }}
            />
          )}
          {sheet === 'events' && (
            <section className="events-panel">
              {state.issueGuidance && <GuidanceCard guidance={state.issueGuidance} />}
              {state.recovering && (
                <p className="recovery-note">
                  正在确认连续稳定恢复；此时仍保留异常提醒，启动不会放行。
                </p>
              )}
              <div className="section-heading">
                <h2>异常与恢复记录</h2>
                <span className="chip neutral">仅保留最近 8 条 · 不落盘</span>
              </div>
              {state.events.length === 0 ? (
                <div className="empty">
                  暂无异常记录<p>正常时安静运行，有变化时在这里说明。</p>
                </div>
              ) : (
                state.events.map((event, i) => (
                  <article className="event-row" key={`${event.atMs}-${i}`}>
                    <span
                      className={event.notice.kind === 'problem' ? 'event-warning' : 'event-good'}
                    >
                      {event.notice.kind === 'problem' ? '!' : '✓'}
                    </span>
                    <div>
                      <time>{eventTime(state, event.atMs)}</time>
                      <strong>
                        {event.notice.kind === 'problem' ? event.notice.message : '代理采样已恢复'}
                      </strong>
                      <p>
                        {event.notice.kind === 'problem'
                          ? '建议检查代理；本工具不会中断网络。'
                          : '后台恢复不自动启动客户端，启动放行仍以完整复检为准。'}
                      </p>
                    </div>
                  </article>
                ))
              )}
            </section>
          )}
          {sheet === 'catalog' && <DependencyCatalog />}
          {sheet === 'settings' && (
            <Settings
              key={JSON.stringify(state.profile)}
              profile={state.profile}
              firstRun={state.setupRequired}
              onDiscover={guard.discoverProxy}
              demo={state.demo}
              busy={busy}
              onSave={(p) =>
                saveProfileAndCheck(p, guard.configure, async () => {
                  setSheet(null);
                  await guard.scan(state.demo ? '演示配置已应用，' : '配置已保存，');
                })
              }
            />
          )}
        </Sheet>
      )}
      {help && <LaunchHelp target={help} onClose={() => setHelp(null)} />}
    </main>
  );
}
