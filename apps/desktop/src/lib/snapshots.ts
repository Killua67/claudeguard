import type { Snapshot } from './types';
/** IPC responses and events may arrive out of order; old green must not win. */
export function newerSnapshot(previous: Snapshot | null, next: Snapshot): Snapshot {
  if (!previous) return next;
  if (previous.sessionId !== next.sessionId || next.sequence <= previous.sequence) return previous;
  return next;
}
