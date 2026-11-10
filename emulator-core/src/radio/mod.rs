//! The badge radio (Milestone 10): a high-level BLE controller at the HCI
//! boundary, in place of the ESP32-C3's mask-ROM controller.
//!
//! [`hci`] is the controller model (pure HCI). [`Radio`] connects it to the
//! firmware through app-image hooks (`crate::cpu::app_hooks`) at bt.c's
//! VHCI seam, whose v392 addresses are in [`v392`] (spec
//! `docs/superpowers/specs/2026-10-22-milestone10-badge-radio-design.md`,
//! "Measured on v392" and Part 1).
//!
//! ## The hooks (bt.c, ESP-IDF v5.5.3, `components/bt/controller/esp32c3/bt.c`)
//!
//! Every hook replaces its whole function with [`Cpu::hook_return`] or a
//! redirect, never [`Cpu::hook_pass_through`]: a pass-through survives a
//! FreeRTOS context switch, so another task arriving first would run the
//! closed blob behind it. Each mirrors bt.c's status checks on
//! `btdm_controller_status` ([`v392::BTDM_CONTROLLER_STATUS`]), which stays
//! the source of truth in guest memory, so the unhooked readers
//! (`esp_bt_controller_get_status`, the deinit paths) agree with it:
//!
//! - `esp_bt_controller_init`: `ESP_ERR_INVALID_STATE` unless IDLE, else
//!   INITED, `ESP_OK`, and [`Radio::init_count`] counts it. No blob, no
//!   ROM, so "BT controller compile version" does not print.
//! - `esp_bt_controller_enable`: `ESP_ERR_INVALID_STATE` unless INITED,
//!   else ENABLED, `ESP_OK`.
//! - `esp_vhci_host_register_callback(cb)`: `ESP_FAIL` unless ENABLED,
//!   else stores `cb->notify_host_send_available` and `cb->notify_host_recv`
//!   (`esp_vhci_host_callback_t`, `esp_bt.h` line 876), `ESP_OK`.
//! - `esp_vhci_host_check_send_available`: whether ENABLED (the blob's
//!   answer is always "yes" here: the model takes every packet at once).
//! - `esp_vhci_host_send_packet(data, len)`: nothing unless ENABLED, as
//!   bt.c; else the H4 packet goes to [`hci::HciController`], and the call
//!   returns through the trampoline below, which first calls
//!   `notify_host_send_available` (the controller took the packet; it
//!   gives back `esp_nimble_hci.c`'s `vhci_send_sem`, without which the
//!   next command waits out `NIMBLE_VHCI_TIMEOUT_MS`), then
//!   `notify_host_recv` with the command's own Command Complete (the oldest
//!   queued reply; a report waiting ahead of it stays queued for the idle
//!   task).
//!
//! ## Event injection (ruling R-T6-2)
//!
//! The real controller calls `notify_host_recv` (`host_rcv_pkt`) from its
//! own task. The model has no task, so it calls it in a guest task at a
//! function-call boundary it hooks, through
//! [`crate::rom::RADIO_CALL_TRAMPOLINE`]: the host lowers `sp` by a
//! 16-byte-aligned frame, writes into it the hooked call's `ra`, the old
//! `sp`, the two function pointers and the packet, and sets `pc` to the
//! trampoline, which makes the calls, restores `sp` and returns to `ra`.
//! A hooked entry is an ABI call boundary, so only caller-saved registers
//! change, as the call itself would have changed them; the callees save
//! the rest. The packet lives in its task's own stack frame, so two
//! deliveries in two tasks never share memory, and an interrupt taken in
//! the trampoline saves its context below the frame.
//!
//! - **Command replies:** inside the `send_packet` hook, in the NimBLE
//!   host task that sent the command, even with a report queued ahead of
//!   the reply.
//! - **Asynchronous events** (advertising reports; and any event left when
//!   one hook delivers one): from the idle task, at
//!   [`v392::ESP_V_APPLICATION_IDLE_HOOK`]'s entry, one event per idle
//!   pass, skipping that pass's hook body (it only waits for an
//!   interrupt; the next pass runs it). The idle hook is in the table only
//!   while an event is waiting and the callbacks are registered
//!   ([`Radio::on_hook`] swaps the table), so without radio traffic it
//!   never hits and costs nothing. Swapping is safe at any time here:
//!   `Cpu::set_app_hooks` drops only a pending pass-through, and this
//!   module never makes one.
//!
//! ## Resets
//!
//! A software reset rebuilds the `Cpu` (no hook table) and the guest
//! reloads `btdm_controller_status` as IDLE, so the runtime calls
//! [`Radio::soft_reset`] and [`Radio::install`] on every boot: the
//! controller model, the callbacks and the idle hook start over, the MAC
//! stays, and the radio initializes again as on power-on.

