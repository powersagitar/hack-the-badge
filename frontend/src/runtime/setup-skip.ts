/**
 * Skip setup (Milestone 13): types the badge's own console commands that
 * end the onboarding app, and watches whether that app is on screen.
 *
 * The skip (spec, "How the skip works"; pinned natively by
 * `skipping_setup_lands_on_the_registered_screen` in
 * `emulator-core/tests/boot_progress.rs`): `config system onboard_build
 * 2147483647` writes the setting Setup's `finish()` writes (INT32_MAX is at
 * or above any build number), and the firmware answers
 * `OK system.onboard_build=2147483647`. Setup never re-reads it, so the
 * skip then types `reboot`; the new boot takes the normal path to My Badge.
 * Exactly one `config` and one `reboot` are typed per skip, and nothing
 * after the skip is done or has failed.
 *
 * Like the provisioner (`./provisioner.ts`), the skipper is a state
 * machine polled between runs (once per frame by the firmware runtime);
 * it never runs the CPU. Its phases: wait for a prompt in the current
 * boot (at or after `bootConsoleStart()`), type the config; wait for the
 * `OK` line and a prompt after it (a `usage:` line fails the skip with
 * that line), type `reboot`; done when `softResetCount()` rises. A soft
 * reset before `reboot` is typed fails the skip and types nothing more,
 * as a reboot mid-provision does (the new boot's REPL never saw the
 * command). No answer at all is the caller's timeout
 * ({@link SKIP_SETUP_TIMEOUT_STEPS}).
 *
 * The watcher ({@link setupOnScreen}, spec "When the button shows"): the
 * app registry logs `app_reg: launched <name>` at every launch, both ways
 * into Setup included (the boot-time path, pinned by
 * `setup_logs_its_launch_after_a_reboot`, and My Badge's hand-off after
 * `prov apply`), so Setup is on screen when the current boot's last
 * complete launch line names it.
 *
 * Console offsets are absolute byte offsets (`console-io.ts`), so a console
 * past its 256 KiB ring still finds the prompt and the reply.
 */
import type { FirmwareEmulatorHandle } from "../cpu/bridge";
import { completeLine, consoleSince, PROMPT, typeLine } from "./console-io";

/** The skip's first line: Setup's own `onboard_build` setting, at INT32_MAX. */
export const SKIP_SETUP_CONFIG = "config system onboard_build 2147483647";
/** The REPL's answer to {@link SKIP_SETUP_CONFIG} (`OK %s.%s=%s`). */
export const SKIP_SETUP_CONFIG_OK = "OK system.onboard_build=2147483647";
/** The skip's second line: the console's reboot command. */
export const REBOOT_LINE = "reboot";
/** The registry's log line when Setup launches. */
export const SETUP_LAUNCHED = "app_reg: launched Setup";
/** The registry's log line prefix for any launch; the app's name follows. */
export const LAUNCHED = "app_reg: launched ";
/**
 * Emulated steps after `skipSetup()` before an unfinished skip is reported
 * as failed ("timed out"); the provisioner's bound
 * (`PROVISION_TIMEOUT_STEPS`). The skip takes about 1.85M steps on v392.
 */
export const SKIP_SETUP_TIMEOUT_STEPS = 60_000_000;

/** Why a skip fails when the badge soft-resets before `reboot` is typed. */
export const SKIP_REBOOTED_REASON = "the badge rebooted during the skip";
/** Why a skip fails when the serial queue refuses bytes. */
const REFUSED = "the serial queue refused the input";
/** The REPL's answer to a malformed command (`usage: config <slug> <key> <value>`). */
const USAGE = "usage:";

export type SkipState = { kind: "typing" } | { kind: "done" } | { kind: "failed"; reason: string };

export interface SetupSkipper {
  poll(): SkipState;
}

/**
 * True iff the last complete `app_reg: launched <name>` line in
 * `bootText` (the current boot's console) names `Setup`. An unterminated
 * launch line is not counted yet.
 */
export function setupOnScreen(bootText: string): boolean {
  let at = bootText.lastIndexOf(LAUNCHED);
  // A launch line still unterminated at the end: look at the one before.
  if (at >= 0 && bootText.indexOf("\n", at) < 0) at = at > 0 ? bootText.lastIndexOf(LAUNCHED, at - 1) : -1;
  if (at < 0) return false;
  const end = bootText.indexOf("\n", at);
  return bootText.slice(at + LAUNCHED.length, end).trim() === "Setup";
}

/**
 * `bootConsoleStart` returns the absolute console offset where the current
 * boot began; it is read at every poll while the skipper waits for its
 * first prompt.
 */
export function createSetupSkipper(handle: FirmwareEmulatorHandle, bootConsoleStart: () => number): SetupSkipper {
  let phase: "prompt" | "configReply" | "reset" | "over" = "prompt";
  /** Absolute console offset (bytes) where the config's output starts. */
  let marker = 0;
  /** `softResetCount()` when the skipper was made: a rise is the skip's reboot, or a failure before it. */
  const resetsAtStart = handle.softResetCount();
  let result: SkipState = { kind: "typing" };

  function fail(reason: string): SkipState {
    phase = "over";
    return (result = { kind: "failed", reason });
  }

  return {
    poll(): SkipState {
      if (phase === "over") return result;
      const rebooted = handle.softResetCount() !== resetsAtStart;
      if (phase === "reset") {
        if (!rebooted) return result;
        phase = "over";
        return (result = { kind: "done" });
      }
      if (rebooted) return fail(SKIP_REBOOTED_REASON);
      if (phase === "prompt") {
        if (!consoleSince(handle, bootConsoleStart()).includes(PROMPT)) return result;
        const at = typeLine(handle, SKIP_SETUP_CONFIG);
        if (at === null) return fail(REFUSED);
        marker = at;
        phase = "configReply";
        return result;
      }
      // configReply
      const text = consoleSince(handle, marker);
      const usageAt = text.indexOf(USAGE);
      if (usageAt >= 0) {
        const line = completeLine(text, usageAt);
        return line === undefined ? result : fail(line);
      }
      const okAt = text.indexOf(SKIP_SETUP_CONFIG_OK);
      // `config` returns to the REPL, which prompts again; type only then.
      if (okAt < 0 || text.indexOf(PROMPT, okAt) < 0) return result;
      if (typeLine(handle, REBOOT_LINE) === null) return fail(REFUSED);
      phase = "reset";
      return result;
    },
  };
}
