import { expect, it } from 'vitest';
import { saveProfileAndCheck } from './actions';
import catalog from '../../../../resources/network-dependencies.json';
import type { Profile } from './types';
const profile: Profile = {
  schemaVersion: 1,
  proxyUrl: 'http://127.0.0.1:9999',
  expectedCountry: 'US',
  expectedProxyExecutable: '/fixture/proxy',
  nonMainlandTimezone: true,
};
it('does not scan an old profile after config is rejected', async () => {
  const calls: string[] = [];
  let error = '';
  const saved = await saveProfileAndCheck(
    profile,
    async () => {
      calls.push('configure');
      error = '保存失败';
      return false;
    },
    async () => {
      calls.push('scan');
      error = '';
    },
  );
  expect(saved).toBe(false);
  expect(calls).toEqual(['configure']);
  expect(error).toBe('保存失败');
});
it('scans only after the new profile has been accepted', async () => {
  const calls: string[] = [];
  expect(
    await saveProfileAndCheck(
      profile,
      async (p) => {
        expect(p).toBe(profile);
        calls.push('configure');
        return true;
      },
      async () => {
        calls.push('scan');
      },
    ),
  ).toBe(true);
  expect(calls).toEqual(['configure', 'scan']);
});
it('preserves the legacy offline dependency snapshot without active URL actions', () => {
  expect(catalog.version).toBe('2026-10-27.1');
  expect(catalog.entries).toHaveLength(41);
  expect(new Set(catalog.entries.map((e) => e.host)).size).toBe(41);
  expect(catalog.entries.filter((e) => !e.host.includes('*'))).toHaveLength(31);
  expect(catalog.entries.filter((e) => e.scope === 'dynamic')).toHaveLength(6);
  expect(catalog.entries.filter((e) => e.scope === 'broadAlternative')).toHaveLength(4);
});