pub mod hci;
pub mod v392;

use crate::cpu::{rom_stubs, AppHookTable, Cpu, HookHit};
use crate::mem::bus::FirmwareBus;
use crate::mem::Bus;
use crate::rom::{
    RADIO_CALL_TRAMPOLINE_ADDR, RADIO_FRAME_FIRST_CALL, RADIO_FRAME_LEN, RADIO_FRAME_OLD_SP,
    RADIO_FRAME_PACKET, RADIO_FRAME_RA, RADIO_FRAME_SECOND_CALL,
};
use hci::{Advertisement, HciController};

/// Hook ids in the table [`Radio::install`] builds.
const HOOK_INIT: u16 = 1;
const HOOK_ENABLE: u16 = 2;
const HOOK_REGISTER_CALLBACK: u16 = 3;
const HOOK_CHECK_SEND_AVAILABLE: u16 = 4;
const HOOK_SEND_PACKET: u16 = 5;
const HOOK_IDLE: u16 = 6;

/// The largest H4 packet the model reads from the host: an H4 command is
/// the indicator, a 3-byte header and at most 255 parameter bytes.
const MAX_H4_COMMAND: u32 = 1 + 3 + 255;

/// The fixed part of an injection frame, before the packet.
const FRAME_HEADER: u32 = RADIO_FRAME_PACKET as u32;

/// The VHCI callbacks the host registered (`esp_vhci_host_callback_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Callbacks {
    send_available: u32,
    recv: u32,
}

/// The radio: the [`HciController`] model plus its guest wiring (module
/// doc). Owned by `crate::runtime::FirmwareRuntime`; a clone is a full
/// snapshot (ruling R-PF-2).
#[derive(Clone, Debug)]
pub struct Radio {
    /// The factory MAC the controller's BD_ADDR derives from (kept across
    /// every reset, as the chip's).
    mac: Option<[u8; 6]>,
    hci: HciController,
    callbacks: Option<Callbacks>,
    /// Successful `esp_bt_controller_init` hooks since power-on.
    init_count: u32,
    /// Hook hits answered since power-on (a diagnostic: zero on any path
    /// that never opens a radio app).
    hook_hits: u64,
    /// Whether the installed table holds the idle hook.
    idle_armed: bool,
}

impl Default for Radio {
    fn default() -> Self {
        Self::new()
    }
}

impl Radio {
    /// A powered-on radio with no MAC set.
    pub fn new() -> Self {
        Self {
            mac: None,
            hci: HciController::new(bd_addr(None)),
            callbacks: None,
            init_count: 0,
            hook_hits: 0,
            idle_armed: false,
        }
    }

    /// Sets the factory MAC the controller's BD_ADDR derives from. Takes
    /// effect at the next controller (re)start, as on the chip.
    pub fn set_mac(&mut self, mac: Option<[u8; 6]>) {
        self.mac = mac;
    }

    /// The factory MAC the radio was given.
    pub fn mac(&self) -> Option<[u8; 6]> {
        self.mac
    }

    /// Successful `esp_bt_controller_init` hooks since power-on (a cold
    /// reset makes a new `Radio`; a soft reset keeps the count).
    pub fn init_count(&self) -> u32 {
        self.init_count
    }

    /// Hook hits answered since power-on (a cold reset makes a new
    /// `Radio`; a soft reset keeps the count). Zero on any run that never
    /// opens a radio app: the hooks then cost nothing but the hook check.
    pub fn hook_hits(&self) -> u64 {
        self.hook_hits
    }

    /// The controller model, for the host API and tests.
    pub fn controller(&self) -> &HciController {
        &self.hci
    }

    /// The controller model, mutably: tests drive it without a guest.
    #[cfg(test)]
    pub(crate) fn controller_mut(&mut self) -> &mut HciController {
        &mut self.hci
    }

    /// The guest rebooted (software reset): the controller, the callbacks
    /// and the idle hook start over, the MAC and the init count stay.
    pub fn soft_reset(&mut self) {
        self.hci = HciController::new(bd_addr(self.mac));
        self.callbacks = None;
        self.idle_armed = false;
    }

    /// Installs the radio's hook table on a freshly booted `cpu`: the
    /// bt.c seam, plus the idle hook if an event is waiting. Inert until
    /// the firmware calls a hooked address, which only a radio app does.
    pub fn install(&mut self, cpu: &mut Cpu) {
        let want = self.wants_idle_hook();
        cpu.set_app_hooks(table(want));
        self.idle_armed = want;
    }

