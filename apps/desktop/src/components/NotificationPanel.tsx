import type { NotificationDiagnostics, NotificationSetting } from '../lib/types';
const setting: Record<NotificationSetting, string> = {
  enabled: '开启',
  disabled: '关闭',
  unknown: '尚未确认',
  unsupported: '系统不支持',
};
const style = { unknown: '尚未确认', none: '无', banner: '横幅', alert: '提醒（持续显示）' };
const authorization = {
  unknown: '尚未确认',
  not_determined: '尚未授权',
  denied: '已拒绝',
  authorized: '已授权',
  provisional: '临时授权',
};
const phase = {
  pending: '正在读取权限',
  submitted: '已提交，等待系统回应',
  accepted: '系统已接收，尚不能证明横幅已显示',
  rejected: '未提交成功或系统拒绝',
  cancelled: '提交前已取消',
  unconfirmed: '提交后未及时确认，不会自动重发',
};
const receipt = { unconfirmed: '等待你确认', received: '你已确认收到', not_seen: '你已标记没看到' };
const time = (at: number | null) =>
  at == null ? '尚未读取' : new Date(at).toLocaleTimeString('zh-CN', { hour12: false });
export function NotificationPanel({
  diagnostics: d,
  sending,
  onRefresh,
  onTest,
  onConfirm,
}: {
  diagnostics: NotificationDiagnostics | null | undefined;
  sending: boolean;
  onRefresh: () => void;
  onTest: () => void;
  onConfirm: (sequence: number, received: boolean) => void;
}) {
  const test = d?.lastTest;
  const confirmable = test != null && ['accepted', 'unconfirmed'].includes(test.phase);
  return (
    <section className="notification-panel">
      <div className="section-heading">
        <div>
          <h2>通知诊断</h2>
          <p className="muted">先核对系统设置，再区分提交结果与实际收到。</p>
        </div>
        <button onClick={onRefresh}>刷新诊断</button>
      </div>
      {!d ? (
        <p>尚未读到通知配置，请刷新诊断。未知状态不会显示为已开启。</p>
      ) : (
        <>
          <p className="notification-app">
            当前应用：<strong>{d.applicationName}</strong>
            <small>最近读取：{time(d.system.queriedAt)}</small>
          </p>
          <dl className="notification-settings">
            <div>
              <dt>系统授权</dt>
              <dd>{authorization[d.system.authorization]}</dd>
            </div>
            <div>
              <dt>本工具后台提醒</dt>
              <dd>{d.appEnabled ? '已启用' : '未启用'}</dd>
            </div>
            <div>
              <dt>横幅 / 提醒开关</dt>
              <dd>{setting[d.system.alert]}</dd>
            </div>
            <div>
              <dt>提醒样式</dt>
              <dd>{style[d.system.alertStyle]}</dd>
            </div>
            <div>
              <dt>通知中心显示</dt>
              <dd>{setting[d.system.notificationCenter]}</dd>
            </div>
            <div>
              <dt>声音</dt>
              <dd>{setting[d.system.sound]}</dd>
            </div>
            <div>
              <dt>锁屏显示</dt>
              <dd>{setting[d.system.lockScreen]}</dd>
            </div>
            <div>
              <dt>专注模式状态</dt>
              <dd>未读取，请在系统中核对</dd>
            </div>
          </dl>
          <section className="notification-test" aria-label="本地测试通知">
            <div className="section-heading">
              <h2>本地测试通知</h2>
              <button disabled={sending || d.demo || !d.serviceActive} onClick={onTest}>
                {sending ? '正在发送…' : '发送一条测试通知'}
              </button>
            </div>
            {!test ? (
              <p className="muted">
                本次运行尚未发送测试通知。记录仅保存在内存，只有手动点击才会发送。
              </p>
            ) : (
              <>
                <dl className="notification-evidence">
                  <div>
                    <dt>本次测试时间</dt>
                    <dd>{time(test.requestedAt)}</dd>
                  </div>
                  <div>
                    <dt>系统接收结果</dt>
                    <dd>{phase[test.phase]}</dd>
                  </div>
                  <div>
                    <dt>通知中心记录</dt>
                    <dd>
                      {test.inNotificationCenter === true
                        ? '已找到这条测试记录'
                        : test.inNotificationCenter === false
                          ? '本次查询未找到（不等同于发送失败）'
                          : '尚未查询或不适用'}
                    </dd>
                  </div>
                  {test.centerQueriedAt != null && (
                    <div>
                      <dt>通知中心查询时间</dt>
                      <dd>{time(test.centerQueriedAt)}</dd>
                    </div>
                  )}
                  <div>
                    <dt>实际收到</dt>
                    <dd>{receipt[test.receipt]}</dd>
                  </div>
                </dl>
                <div className="receipt-actions">
                  <span>你看到了吗？</span>
                  <button
                    disabled={!confirmable || d.demo}
                    onClick={() => onConfirm(test.sequence, true)}
                  >
                    我收到了
                  </button>
                  <button
                    disabled={!confirmable || d.demo}
                    onClick={() => onConfirm(test.sequence, false)}
                  >
                    没看到
                  </button>
                </div>
                <small className="muted">
                  通知中心记录不代表横幅一定出现；收到结果仅由你手动确认，不会改变网络检查或启动权限。
                </small>
              </>
            )}
          </section>
          <section className="guidance-card">
            <strong>建议处理</strong>
            <ul>
              {d.suggestions.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          </section>
          <details className="notification-identity">
            <summary>应用信息（用于核对通知设置）</summary>
            <code>{d.applicationId}</code>
            <p>此页面只读取本工具的通知设置与测试记录，不修改系统设置。</p>
          </details>
        </>
      )}
    </section>
  );
}
