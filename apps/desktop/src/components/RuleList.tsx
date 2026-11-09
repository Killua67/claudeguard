import { useEffect, useRef } from 'react';
import type { Check, RuleId } from '../lib/types';
import { rules, checkStatusText } from '../lib/types';
export function RuleList({
  checks,
  running,
  selected,
  onSelect,
}: {
  checks: Check[];
  running: boolean;
  selected: RuleId;
  onSelect: (id: RuleId) => void;
}) {
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const row = list.current?.querySelector(`[data-rule="${selected}"]`);
    if (row instanceof HTMLElement && typeof row.scrollIntoView === 'function')
      row.scrollIntoView({ block: 'nearest' });
  }, [selected]);
  const next = rules.find((r) => !checks.some((c) => c.id === r.id))?.id;
  return (
    <div className="rule-list" ref={list}>
      {rules.map((rule, index) => {
        const result = checks.find((c) => c.id === rule.id);
        const active = running && next === rule.id;
        const status = active ? 'running' : (result?.status ?? 'pending');
        return (
          <button
            key={rule.id}
            data-rule={rule.id}
            className={`rule-row ${selected === rule.id ? 'selected' : ''}`}
            onClick={() => onSelect(rule.id)}
          >
            <span className={`rule-symbol ${status}`}>
              {active ? (
                <span className="spinner small" />
              ) : status === 'pass' ? (
                '✓'
              ) : status === 'fail' ? (
                '×'
              ) : status === 'warning' ? (
                '!'
              ) : status === 'info' ? (
                'i'
              ) : (
                String(index + 1).padStart(2, '0')
              )}
            </span>
            <span className="rule-copy">
              <strong>{rule.title}</strong>
              <small>{rule.note}</small>
            </span>
            <span className={`rule-status ${status}`}>
              {active ? '检测中' : result ? checkStatusText(result) : '待检查'}
            </span>
            <span className="chevron">›</span>
          </button>
        );
      })}
    </div>
  );
}
