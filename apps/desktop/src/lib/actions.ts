import type { Profile } from './types';
/** Failed config must never be hidden by a successful scan of the old profile. */
export async function saveProfileAndCheck(
  profile: Profile,
  configure: (p: Profile) => Promise<boolean>,
  scan: () => Promise<void>,
): Promise<boolean> {
  if (!(await configure(profile))) return false;
  await scan();
  return true;
}
