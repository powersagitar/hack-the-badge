import { describe, expect, test } from "bun:test";
import { roleLabel } from "../src/ui/shell";

describe("roleLabel", () => {
  test("shows a firmware role string as words", () => {
    expect(roleLabel("hacker")).toBe("hacker");
    expect(roleLabel("workshop_lead")).toBe("workshop lead");
  });
});
