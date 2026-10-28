const PATHS = {
  plus: 'M12 5v14M5 12h14',
  trash: 'M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14M10 11v6M14 11v6',
  up: 'M6 15l6-6 6 6',
  down: 'M6 9l6 6 6-6',
  right: 'M9 6l6 6-6 6',
  x: 'M6 6l12 12M18 6L6 18',
  service: 'M4 5h16v6H4zM4 13h16v6H4zM8 8h.01M8 16h.01',
  topic: 'M3 7l9-4 9 4-9 4-9-4zM3 12l9 4 9-4M3 17l9 4 9-4',
  schema:
    'M8 4c-2 0-3 1-3 3v2c0 1-1 2-2 3 1 1 2 2 2 3v2c0 2 1 3 3 3M16 4c2 0 3 1 3 3v2c0 1 1 2 2 3-1 1-2 2-2 3v2c0 2-1 3-3 3',
  log: 'M6 3h9l4 4v14H6zM14 3v5h5M9 13h7M9 17h7',
  let: 'M5 9h14M5 15h14',
  publish: 'M22 2L11 13M22 2l-7 20-4-9-9-4z',
  callRpc: 'M7 7h13l-4-4M17 17H4l4 4',
  branch: 'M6 3v12M18 9a3 3 0 100-6 3 3 0 000 6zM6 21a3 3 0 100-6 3 3 0 000 6zM18 9c0 6-12 3-12 6',
  respond: 'M20 4v7a4 4 0 01-4 4H4M9 10l-5 5 5 5',
  code: 'M16 18l6-6-6-6M8 6l-6 6 6 6',
  open: 'M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z',
  save: 'M5 3h11l3 3v15H5zM8 3v6h8V3M8 21v-7h8v7',
  export: 'M12 3v12M7 10l5 5 5-5M5 21h14',
  file: 'M6 3h9l4 4v14H6zM14 3v5h5',
  warn: 'M12 3l10 18H2zM12 10v5M12 18h.01',
  error: 'M12 22a10 10 0 100-20 10 10 0 000 20zM12 7v6M12 16h.01',
  ok: 'M12 22a10 10 0 100-20 10 10 0 000 20zM8 12l3 3 5-6',
  copy: 'M9 9h11v11H9zM5 15V4h11',
  sparkles:
    'M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8zM19 16l.9 2.1L22 19l-2.1.9L19 22l-.9-2.1L16 19l2.1-.9z',
  grip: 'M9 5h.01M9 12h.01M9 19h.01M15 5h.01M15 12h.01M15 19h.01',
  graph: 'M5 5h5v5H5zM14 14h5v5h-5zM10 7.5h4a3 3 0 013 3V14',
  new: 'M6 3h9l4 4v14H6zM14 3v5h5M12 11v6M9 14h6',
} as const

export type IconName = keyof typeof PATHS

export function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={PATHS[name]} />
    </svg>
  )
}