    /// Hears a peer's advertisement ([`HciController::deliver`]); a report
    /// it queues reaches the host from the idle task.
    pub fn deliver(&mut self, cpu: &mut Cpu, adv: &Advertisement, rssi: i8) -> bool {
        let queued = self.hci.deliver(adv, rssi);
        self.sync_idle_hook(cpu);
        queued
    }

    /// Answers one hook hit. Every arm ends in a return or a redirect.
    pub fn on_hook(&mut self, hit: HookHit, cpu: &mut Cpu, bus: &mut FirmwareBus) {
        self.hook_hits += 1;
        let status = bus.read32(v392::BTDM_CONTROLLER_STATUS);
        match hit.id {
            HOOK_INIT => {
                if status != v392::STATUS_IDLE {
                    cpu.hook_return(Some(v392::ESP_ERR_INVALID_STATE));
                    return;
                }
                bus.write32(v392::BTDM_CONTROLLER_STATUS, v392::STATUS_INITED);
                self.hci = HciController::new(bd_addr(self.mac));
                self.callbacks = None;
                self.init_count += 1;
                cpu.hook_return(Some(v392::ESP_OK));
                self.sync_idle_hook(cpu);
            }
            HOOK_ENABLE => {
                if status != v392::STATUS_INITED {
                    cpu.hook_return(Some(v392::ESP_ERR_INVALID_STATE));
                    return;
                }
                bus.write32(v392::BTDM_CONTROLLER_STATUS, v392::STATUS_ENABLED);
                cpu.hook_return(Some(v392::ESP_OK));
            }
            HOOK_REGISTER_CALLBACK => {
                if status != v392::STATUS_ENABLED {
                    cpu.hook_return(Some(v392::ESP_FAIL));
                    return;
                }
                let cb = cpu.regs.read(rom_stubs::REG_A0);
                self.callbacks = Some(Callbacks {
                    send_available: bus.read32(cb),
                    recv: bus.read32(cb.wrapping_add(4)),
                });
                cpu.hook_return(Some(v392::ESP_OK));
                self.sync_idle_hook(cpu);
            }
            HOOK_CHECK_SEND_AVAILABLE => {
                cpu.hook_return(Some(u32::from(status == v392::STATUS_ENABLED)));
            }
            HOOK_SEND_PACKET => {
                if status != v392::STATUS_ENABLED {
                    cpu.hook_return(None);
                    return;
                }
                let data = cpu.regs.read(rom_stubs::REG_A0);
                let len = cpu.regs.read(rom_stubs::REG_A1) & 0xFFFF;
                let packet: Vec<u8> = (0..len.min(MAX_H4_COMMAND))
                    .map(|i| bus.read8(data.wrapping_add(i)))
                    .collect();
                // A refused packet (malformed, or a host past its command
                // credit) gets no reply, as R-T5-6 rules.
                self.hci.host_to_controller(&packet);
                match self.callbacks {
                    Some(cb) => {
                        let event = self.hci.pop_reply();
                        self.call_back(cpu, bus, Some(cb.send_available), event, cb.recv);
                    }
                    None => cpu.hook_return(None),
                }
                self.sync_idle_hook(cpu);
            }
            HOOK_IDLE => {
                // With nothing to deliver, `pc` stays on the entry: the
                // hook leaves the table below and the body runs next step.
                if let Some(cb) = self.callbacks {
                    if let Some(event) = self.hci.pop_event() {
                        self.call_back(cpu, bus, None, Some(event), cb.recv);
                    }
                }
                self.sync_idle_hook(cpu);
            }
            _ => {
                debug_assert!(false, "unknown radio hook {hit:?}");
                cpu.hook_return(None);
            }
        }
    }

    /// Redirects the hooked call at `cpu.pc` through the trampoline:
    /// `first()` if given, then `recv(event)` if there is one, then back
    /// to the hooked call's `ra` (module doc, "Event injection").
    fn call_back(
        &mut self,
        cpu: &mut Cpu,
        bus: &mut FirmwareBus,
        first: Option<u32>,
        event: Option<Vec<u8>>,
        recv: u32,
    ) {
        let packet = event.as_deref().unwrap_or(&[]);
        let old_sp = cpu.regs.read(rom_stubs::REG_SP);
        let size = (FRAME_HEADER + packet.len() as u32 + 15) & !15;
        let sp = old_sp.wrapping_sub(size) & !15;
        let at = |off: i32| sp.wrapping_add(off as u32);
        bus.write32(at(RADIO_FRAME_RA), cpu.regs.read(rom_stubs::REG_RA));
        bus.write32(at(RADIO_FRAME_OLD_SP), old_sp);
        bus.write32(at(RADIO_FRAME_FIRST_CALL), first.unwrap_or(0));
        let second = if event.is_some() { recv } else { 0 };
        bus.write32(at(RADIO_FRAME_SECOND_CALL), second);
        bus.write32(at(RADIO_FRAME_LEN), packet.len() as u32);
        for (i, b) in packet.iter().enumerate() {
            bus.write8(at(RADIO_FRAME_PACKET).wrapping_add(i as u32), *b);
        }
        cpu.regs.write(rom_stubs::REG_SP, sp);
        cpu.regs.pc = RADIO_CALL_TRAMPOLINE_ADDR;
    }

