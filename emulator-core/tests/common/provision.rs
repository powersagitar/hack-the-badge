//! Driving the real firmware's console REPL from tests, as a USB host
//! would. The REPL, its `put` and `prov` commands and their output are the
//! firmware's (`docs/milestone-5-decisions.md`, "Trace facts"); this module
//! only types and waits. The console buffer is a 256 KiB ring, far above
//! what boot and provisioning print, so byte offsets into it are stable.
#![allow(dead_code)] // each test binary uses a subset

use emulator_core::runtime::FirmwareRuntime;

pub const PROMPT: &str = "badge> ";
pub const IDENTITY_PATH: &str = "/littlefs/identity.json";
/// What ends a REPL line (Task 1, `REPL_LINE_END`): a single `\n`. `\r\n`
/// would end the command and then submit an empty line.
pub const LINE_END: &str = "\n";
/// What `put` prints once it has opened the file and is about to read the
/// payload with raw `read()` (Task 1, `PUT_READ`). The payload is sent only
/// after this appears: identity files exceed the driver's 256-byte RX ring
/// buffer, whose ISR drops what does not fit, so a fixed sleep would not
/// be enough in general (decisions file, R-T1-3).
pub const PUT_READY: &str = "READY";
const CHUNK: u32 = 50_000;
/// Host-side pacing of a `put` payload, NOT a bus property: the host sends
/// 64-byte chunks this many steps apart. The firmware's driver ISR drops
/// what does not fit its 256-byte RX ring, and the `put` task needs about
/// 20-24k steps per 64-byte packet to drain it (measured, Milestone 5 Task
/// D-M5-1: a 600-byte payload fails at 20,000 steps/packet and passes at
/// 24,000), so the gap is 2x that. Re-measured under Milestone 6's clock
/// (packets 821 SYSTIMER ticks apart): 24,000 still passes both the
/// 600-byte payload and role hacker's provisioning, 12,000 fails the
/// former, so the gap stays 48,000. See `docs/milestone-5-decisions.md`
/// and `docs/milestone-6-decisions.md`.
pub const PUT_CHUNK_GAP_STEPS: u32 = 48_000;

pub enum Outcome {
    Done(String),
    Timeout(String),
}

/// The console text after byte `marker`, or all of it if `marker` is past
/// the end. Offsets are stable only until the 256 KiB console ring
/// (`CONSOLE_CAPACITY`) wraps, far above what boot and provisioning print;
/// after a wrap the fallback rescans the whole (newest) text, so an old
/// match could be seen again.
fn console_since(rt: &FirmwareRuntime, marker: usize) -> String {
    let text = rt.console_output();
    text.get(marker..).unwrap_or(&text).to_string()
}

/// Runs until the console text after byte `marker` contains one of
/// `needles` (returns its index and that text), or `deadline` total steps.
pub fn wait_for_any(
    rt: &mut FirmwareRuntime,
    marker: usize,
    needles: &[&str],
    deadline: u64,
) -> Option<(usize, String)> {
    loop {
        let since = console_since(rt, marker);
        if let Some(i) = needles.iter().position(|n| since.contains(n)) {
            return Some((i, since));
        }
        if rt.total_steps() >= deadline {
            return None;
        }
        let s = rt.run(CHUNK);
        assert_eq!(s.last_instruction_fault, None, "{s:?}");
    }
}

/// Types `line` and waits for the REPL's next prompt.
pub fn type_line(rt: &mut FirmwareRuntime, line: &str, deadline: u64) -> Outcome {
    let marker = rt.console_output().len();
    let input = format!("{line}{LINE_END}");
    assert_eq!(rt.serial_input(input.as_bytes()), input.len());
    match wait_for_any(rt, marker, &[PROMPT], deadline) {
        Some((_, text)) => Outcome::Done(text),
        None => Outcome::Timeout(console_since(rt, marker)),
    }
}

/// `put <path> <len>`, wait for `READY`, send `payload`, wait for
/// `OK <len>` and the REPL's next prompt. Waits for the first prompt before
/// typing (the REPL discards input during its 500 ms terminal probe).
/// Returns the console text from the `put` command on.
pub fn put_file(
    rt: &mut FirmwareRuntime,
    path: &str,
    payload: &[u8],
    deadline: u64,
) -> Result<String, String> {
    if wait_for_any(rt, 0, &[PROMPT], deadline).is_none() {
        return Err(format!("no `{PROMPT}` prompt by step {deadline}"));
    }
    let marker = rt.console_output().len();
    let header = format!("put {path} {}{LINE_END}", payload.len());
    if rt.serial_input(header.as_bytes()) != header.len() {
        return Err("serial queue refused the put command".into());
    }
    if wait_for_any(rt, marker, &[PUT_READY], deadline).is_none() {
        return Err(format!(
            "no `{PUT_READY}` after put:\n{}",
            console_since(rt, marker)
        ));
    }
    for (i, chunk) in payload.chunks(64).enumerate() {
        if i > 0 {
            let s = rt.run(PUT_CHUNK_GAP_STEPS);
            assert_eq!(s.last_instruction_fault, None, "{s:?}");
        }
        if rt.serial_input(chunk) != chunk.len() {
            return Err("serial queue refused the payload".into());
        }
    }
    let ok = format!("OK {}", payload.len());
    match wait_for_any(
        rt,
        marker,
        &[&ok, "bad size", "write error", "short read"],
        deadline,
    ) {
        Some((0, _)) => {}
        Some((_, text)) => return Err(format!("put failed:\n{text}")),
        None => return Err(format!("put timed out:\n{}", console_since(rt, marker))),
    }
    // `put` returns to the REPL, which prints a fresh prompt; type only then.
    if wait_for_any(rt, marker, &[PROMPT], deadline).is_none() {
        return Err(format!(
            "no prompt after put:\n{}",
            console_since(rt, marker)
        ));
    }
    Ok(console_since(rt, marker))
}

