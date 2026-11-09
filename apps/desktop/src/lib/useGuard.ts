import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { newerSnapshot } from './snapshots';
import type { Discovery, Profile, Snapshot } from './types';
export function useGuard() {
  const [state, setState] = useState<Snapshot | null>(null);
  const accept = (next: Snapshot) => setState((previous) => newerSnapshot(previous, next));
  const [error, setError] = useState('');
  const [feedback, setFeedback] = useState('');
  const [pending, setPending] = useState<'scan' | 'configure' | 'launch' | null>(null);
  useEffect(() => {
    if (!feedback) return;
    const timeout = setTimeout(() => setFeedback(''), 4000);
    return () => clearTimeout(timeout);
  }, [feedback]);
  const inFlight = useRef(false);
  const initialized = useRef(false);
  useEffect(() => {
    let alive = true;
    let dispose: (() => void) | undefined;
    let disposeAction: (() => void) | undefined;
    const start = async () => {
      try {
        dispose = await listen<Snapshot>('guard://state', (event) => {
          if (alive) accept(event.payload);
        });
        if (!alive) {
          dispose();
          return;
        }
        disposeAction = await listen<{ kind: string; message: string }>(
          'guard://action',
          (event) => {
            if (!alive) return;
            if (event.payload.kind === 'error') {
              setError(event.payload.message);
              setFeedback('');
            } else {
              setError('');
              setFeedback(event.payload.message);
            }
          },
        );
        if (!alive) {
          disposeAction();
          return;
        }
        const initial = await invoke<Snapshot>('snapshot');
        if (!alive) return;
        accept(initial);
        if (!initialized.current) {
          initialized.current = true;
          if (!initial.setupRequired) {
            setPending('scan');
            const checked = await invoke<Snapshot>('scan');
            if (alive) accept(checked);
          }
        }
      } catch (error) {
        if (alive)
          setError(
            typeof error === 'string'
              ? error
              : '未连接桌面后端。请通过 Tauri 桌面应用运行；浏览器预览不会产生检测结果。',
          );
      } finally {
        if (alive) setPending(null);
      }
    };
    void start();
    return () => {
      alive = false;
      dispose?.();
      disposeAction?.();
    };
  }, []);
  async function action<T>(
    name: string,
    args?: Record<string, unknown>,
    operation: typeof pending = null,
  ) {
    if (inFlight.current) {
      setError('正在处理上一项操作，请稍候');
      return undefined;
    }
    inFlight.current = true;
    setPending(operation);
    setError('');
    setFeedback('');
    try {
      return await invoke<T>(name, args);
    } catch (error) {
      setError(typeof error === 'string' ? error : '操作未完成，请重试');
      return undefined;
    } finally {
      inFlight.current = false;
      setPending(null);
    }
  }
  return {
    state,
    error,
    feedback,
    pending,
    dismissError: () => setError(''),
    notify: (message: string) => {
      setError('');
      setFeedback(message);
    },
    fail: (message: string) => {
      setFeedback('');
      setError(message);
    },
    scan: async (prefix = '') => {
      const next = await action<Snapshot>('scan', undefined, 'scan');
      if (next) {
        accept(next);
        setFeedback(
          `${prefix}${next.demo ? '离线检测' : '检测'}已完成 · ${next.checks.length} 项 · ${new Date().toLocaleTimeString('zh-CN', { hour12: false })}`,
        );
      }
    },
    configure: async (profile: Profile) => {
      const next = await action<Snapshot>('configure', { profile }, 'configure');
      if (next) {
        accept(next);
        return true;
      }
      return false;
    },
    launch: async (target: 'cli' | 'desktop') => {
      const next = await action<Snapshot>('launch', { target }, 'launch');
      if (next) accept(next);
    },
    // Window visibility is independent of scans/launch preparation. Hiding must
    // remain responsive and must not reset their pending state or cancel them.
    hide: async () => {
      try {
        await invoke('hide_main');
      } catch {
        setError('收起窗口失败，请重试或使用窗口关闭按钮。');
      }
    },
    discoverProxy: () => invoke<Discovery>('discover_proxy'),
    enableNotifications: () => action<string>('enable_notifications'),
    refreshNotifications: () => action<string>('refresh_notifications'),
    testNotification: () => action<string>('test_notification'),
    confirmTestNotification: async (sequence: number, received: boolean) => {
      const next = await action<Snapshot>('confirm_test_notification', { sequence, received });
      if (next) accept(next);
    },
    notificationStatus: () =>
      invoke<string>('notification_status').catch(() => '暂时无法读取通知状态'),
  };
}