    /// Puts the idle hook in the table exactly while an event waits for
    /// registered callbacks.
    fn sync_idle_hook(&mut self, cpu: &mut Cpu) {
        let want = self.wants_idle_hook();
        if want != self.idle_armed {
            cpu.set_app_hooks(table(want));
            self.idle_armed = want;
        }
    }

    fn wants_idle_hook(&self) -> bool {
        self.callbacks.is_some() && self.hci.has_events()
    }
}

/// The RSSI, in dBm, at which a linked peer's advertisements are heard:
/// a fixed strong signal, as from a badge held next to this one (the
/// human partner approved −40 at the Task 6 gate, 2026-10-30).
///
/// It must be at least −64 dBm, Connect's proximity gate in v392. The
/// receive loop at `0x4201ed4e` drops a report whose RSSI is below −64
/// (`li a4, -0x40; blt a5, a4`), and a two-badge probe paired at −64 but
/// not at −65 (`docs/milestone-10-decisions.md`, "Connect's pairing
/// exchange"). The `rssi_min` key (default −60) is Chess's, not Connect's.
pub const LINKED_PEER_RSSI: i8 = -40;

/// The length of the address header in [`advertisement_to_bytes`]'s wire
/// form: the address (6 bytes) and its type (1 byte).
pub const ADVERTISEMENT_HEADER_LEN: usize = 7;

/// An advertisement as bytes for a peer: `addr(6) ‖ addr_type(1) ‖ data`,
/// `addr` in HCI (little-endian) byte order, as [`hci::Advertisement`]
/// holds it. The host passes these bytes between badges unchanged.
pub fn advertisement_to_bytes(adv: &Advertisement) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(ADVERTISEMENT_HEADER_LEN + adv.data.len());
    bytes.extend_from_slice(&adv.addr);
    bytes.push(adv.addr_type);
    bytes.extend_from_slice(&adv.data);
    bytes
}

/// The inverse of [`advertisement_to_bytes`], for bytes from a peer (the
/// network): `None` for anything malformed, never a panic. Malformed is
/// shorter than [`ADVERTISEMENT_HEADER_LEN`], data over
/// [`hci::MAX_ADV_DATA_LEN`], or an Address_Type over 3 (7.7.65.13).
pub fn advertisement_from_bytes(bytes: &[u8]) -> Option<Advertisement> {
    if bytes.len() < ADVERTISEMENT_HEADER_LEN {
        return None;
    }
    let (header, data) = bytes.split_at(ADVERTISEMENT_HEADER_LEN);
    let addr_type = header[6];
    if data.len() > hci::MAX_ADV_DATA_LEN || addr_type > 3 {
        return None;
    }
    let mut addr = [0; 6];
    addr.copy_from_slice(&header[..6]);
    Some(Advertisement {
        addr,
        addr_type,
        data: data.to_vec(),
    })
}

/// The bt.c seam (module doc), plus the idle hook if `idle`.
fn table(idle: bool) -> AppHookTable {
    let mut table = AppHookTable::new();
    if idle {
        table.insert(v392::ESP_V_APPLICATION_IDLE_HOOK, HOOK_IDLE);
    }
    table
        .insert(v392::ESP_BT_CONTROLLER_INIT, HOOK_INIT)
        .insert(v392::ESP_BT_CONTROLLER_ENABLE, HOOK_ENABLE)
        .insert(
            v392::ESP_VHCI_HOST_REGISTER_CALLBACK,
            HOOK_REGISTER_CALLBACK,
        )
        .insert(
            v392::ESP_VHCI_HOST_CHECK_SEND_AVAILABLE,
            HOOK_CHECK_SEND_AVAILABLE,
        )
        .insert(v392::ESP_VHCI_HOST_SEND_PACKET, HOOK_SEND_PACKET);
    table
}

