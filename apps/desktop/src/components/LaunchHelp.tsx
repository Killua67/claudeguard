import { useEffect, useRef } from 'react';
export function LaunchHelp({
  target,
  onClose,
}: {
  target: 'cli' | 'desktop';
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const element = dialog.current;
    const previous = document.activeElement as HTMLElement | null;
    element?.showModal();
    return () => {
      element?.close();
      previous?.focus();
    };
  }, []);
  const name = target === 'cli' ? 'CLI' : 'Desktop';
  return (
    <dialog
      ref={dialog}
      className="launch-help"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      aria-labelledby="launch-help-title"
    >
      <div className="dialog-top">
        <span className="eyebrow">离线说明</span>
        <button onClick={onClose} aria-label="关闭启动说明">
          ×
        </button>
      </div>
      <h2 id="launch-help-title">{name} 将如何启动</h2>
      <p>当前是离线演示。这些步骤仅用于说明，不会执行客户端，也不会请求 Anthropic。</p>
      <ol>
        <li>用户明确点击启动，才进入启动流程。</li>
        <li>再次完整检查代理、出口和本机环境。</li>
        <li>
          {target === 'cli'
            ? '选择工作目录，在终端中复检并传入本次代理配置。'
            : '验证应用签名及已有实例，传入本次代理配置。'}
        </li>
        <li>任一检查未通过，拒绝本次启动。</li>
      </ol>
      <div className="dialog-note">
        本次没有启动任何客户端。真实启动仍需在正式模式下由你手动操作。
      </div>
      <button className="primary" onClick={onClose}>
        我知道了
      </button>
    </dialog>
  );
}
