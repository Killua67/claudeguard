export function Icon({
  name,
}: {
  name: 'cli' | 'desktop' | 'pulse' | 'refresh' | 'hand' | 'check' | 'info' | 'settings';
}) {
  const paths = {
    cli: (
      <>
        <rect x="2" y="3" width="20" height="17" rx="2" />
        <path d="m6 8 3 3-3 3m6 0h5" />
      </>
    ),
    desktop: (
      <>
        <rect x="2" y="3" width="20" height="14" rx="2" />
        <path d="M12 17v4m-5 0h10" />
      </>
    ),
    pulse: <path d="M1 12h5l3-8 5 16 3-8h6" />,
    refresh: (
      <>
        <path d="M20 7a9 9 0 1 0 1 9M20 2v6h-6" />
      </>
    ),
    hand: (
      <>
        <path d="M7 12V6a1.5 1.5 0 0 1 3 0v6-9a1.5 1.5 0 0 1 3 0v9-7a1.5 1.5 0 0 1 3 0v7-4a1.5 1.5 0 0 1 3 0v8c0 4-3 6-6 6-2 0-4-1-5-3l-4-6a1.5 1.5 0 0 1 2-2l2 2" />
      </>
    ),
    check: <path d="m5 12 5 6L20 5" />,
    info: (
      <>
        <circle cx="12" cy="12" r="9" />
        <path d="M12 11v6m0-10v.1" />
      </>
    ),
    settings: (
      <>
        <circle cx="12" cy="12" r="3" />
        <path d="m10 2-1 3-3 1-3 1 1 4v2l-1 4 3 1 3 1 1 3h4l1-3 3-1 3-1-1-4v-2l1-4-3-1-3-1-1-3Z" />
      </>
    ),
  };
  return (
    <svg
      className={`icon icon-${name}`}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {paths[name]}
    </svg>
  );
}
