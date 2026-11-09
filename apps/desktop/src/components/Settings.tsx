import { useEffect, useState } from 'react';
import type { Discovery, Profile } from '../lib/types';
export function Settings({
  profile,
  busy,
  onSave,
  demo,
  firstRun = false,
  onDiscover,
}: {
  profile: Profile;
  busy: boolean;
  onSave: (p: Profile) => Promise<boolean>;
  demo: boolean;
  firstRun?: boolean;
  onDiscover: () => Promise<Discovery>;
}) {
  const [draft, setDraft] = useState(profile);
  const [discovery, setDiscovery] = useState<Discovery | null>(null);
  const [discovering, setDiscovering] = useState(firstRun);
  const [discoveryError, setDiscoveryError] = useState('');
  async function discover() {
    setDiscovering(true);
    setDiscoveryError('');
    try {
      const found = await onDiscover();
      setDiscovery(found);
      if (found.candidates.length === 1) {
        const candidate = found.candidates[0].profile;
        setDraft((previous) => ({
          ...previous,
          proxyUrl: candidate.proxyUrl,
          expectedProxyExecutable: candidate.expectedProxyExecutable,
        }));
      }
    } catch {
      setDiscoveryError('自动识别未完成，请重试或人工填写；未发起外网请求。');
    } finally {
      setDiscovering(false);
    }
  }
  useEffect(() => {
    if (!firstRun) return;
    let alive = true;
    void onDiscover()
      .then((found) => {
        if (!alive) return;
        setDiscovery(found);
        if (found.candidates.length === 1) {
          const candidate = found.candidates[0].profile;
          setDraft((previous) => ({
            ...previous,
            proxyUrl: candidate.proxyUrl,
            expectedProxyExecutable: candidate.expectedProxyExecutable,
          }));
        }
      })
      .catch(() => {
        if (alive) setDiscoveryError('自动识别未完成，请重试或人工填写；未发起外网请求。');
      })
      .finally(() => {
        if (alive) setDiscovering(false);
      });
    return () => {
      alive = false;
    };
  }, []);
  const [saved, setSaved] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (busy || submitting || discovering) return;
    setSubmitting(true);
    setSaved(false);
    try {
      setSaved(await onSave(draft));
    } finally {
      setSubmitting(false);
    }
  }
  return (
    <form className="settings-panel" onSubmit={(event) => void submit(event)}>
      <div className="section-heading">
        <div>
          <small>连接偏好</small>
          <h2>{firstRun ? '首次使用 · 确认本机代理' : '使用环境配置'}</h2>
        </div>
        <span className="chip neutral">{demo ? '仅本次演示有效' : '仅保存到本工具'}</span>
      </div>
      <p className="muted">
        {demo
          ? '这是离线演示。修改后会回到总览，展示一轮模拟检查，不改动你的真实配置。'
          : '选择你正在使用的本机代理。保存后会回到总览并重新检查，不改动系统代理。'}
      </p>
      <section className="discovery-card" aria-label="代理自动识别">
        <strong>
          {discovering ? '正在读取本机代理…' : firstRun ? '识别结果等待你确认' : '重新识别本机代理'}
        </strong>
        <p>
          {discoveryError ||
            discovery?.message ||
            '仅读取系统代理与对应监听进程；不扫描端口、不读取订阅、不请求外网。'}
        </p>
        {discovery?.candidates.map((candidate) => (
          <p key={candidate.profile.proxyUrl} className="muted">
            {candidate.source}
          </p>
        ))}
        <button
          type="button"
          disabled={busy || submitting || discovering}
          onClick={() => void discover()}
        >
          {discovering ? '识别中…' : '重新识别'}
        </button>
        <small>
          候选仅供核对；请确认这是你信任的代理程序。首次确认前不开始检测；重新识别仅修改草稿，保存后才替换已有配置。
        </small>
      </section>
      <fieldset disabled={busy || submitting || discovering} className="settings-card">
        <legend>代理连接</legend>
        <label>
          本机 HTTP 代理地址
          <input
            value={draft.proxyUrl}
            onChange={(e) => setDraft({ ...draft, proxyUrl: e.target.value })}
            spellCheck={false}
          />
          <small>当前支持 127.0.0.1 上的明确端口；不支持在地址中存储认证信息。</small>
        </label>
        <label>
          预期代理程序路径
          <input
            value={draft.expectedProxyExecutable}
            onChange={(e) => setDraft({ ...draft, expectedProxyExecutable: e.target.value })}
            spellCheck={false}
          />
          <small>用于核对监听进程，配置路径不会被当作命令执行。</small>
        </label>
      </fieldset>
      <fieldset disabled={busy || submitting} className="settings-card">
        <legend>检查偏好</legend>
        <div className="settings-grid">
          <label>
            出口地区偏好
            <input
              maxLength={2}
              placeholder="不固定"
              value={draft.expectedCountry ?? ''}
              onChange={(e) =>
                setDraft({ ...draft, expectedCountry: e.target.value.toUpperCase() || null })
              }
            />
            <small>两位地区代码。它是个人偏好，不代表账号或地区使用资格。</small>
          </label>
          <label className="checkbox">
            <input
              type="checkbox"
              checked={draft.nonMainlandTimezone}
              onChange={(e) => setDraft({ ...draft, nonMainlandTimezone: e.target.checked })}
            />
            要求非中国大陆时区（可选偏好）
          </label>
        </div>
      </fieldset>
      <p className="muted">
        不读取 Claude 账号、对话或配置文件。出口地区和时区是个人偏好，不代表账号使用资格。
      </p>
      <button className="primary" type="submit" disabled={busy || submitting || discovering}>
        {submitting
          ? '正在应用配置…'
          : demo
            ? '应用并查看模拟检查'
            : firstRun
              ? '确认代理并开始检测'
              : '保存并重新检查'}
      </button>
      {saved && <span className="saved">配置已提交，结果以页面提示为准</span>}
    </form>
  );
}
