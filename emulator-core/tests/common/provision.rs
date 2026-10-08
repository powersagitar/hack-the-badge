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

pub enum Outcome {
    Done(String),
    Timeout(String),
}

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

/// The registration-desk flow: `put <IDENTITY_PATH> <len>`, wait for
/// `READY`, the bytes, then `prov apply`. Waits for the first prompt before
/// typing (the REPL discards input during its 500 ms terminal probe).
pub fn provision(
    rt: &mut FirmwareRuntime,
    identity_json: &[u8],
    deadline: u64,
) -> Result<String, String> {
    if wait_for_any(rt, 0, &[PROMPT], deadline).is_none() {
        return Err(format!("no `{PROMPT}` prompt by step {deadline}"));
    }
    let marker = rt.console_output().len();
    let header = format!("put {IDENTITY_PATH} {}{LINE_END}", identity_json.len());
    if rt.serial_input(header.as_bytes()) != header.len() {
        return Err("serial queue refused the put command".into());
    }
    if wait_for_any(rt, marker, &[PUT_READY], deadline).is_none() {
        return Err(format!(
            "no `{PUT_READY}` after put:\n{}",
            console_since(rt, marker)
        ));
    }
    if rt.serial_input(identity_json) != identity_json.len() {
        return Err("serial queue refused the payload".into());
    }
    let ok = format!("OK {}", identity_json.len());
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
        return Err(format!("no prompt after put:\n{}", console_since(rt, marker)));
    }
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