/// The controller's BD_ADDR, in HCI (little-endian) byte order: the
/// factory MAC with `MAC_ADDR_UNIVERSE_BT_OFFSET` (2) added to its last
/// byte, as `esp_read_mac(ESP_MAC_BT)` derives it
/// (`components/esp_hw_support/mac_addr.c` lines 17 and 392 to 396,
/// ESP-IDF v5.5.3). All zeros without a MAC. The firmware advertises from
/// its random address, so nothing on the air shows this one.
fn bd_addr(mac: Option<[u8; 6]>) -> [u8; 6] {
    let Some(mut mac) = mac else {
        return [0; 6];
    };
    mac[5] = mac[5].wrapping_add(2);
    mac.reverse();
    mac
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::{boot_from_firmware_image_with_rom_stubs, step_with_interrupts};
    use crate::cpu::encode::{addi, jal, jalr, lbu, A0, A1, RA, T3, T4, T5, ZERO};
    use crate::radio::hci::{
        LE_SET_EVENT_MASK, LE_SET_EXT_SCAN_ENABLE, LE_SET_EXT_SCAN_PARAMS, RESET, SET_EVENT_MASK,
    };

    const HACKER_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x10, 0x01];
    /// The synthetic image's code (`crate::runtime`'s test images have
    /// the same shape): an idle loop, and two VHCI callbacks that record
    /// what they were called with in `t3`, `t4` and `t5`.
    const CODE_BASE: u32 = 0x4200_0020;
    /// `j .`: where every hooked call returns to.
    const LOOP: u32 = CODE_BASE;
    /// `notify_host_send_available`: `t3 += 1; ret`.
    const SEND_AVAILABLE: u32 = CODE_BASE + 4;
    /// `notify_host_recv(data, len)`: `t4 = data[1]` (the event code),
    /// `t5 = len`; `ret`.
    const RECV: u32 = CODE_BASE + 12;
    const CODE: [u32; 6] = [
        jal(ZERO, 0),
        addi(T3, T3, 1),
        jalr(ZERO, RA, 0),
        lbu(T4, A0, 1),
        addi(T5, A1, 0),
        jalr(ZERO, RA, 0),
    ];
    /// Guest RAM the tests use: the callback struct, a command buffer and
    /// the stack.
    const CALLBACKS: u32 = 0x3fcb_0000;
    const PACKET: u32 = 0x3fcb_0100;
    const STACK_TOP: u32 = 0x3fcc_0000;

    fn image() -> Vec<u8> {
        let mut buf = vec![0u8; 24];
        buf[0] = 0xE9;
        buf[1] = 1;
        buf[4..8].copy_from_slice(&CODE_BASE.to_le_bytes());
        buf.extend_from_slice(&CODE_BASE.to_le_bytes());
        buf.extend_from_slice(&((CODE.len() * 4) as u32).to_le_bytes());
        for w in CODE {
            buf.extend_from_slice(&w.to_le_bytes());
        }
        buf
    }

    /// The most steps one idle-hook pass may take, entry to return: the
    /// hook's own step, the trampoline and the callback (measured at 14).
    /// A pass that drained the queue or never came back would exceed it.
    const IDLE_PASS_MAX_STEPS: u32 = 20;

    struct Guest {
        cpu: Cpu,
        bus: FirmwareBus,
        radio: Radio,
        /// Steps the last [`Guest::call`] took, entry to return.
        call_steps: u32,
    }

    impl Guest {
        fn new() -> Self {
            let (mut cpu, mut bus) = boot_from_firmware_image_with_rom_stubs(&image()).unwrap();
            let mut radio = Radio::new();
            radio.set_mac(Some(HACKER_MAC));
            radio.install(&mut cpu);
            bus.write32(CALLBACKS, SEND_AVAILABLE);
            bus.write32(CALLBACKS + 4, RECV);
            Self {
                cpu,
                bus,
                radio,
                call_steps: 0,
            }
        }

        /// Calls the hooked guest function at `addr` with `a0`, `a1`, as
        /// a task would, and runs until it has returned to [`LOOP`]
        /// (at most 200 steps), answering hits as the runtime does.
        fn call(&mut self, addr: u32, a0: u32, a1: u32) -> u32 {
            self.cpu.regs.write(rom_stubs::REG_A0, a0);
            self.cpu.regs.write(rom_stubs::REG_A1, a1);
            self.cpu.regs.write(rom_stubs::REG_RA, LOOP);
            self.cpu.regs.write(rom_stubs::REG_SP, STACK_TOP);
            self.cpu.regs.pc = addr;
            for step in 1..=200 {
                step_with_interrupts(&mut self.cpu, &mut self.bus);
                if self.cpu.needs_attention() {
                    self.cpu.clear_attention();
                    if let Some(hit) = self.cpu.take_hook_hit() {
                        self.radio.on_hook(hit, &mut self.cpu, &mut self.bus);
                    }
                }
                if self.cpu.regs.pc == LOOP {
                    self.call_steps = step;
                    assert_eq!(self.cpu.regs.read(rom_stubs::REG_SP), STACK_TOP);
                    return self.cpu.regs.read(rom_stubs::REG_A0);
                }
            }
            panic!("no return; pc {:#x}", self.cpu.regs.pc);
        }

        /// Brings the controller up as `nimble_port_init` and
        /// `esp_nimble_hci_init` do: init, enable, register callbacks.
        fn bring_up(&mut self) {
            assert_eq!(self.call(v392::ESP_BT_CONTROLLER_INIT, 0, 0), v392::ESP_OK);
            assert_eq!(
                self.call(v392::ESP_BT_CONTROLLER_ENABLE, 1, 0),
                v392::ESP_OK
            );
            assert_eq!(
                self.call(v392::ESP_VHCI_HOST_REGISTER_CALLBACK, CALLBACKS, 0),
                v392::ESP_OK
            );
        }

        /// Sends one H4 command through `esp_vhci_host_send_packet`.
        fn send(&mut self, opcode: u16, params: &[u8]) {
            let mut h4 = vec![0x01, opcode as u8, (opcode >> 8) as u8, params.len() as u8];
            h4.extend_from_slice(params);
            for (i, b) in h4.iter().enumerate() {
                self.bus.write8(PACKET + i as u32, *b);
            }
            self.call(v392::ESP_VHCI_HOST_SEND_PACKET, PACKET, h4.len() as u32);
        }

        fn reg(&self, r: u8) -> u32 {
            self.cpu.regs.read(r)
        }

        /// Scanning on LE 1M with the reports unmasked.
        fn start_scanning(&mut self) {
            self.send(SET_EVENT_MASK, &(1u64 << 61).to_le_bytes());
            self.send(LE_SET_EVENT_MASK, &(1u64 << 12).to_le_bytes());
            self.send(LE_SET_EXT_SCAN_PARAMS, &[0, 0, 1, 0, 0x10, 0, 0x10, 0]);
            self.send(LE_SET_EXT_SCAN_ENABLE, &[1, 0, 0, 0, 0, 0]);
        }
    }

    fn peer() -> hci::Advertisement {
        hci::Advertisement {
            addr: [0x02, 0x10, 0, 0, 0, 0xC2],
            addr_type: 1,
            data: vec![0x03, 0xFF, 0xFF, 0xFF],
        }
    }

    /// bt.c's state checks, mirrored: enable before init and init twice
    /// are `ESP_ERR_INVALID_STATE`, registering before enable is
    /// `ESP_FAIL`, and the status word in guest RAM follows IDLE, INITED,
    /// ENABLED.
    #[test]
    fn controller_hooks_follow_bt_c_state_checks() {
        let mut g = Guest::new();
        assert_eq!(
            g.call(v392::ESP_BT_CONTROLLER_ENABLE, 1, 0),
            v392::ESP_ERR_INVALID_STATE
        );
        assert_eq!(
            g.call(v392::ESP_VHCI_HOST_REGISTER_CALLBACK, CALLBACKS, 0),
            v392::ESP_FAIL
        );
        assert_eq!(g.call(v392::ESP_VHCI_HOST_CHECK_SEND_AVAILABLE, 0, 0), 0);
        assert_eq!(g.call(v392::ESP_BT_CONTROLLER_INIT, 0, 0), v392::ESP_OK);
        assert_eq!(
            g.bus.read32(v392::BTDM_CONTROLLER_STATUS),
            v392::STATUS_INITED
        );
        assert_eq!(
            g.call(v392::ESP_BT_CONTROLLER_INIT, 0, 0),
            v392::ESP_ERR_INVALID_STATE
        );
        assert_eq!(g.radio.init_count(), 1);
        assert_eq!(g.call(v392::ESP_BT_CONTROLLER_ENABLE, 1, 0), v392::ESP_OK);
        assert_eq!(
            g.bus.read32(v392::BTDM_CONTROLLER_STATUS),
            v392::STATUS_ENABLED
        );
        assert_eq!(g.call(v392::ESP_VHCI_HOST_CHECK_SEND_AVAILABLE, 0, 0), 1);
        assert_eq!(
            g.call(v392::ESP_VHCI_HOST_REGISTER_CALLBACK, CALLBACKS, 0),
            v392::ESP_OK
        );
        assert_eq!(g.radio.hook_hits(), 8);
    }

    /// A command's reply reaches the host inside the send hook, through
    /// the guest-executed trampoline: `notify_host_send_available` once,
    /// then `notify_host_recv` with the Command Complete, then back to the
    /// caller with `sp` restored ([`Guest::call`] checks it).
    #[test]
    fn send_packet_calls_back_with_the_command_complete() {
        let mut g = Guest::new();
        g.bring_up();
        g.send(RESET, &[]);
        assert_eq!(g.reg(T3), 1, "notify_host_send_available");
        assert_eq!(g.reg(T4), 0x0E, "Command Complete");
        assert_eq!(g.reg(T5), 7, "H4 indicator, header and 4 parameters");
        assert!(!g.radio.controller().has_events());
        assert!(g
            .cpu
            .app_hooks()
            .lookup(v392::ESP_V_APPLICATION_IDLE_HOOK)
            .is_none());
    }

    /// An advertisement heard while scanning reaches the host from the
    /// idle task: the idle hook joins the table only while the report
    /// waits, delivers it at the hook's entry, and leaves again; with
    /// nothing waiting, the hook is not in the table at all.
    #[test]
    fn a_report_reaches_the_host_from_the_idle_hook() {
        let mut g = Guest::new();
        g.bring_up();
        g.start_scanning();
        let idle = |g: &Guest| g.cpu.app_hooks().lookup(v392::ESP_V_APPLICATION_IDLE_HOOK);
        assert_eq!(idle(&g), None);
        assert!(g.radio.deliver(&mut g.cpu, &peer(), -40));
        assert_eq!(idle(&g), Some(HOOK_IDLE), "armed while the report waits");
        let sends = g.reg(T3);
        g.call(v392::ESP_V_APPLICATION_IDLE_HOOK, 0, 0);
        assert_eq!(g.reg(T4), 0x3E, "LE Meta");
        assert_eq!(g.reg(T5), 3 + 26 + 4, "an Extended Advertising Report");
        assert_eq!(g.reg(T3), sends, "no send credit from the idle task");
        assert_eq!(idle(&g), None, "disarmed once delivered");
        assert!(!g.radio.controller().has_events());
    }

    /// R-T6-2's ordering risk: a report is already waiting when the host
    /// sends a command, so the queue holds the report ahead of the reply.
    /// The host gets its reply inside `send_packet` regardless, in the task
    /// that sent the command (it is what gives back the command credit);
    /// the report stays queued, keeps the idle hook armed, and is delivered
    /// from the idle task afterwards.
    #[test]
    fn report_popped_in_send_packet_follows_the_reply() {
        let mut g = Guest::new();
        g.bring_up();
        g.start_scanning();
        assert!(g.radio.deliver(&mut g.cpu, &peer(), -40));
        let idle = |g: &Guest| g.cpu.app_hooks().lookup(v392::ESP_V_APPLICATION_IDLE_HOOK);
        assert_eq!(idle(&g), Some(HOOK_IDLE));
        let sends = g.reg(T3);

        g.send(SET_EVENT_MASK, &(1u64 << 61).to_le_bytes());
        assert_eq!(g.reg(T3), sends + 1, "notify_host_send_available");
        assert_eq!(g.reg(T4), 0x0E, "the reply first: Command Complete");
        assert_eq!(g.reg(T5), 7, "the reply's length");
        assert!(g.radio.controller().has_events(), "the report still waits");
        assert_eq!(idle(&g), Some(HOOK_IDLE), "still armed for the report");

        g.call(v392::ESP_V_APPLICATION_IDLE_HOOK, 0, 0);
        assert_eq!(g.reg(T4), 0x3E, "then the report: LE Meta");
        assert_eq!(g.reg(T5), 3 + 26 + 4);
        assert_eq!(g.reg(T3), sends + 1, "no send credit from the idle task");
        assert!(!g.radio.controller().has_events());
        assert_eq!(idle(&g), None, "disarmed once delivered");
    }

    /// The other half of R-T6-2's risk: the idle task must never block.
    /// While reports are queued, each idle pass delivers exactly one and
    /// comes back within a bounded number of steps, leaving the rest for
    /// later passes (the guest's `notify_host_recv` here returns at once;
    /// the firmware's own `host_rcv_pkt` queue send is outside this model).
    #[test]
    fn host_rcv_pkt_does_not_block_the_idle_task() {
        const REPORTS: u8 = 3;
        let mut g = Guest::new();
        g.bring_up();
        g.start_scanning();
        for i in 0..REPORTS {
            let mut adv = peer();
            adv.addr[0] = i;
            assert!(g.radio.deliver(&mut g.cpu, &adv, -40));
        }
        for left in (0..REPORTS).rev() {
            g.cpu.regs.write(T4, 0);
            g.call(v392::ESP_V_APPLICATION_IDLE_HOOK, 0, 0);
            assert!(
                g.call_steps <= IDLE_PASS_MAX_STEPS,
                "{} steps for one idle pass",
                g.call_steps
            );
            assert_eq!(g.reg(T4), 0x3E, "one report this pass");
            assert_eq!(
                g.radio.controller().has_events(),
                left > 0,
                "{left} reports still to go"
            );
        }
        assert_eq!(
            g.cpu.app_hooks().lookup(v392::ESP_V_APPLICATION_IDLE_HOOK),
            None
        );
    }

    /// Review Focus 2: a soft reset while the radio is up. The guest's
    /// reboot drops the core's hook table; the radio resets with it (no
    /// queued report, no scanning, no callbacks, no idle hook) and keeps
    /// its MAC and init count, and the next boot brings it up again
    /// cleanly, as on power-on.
    #[test]
    fn radio_state_resets_with_a_soft_reset() {
        let mut g = Guest::new();
        g.bring_up();
        g.start_scanning();
        assert!(g.radio.deliver(&mut g.cpu, &peer(), -40));
        assert!(g.radio.controller().has_events());
        assert!(g.radio.controller().is_scanning());

        // The runtime's soft reset: a fresh core and bus, then the radio.
        let (mut cpu, bus) = boot_from_firmware_image_with_rom_stubs(&image()).unwrap();
        g.radio.soft_reset();
        g.radio.install(&mut cpu);
        g.cpu = cpu;
        g.bus = bus;
        g.bus.write32(CALLBACKS, SEND_AVAILABLE);
        g.bus.write32(CALLBACKS + 4, RECV);

        assert!(
            !g.radio.controller().has_events(),
            "nothing queued survives"
        );
        assert!(!g.radio.controller().is_scanning());
        assert_eq!(
            g.radio.controller(),
            &HciController::new(bd_addr(Some(HACKER_MAC))),
            "a power-on controller with the same BD_ADDR"
        );
        assert_eq!(g.radio.mac(), Some(HACKER_MAC));
        assert_eq!(g.radio.init_count(), 1);
        assert!(g
            .cpu
            .app_hooks()
            .lookup(v392::ESP_V_APPLICATION_IDLE_HOOK)
            .is_none());
        assert!(g
            .cpu
            .app_hooks()
            .lookup(v392::ESP_VHCI_HOST_SEND_PACKET)
            .is_some());
        // A report delivered now has no scan to land in.
        assert!(!g.radio.deliver(&mut g.cpu, &peer(), -40));

        g.bring_up();
        assert_eq!(g.radio.init_count(), 2);
        g.send(RESET, &[]);
        assert_eq!(
            (g.reg(T3), g.reg(T4)),
            (1, 0x0E),
            "the first reply after the reset"
        );
    }

    /// The wire form for peers (`addr(6) ‖ addr_type(1) ‖ data`): an
    /// advertisement survives the round trip, and a decoded one reaches a
    /// scanning host like the original; anything too short, with data
    /// over [`hci::MAX_ADV_DATA_LEN`] or an Address_Type over 3 decodes to
    /// nothing.
    #[test]
    fn advertisement_bytes_round_trip_and_reject_malformed() {
        let adv = peer();
        let bytes = advertisement_to_bytes(&adv);
        assert_eq!(
            bytes,
            [0x02, 0x10, 0, 0, 0, 0xC2, 1, 0x03, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(advertisement_from_bytes(&bytes), Some(adv.clone()));
        let empty = hci::Advertisement {
            addr: [1, 2, 3, 4, 5, 6],
            addr_type: 0,
            data: Vec::new(),
        };
        assert_eq!(
            advertisement_from_bytes(&advertisement_to_bytes(&empty)),
            Some(empty)
        );
        let mut max = vec![0u8; 7 + hci::MAX_ADV_DATA_LEN];
        max[6] = 3;
        assert!(advertisement_from_bytes(&max).is_some());
        max.push(0);
        assert_eq!(advertisement_from_bytes(&max), None, "230 bytes of data");
        assert_eq!(advertisement_from_bytes(&bytes[..6]), None);
        assert_eq!(advertisement_from_bytes(&[]), None);
        let mut bad = bytes.clone();
        bad[6] = 4;
        assert_eq!(advertisement_from_bytes(&bad), None);

        let mut g = Guest::new();
        g.bring_up();
        g.start_scanning();
        let decoded = advertisement_from_bytes(&bytes).unwrap();
        assert!(g.radio.deliver(&mut g.cpu, &decoded, LINKED_PEER_RSSI));
        g.call(v392::ESP_V_APPLICATION_IDLE_HOOK, 0, 0);
        assert_eq!(g.reg(T4), 0x3E, "LE Meta");
    }

    /// The BD_ADDR is `esp_read_mac(ESP_MAC_BT)`'s, in HCI byte order.
    #[test]
    fn bd_addr_is_the_bt_mac_reversed() {
        assert_eq!(bd_addr(None), [0; 6]);
        assert_eq!(
            bd_addr(Some(HACKER_MAC)),
            [0x03, 0x10, 0x00, 0x00, 0x00, 0x02]
        );
    }
}
