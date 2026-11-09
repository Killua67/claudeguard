import { useState } from 'react';
import catalog from '../../../../resources/network-dependencies.json';
export function DependencyCatalog() {
  const [profile, setProfile] = useState('combined');
  const scopes: Record<string, string> = {
    core: '基础连接',
    feature: '按功能使用',
    dynamic: '动态规则',
    broadAlternative: '宽泛备选',
  };
  const entries = catalog.entries.filter(
    (e) => profile === 'combined' || e.profiles.includes(profile),
  );
  return (
    <section className="catalog-panel">
      <div className="section-heading">
        <div>
          <small>OFFLINE REFERENCE · {catalog.version}</small>
          <h2>Claude 网络依赖清单</h2>
        </div>
        <span className="chip neutral">只展示 · 不探测</span>
      </div>
      <p className="muted">{catalog.limitations}</p>
      <div className="catalog-tabs">
        {[
          ['combined', '全部'],
          ['cli', 'CLI'],
          ['desktop', 'Desktop'],
        ].map(([id, label]) => (
          <button
            key={id}
            className={profile === id ? 'active' : ''}
            onClick={() => setProfile(id)}
          >
            {label}
          </button>
        ))}
        <small>{entries.length} 条</small>
      </div>
      <div className="catalog-list">
        {entries.map((e) => (
          <article key={e.host}>
            <code>{e.host}</code>
            <span>{scopes[e.scope]}</span>
            <p>{e.purpose}</p>
            <small>依据：{e.source}</small>
          </article>
        ))}
      </div>
    </section>
  );
}