/// The registration-desk flow: `put <IDENTITY_PATH> <len>` (see
/// [`put_file`]), then `prov apply`.
pub fn provision(
    rt: &mut FirmwareRuntime,
    identity_json: &[u8],
    deadline: u64,
) -> Result<String, String> {
    put_file(rt, IDENTITY_PATH, identity_json, deadline)?;
    let marker = rt.console_output().len();
    let apply = format!("prov apply{LINE_END}");
    if rt.serial_input(apply.as_bytes()) != apply.len() {
        return Err("serial queue refused prov apply".into());
    }
    // The result line is complete when the REPL prompts again (`prov apply`
    // prints it, then flashes the LEDs; the prompt follows).
    let outcome = wait_for_any(rt, marker, &["PROV OK", "PROV FAIL"], deadline);
    let Some((which, seen)) = outcome else {
        return Err(format!(
            "prov apply timed out:\n{}",
            console_since(rt, marker)
        ));
    };
    // Look for the prompt only after the result line, not the one before it.
    let found = seen.find(["PROV OK", "PROV FAIL"][which]).unwrap_or(0);
    let text = match wait_for_any(rt, marker + found, &[PROMPT], deadline) {
        Some((_, text)) => text,
        None => console_since(rt, marker + found),
    };
    if which == 0 {
        Ok(text)
    } else {
        Err(text)
    }
}

/// The fake upload credential the new Sync rungs set with
/// [`set_upload_token`]: 64 lowercase hex characters, obviously fake
/// (`0123456789abcdef` four times), one constant for every role. Committed
/// with the organizers' permission (2026-11-10), like the fake identities;
/// it never goes into the identity files. Mirrored as `FAKE_UPLOAD_TOKEN`
/// in `frontend/src/runtime/provisioner.ts`; keep the two equal.
pub const FAKE_UPLOAD_TOKEN: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// What `badge_token set` prints before it reads the token's 64 bytes
/// from stdin (v392's console; the token is sent only after this).
pub const TOKEN_READY: &str = "BADGE_TOKEN READY";
/// What `badge_token set` prints once the credential is stored.
pub const TOKEN_OK: &str = "BADGE_TOKEN OK";
/// What `badge_token set` prints when it refuses the token or cannot
/// store it.
pub const TOKEN_FAIL: &str = "BADGE_TOKEN FAIL";
/// The REPL's answer to a malformed `badge_token` command.
pub const TOKEN_USAGE: &str = "usage:";

/// The registration desk's second step: `badge_token set`, wait for
/// [`TOKEN_READY`], send `token`'s bytes in one `serial_input` (64 bytes,
/// one USB packet, under the 256-byte RX ring), wait for [`TOKEN_OK`] and
/// then the REPL's next prompt after it. Call it only at an idle prompt
/// (the caller has just seen one, e.g. right after [`provision`]).
/// [`TOKEN_USAGE`] or [`TOKEN_FAIL`] is an `Err` with the console text.
/// Returns the console text from the command on.
pub fn set_upload_token(
    rt: &mut FirmwareRuntime,
    token: &str,
    deadline: u64,
) -> Result<String, String> {
    let marker = rt.console_output().len();
    let command = format!("badge_token set{LINE_END}");
    if rt.serial_input(command.as_bytes()) != command.len() {
        return Err("serial queue refused badge_token set".into());
    }
    match wait_for_any(
        rt,
        marker,
        &[TOKEN_READY, TOKEN_USAGE, TOKEN_FAIL],
        deadline,
    ) {
        Some((0, _)) => {}
        Some((_, text)) => return Err(format!("badge_token set refused:\n{text}")),
        None => {
            return Err(format!(
                "no `{TOKEN_READY}`:\n{}",
                console_since(rt, marker)
            ))
        }
    }
    if rt.serial_input(token.as_bytes()) != token.len() {
        return Err("serial queue refused the token".into());
    }
    let seen = match wait_for_any(rt, marker, &[TOKEN_OK, TOKEN_FAIL], deadline) {
        Some((0, seen)) => seen,
        Some((_, text)) => return Err(format!("token refused:\n{text}")),
        None => {
            return Err(format!(
                "badge_token set timed out:\n{}",
                console_since(rt, marker)
            ))
        }
    };
    // Look for the prompt only after the result line.
    let found = seen.find(TOKEN_OK).unwrap_or(0);
    if wait_for_any(rt, marker + found, &[PROMPT], deadline).is_none() {
        return Err(format!(
            "no prompt after `{TOKEN_OK}`:\n{}",
            console_since(rt, marker)
        ));
    }
    Ok(console_since(rt, marker))
}

/// A committed test identity
/// (`frontend/public/firmware/test-identities/<role>.json`: obviously fake,
/// one per role, committed with the organizers' permission, R-M5-2). When
/// `$BADGE_TEST_IDENTITIES` is set, the file is read from that directory
/// instead (a relative path is taken from the repo root), for trying a
/// local variant.
pub fn identity_fixture(role: &str) -> Vec<u8> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let dir = std::env::var_os("BADGE_TEST_IDENTITIES")
        .map(|d| root.join(d))
        .unwrap_or_else(|| root.join("frontend/public/firmware/test-identities"));
    std::fs::read(dir.join(format!("{role}.json")))
        .unwrap_or_else(|e| panic!("fixture {role} in {}: {e}", dir.display()))
}
