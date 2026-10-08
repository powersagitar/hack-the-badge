/**
 * Obviously fake badge identities, one per role in the firmware's role
 * table (`ROLE_TABLE` in docs/milestone-5-decisions.md, "Trace facts";
 * ruling R-M5-4), in its index order. Committed with the Hack the North
 * organizers' permission (R-M5-2, relayed 2026-10-08). They are typed into
 * the emulated badge's own console to provision it
 * (`./provisioner.ts`); `frontend/test/test-identities.test.ts` keeps them
 * obviously fake.
 */
export const TEST_IDENTITY_ROLES = [
  "hacker",
  "organizer",
  "sponsor",
  "judge",
  "mentor",
  "volunteer",
  "media",
  "staff",
  "general",
  "workshop_lead",
  "visitor",
] as const;

/** Where `role`'s fixture is served from (`public/firmware/test-identities/`). */
export function testIdentityUrl(role: string): string {
  return `/firmware/test-identities/${role}.json`;
}
