import { describe, it, expect } from 'vitest';
import { reportText } from './report';
import { rules, type Snapshot } from './types';
describe('privacy and contract', () => {
  it('has exactly fifteen stable unique IDs', () => {
    expect(new Set(rules.map((r) => r.id)).size).toBe(15);
    expect(rules.map((r) => r.id)).toContain('final_recheck');
  });
  it('exports a bounded report projection and labels demo evidence', () => {
    const state = {
      demo: true,
      policyVersion: 1,
      checks: [],
      profile: { expectedProxyExecutable: '/private/user/path' },
      baseline: '203.0.113.1',
      canLaunch: true,
    } as unknown as Snapshot;
    const text = reportText(state);
    expect(text).toContain('offline-demo');
    for (const secret of ['203.0.113', '/private/user', 'canLaunch', 'baseline'])
      expect(text).not.toContain(secret);
  });
});
it('does not export notification correlation IDs or app identity and does not invent receipt', () => {
  const state = {
    demo: false,
    policyVersion: 1,
    checks: [],
    events: [],
    notificationDiagnostics: {
      appEnabled: true,
      applicationId: 'private-app-id',
      system: {
        authorization: 'authorized',
        alert: 'enabled',
        alertStyle: 'banner',
        sound: 'enabled',
        notificationCenter: 'enabled',
        lockScreen: 'unknown',
      },
      lastTest: {
        sequence: 98765,
        phase: 'accepted',
        receipt: 'unconfirmed',
        inNotificationCenter: true,
      },
    },
  } as unknown as Snapshot;
  const text = reportText(state);
  expect(text).toContain('unconfirmed');
  expect(text).toContain('inNotificationCenter');
  expect(text).not.toContain('98765');
  expect(text).not.toContain('private-app-id');
});
