import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { TEST_IDENTITY_ROLES, testIdentityUrl } from "../src/runtime/test-identities";

const DIR = new URL("../public/firmware/test-identities/", import.meta.url);
/**
 * Per-field maximum bytes, from docs/milestone-5-decisions.md "Trace
 * facts" (`VALIDATION`: the parser's buffer size - 1 at each call). `role`
 * has no byte limit (it is matched against `ROLE_TABLE`, checked below).
 */
const MAX_LEN: Record<string, number> = {
  badge_id: 63,
  display_name: 39,
  account_email: 63,
  claim_id: 23,
  net_email: 63,
  net_phone: 19,
  net_linkedin: 39,
  net_discord: 39,
  net_instagram: 31,
  net_x: 31,
};

describe("committed test identities", () => {
  test("one file per role, and nothing else", () => {
    const files = readdirSync(DIR).sort();
    expect(files).toEqual([...TEST_IDENTITY_ROLES].map((r) => `${r}.json`).sort());
  });

  test("testIdentityUrl points into public/firmware/test-identities", () => {
    expect(testIdentityUrl("hacker")).toBe("/firmware/test-identities/hacker.json");
  });

  for (const role of TEST_IDENTITY_ROLES) {
    test(`${role} is obviously fake and within the firmware's limits`, () => {
      const text = readFileSync(new URL(`${role}.json`, DIR), "utf8");
      // The VFS turns `\r` into `\n` in a `put` payload (Trace facts, PUT_READ).
      expect(text.includes("\r")).toBe(false);
      const id = JSON.parse(text);
      expect(id.role).toBe(role);
      expect(id.display_name.startsWith("Test ")).toBe(true);
      expect(id.badge_id.startsWith("test-")).toBe(true);
      expect(id.account_email.endsWith("@example.com")).toBe(true);
      for (const k of ["net_email", "net_phone", "net_linkedin", "net_discord", "net_instagram", "net_x"]) {
        expect(id[k]).toBe("");
      }
      for (const [k, max] of Object.entries(MAX_LEN)) {
        expect(max).toBeGreaterThan(0);
        expect(typeof id[k]).toBe("string");
        expect(new TextEncoder().encode(id[k]).length).toBeLessThanOrEqual(max);
      }
    });
  }
});
