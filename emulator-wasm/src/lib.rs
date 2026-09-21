//! Thin `wasm-bindgen` shim over `emulator-core`.
//!
//! Deliberately logic-free: everything interesting lives in
//! `emulator_core::runtime::FirmwareRuntime`, which is plain Rust and covered
//! by `cargo test -p emulator-core` without a WASM round-trip. What's here is
//! only the boundary — type marshalling, and turning Rust `Result`s into JS
//! exceptions.
//!
//! Built into `src/cpu/wasm-pkg/` (gitignored) by `bun run build:wasm`;
//! consumed by `src/cpu/bridge.ts`.
//!
//! ## Framebuffer aliasing safety
//!
//! [`FirmwareEmulator::framebuffer`] returns an owned `Vec<u16>`, **not** a
//! borrowed slice. That distinction is the whole safety argument, so it's worth
//! stating plainly: a `&[u16]` return would become a `Uint16Array` *view* onto
//! WASM linear memory, and any later Rust-side allocation that grows the WASM
//! heap detaches every such view (and a `Vec` reallocation would leave it
//! pointing at freed bytes). Holding one across a `run()` call — exactly what a
//! render loop does — is a use-after-free waiting to happen.
//!
//! For an owned `Vec<u16>`, wasm-bindgen's generated glue instead copies the
//! bytes out of linear memory into a fresh, JS-owned `Uint16Array`
//! (`getArrayU16FromWasm0(ptr, len).slice()`) and frees the Rust allocation
//! before returning. What crosses the boundary is therefore a plain JS array
//! with no relationship to WASM memory at all, safe to hold indefinitely.
//! `src/cpu/bridge.ts` restates this contract on the TS side.
//!
//! The cost is one 150 KiB copy per frame (320 × 240 × 2 bytes), which is
//! cheap next to the RGB565→RGBA8888 conversion `blitFramebuffer` already does
//! on every pixel of it.

use emulator_core::runtime::{FirmwareRuntime, RunSummary};
use wasm_bindgen::prelude::*;

/// Kept from the Task 1 scaffolding: the smoke test that proves the
/// Rust→WASM→TS round trip itself works (see `test/cpu-wasm.test.ts`).
#[wasm_bindgen]
pub fn add(a: u32, b: u32) -> u32 {
    emulator_core::add(a, b)
}

/// What one [`FirmwareEmulator::run`] call did — see
/// [`emulator_core::runtime::RunSummary`].
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct RunReport {
    inner: RunSummary,
}

#[wasm_bindgen]
impl RunReport {
    /// Instruction steps executed.
    #[wasm_bindgen(getter)]
    pub fn steps(&self) -> u32 {
        self.inner.steps
    }

    /// How many of those steps took a trap instead of completing an
    /// instruction.
    #[wasm_bindgen(getter)]
    pub fn traps(&self) -> u32 {
        self.inner.traps
    }

    /// How many of those steps intercepted a mask-ROM HLE stub.
    #[wasm_bindgen(getter, js_name = romStubCalls)]
    pub fn rom_stub_calls(&self) -> u32 {
        self.inner.rom_stub_calls
    }

    /// `pc` after the run.
    #[wasm_bindgen(getter)]
    pub fn pc(&self) -> u32 {
        self.inner.pc
    }

    /// The faulting address of the most recent unmapped instruction fetch in
    /// this run, or `undefined` if there wasn't one. A non-`undefined` value
    /// almost always means a ROM function that needs a stub
    /// (`emulator_core::rom`).
    #[wasm_bindgen(getter, js_name = lastInstructionFault)]
    pub fn last_instruction_fault(&self) -> Option<u32> {
        self.inner.last_instruction_fault
    }
}

/// The real-firmware emulator, as seen from JS.
#[wasm_bindgen]
pub struct FirmwareEmulator {
    inner: FirmwareRuntime,
}

#[wasm_bindgen]
impl FirmwareEmulator {
    /// Boots `image` — the raw bytes of `public/firmware/factory.bin`.
    /// Throws if the bytes aren't a parseable ESP-IDF app image.
    #[wasm_bindgen(constructor)]
    pub fn new(image: &[u8]) -> Result<FirmwareEmulator, JsError> {
        let inner = FirmwareRuntime::from_image(image).map_err(|e| {
            JsError::new(&format!("factory.bin is not a parseable app image: {e:?}"))
        })?;
        Ok(Self { inner })
    }

    /// Re-boots from the same image, keeping held buttons.
    pub fn reset(&mut self) -> Result<(), JsError> {
        self.inner
            .reset()
            .map_err(|e| JsError::new(&format!("re-parsing the retained image failed: {e:?}")))?;
        Ok(())
    }

    /// Runs `budget` instruction steps.
    pub fn run(&mut self, budget: u32) -> RunReport {
        RunReport {
            inner: self.inner.run(budget),
        }
    }

    /// Presses/releases a raw button slot — slot 0 is `PIN_START` (GPIO9),
    /// slots 1..=7 are the 74HC165's button bits. The human-name→slot mapping
    /// lives in `src/runtime/firmware-runtime.ts`; see
    /// `emulator_core::runtime`'s module doc for why it isn't in Rust.
    #[wasm_bindgen(js_name = setButton)]
    pub fn set_button(&mut self, slot: usize, pressed: bool) {
        self.inner.set_raw_button(slot, pressed);
    }

    /// The reconstructed ST7789 framebuffer as a fresh, JS-owned
    /// `Uint16Array`: RGB565, row-major, `width * height` elements. See this
    /// module's "Framebuffer aliasing safety" note for why this copies.
    pub fn framebuffer(&self) -> Vec<u16> {
        self.inner.framebuffer().to_vec()
    }

    #[wasm_bindgen(getter, js_name = screenWidth)]
    pub fn screen_width(&self) -> usize {
        self.inner.screen_width()
    }

    #[wasm_bindgen(getter, js_name = screenHeight)]
    pub fn screen_height(&self) -> usize {
        self.inner.screen_height()
    }

    /// Current `pc`, for a diagnostics HUD.
    #[wasm_bindgen(getter)]
    pub fn pc(&self) -> u32 {
        self.inner.pc()
    }

    /// Total steps since construction or the last [`FirmwareEmulator::reset`].
    /// `f64` rather than `u64` so it arrives in JS as an ordinary `number`
    /// instead of a `BigInt`; exact up to 2^53 steps, which at any plausible
    /// emulation rate is centuries.
    #[wasm_bindgen(getter, js_name = totalSteps)]
    pub fn total_steps(&self) -> f64 {
        self.inner.total_steps() as f64
    }
}
