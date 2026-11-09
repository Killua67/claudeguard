import { expect, it } from 'vitest';
import { newerSnapshot } from './snapshots';
import type { Snapshot } from './types';
it('keeps a newer problem event when an older successful scan response arrives late', () => {
  const green = { sessionId: 'fixture', sequence: 1, canLaunch: true, issue: null } as Snapshot;
  const problem = { ...green, sequence: 2, canLaunch: false, issue: '代理发生变化' };
  let state: Snapshot | null = newerSnapshot(null, problem);
  state = newerSnapshot(state, green);
  expect(state).toBe(problem);
  expect(state.canLaunch).toBe(false);
  expect(newerSnapshot(state, { ...green, sessionId: 'other', sequence: 999 })).toBe(problem);
  expect(newerSnapshot(state, { ...green, sequence: 3 }).canLaunch).toBe(true);
});
