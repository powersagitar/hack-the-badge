//! A BLE controller at the HCI boundary, in H4 framing (Milestone 10
//! Task 5): pure, with no guest state.
//!
//! ## Why
//!
//! The ESP32-C3's BLE controller is a closed blob calling a large mask-ROM
//! controller, which the emulator cannot run (spec
//! `docs/superpowers/specs/2026-10-22-milestone10-badge-radio-design.md`,
//! "What the firmware's radio does" and Part 1). The firmware's NimBLE
//! host talks to it only through HCI packets over ESP-IDF's VHCI
//! transport (`esp_vhci_host_send_packet` in, `notify_host_recv` out,
//! `components/bt/host/nimble/esp-hci/src/esp_nimble_hci.c`, ESP-IDF
//! v5.5.3), so the controller is replaced as a whole by this model of the
//! HCI it answers. HCI is a published standard, so every behaviour below
//! cites it. This module holds no firmware address: the hooks that feed
//! it are Task 6's `radio/v392.rs`.
//!
//! ## Sources
//!
//! Bluetooth Core Specification **v5.3** ("Core"), Vol 4 Part E (HCI)
//! unless named otherwise. Cited from the specification's text as known to
//! the author, not fetched. NimBLE's own sources (`ble_hs_startup.c` and
//! friends) sit in ESP-IDF's `nimble` submodule, which does not resolve
//! through `raw.githubusercontent.com/espressif/esp-idf/v5.5.3/…`, so the
//! command set is the task brief's list and nothing NimBLE-specific is
//! assumed beyond it. Task 6's command trace on v392 (Connect and Sync
//! opening, `docs/milestone-10-decisions.md`) added LE Rand, LE Set Address
//! Resolution Enable, LE Clear Resolving List, LE Add Device To Resolving
//! List, LE Set Privacy Mode and LE Remove Advertising Set: with those, the
//! firmware gets no Unknown HCI Command reply. The two parameter rules of
//! Milestone 11 Task 6 (7.8.56's duplicate handles, 7.8.65's
//! Filter_Duplicates 2) were read from Core **v5.4**'s HTML text, fetched
//! 2026-11-09: same section numbers.
//!
//! - **Framing.** Vol 4 Part A (UART transport) 2: H4 packet indicators
//!   `0x01` command, `0x02` ACL, `0x03` SCO, `0x04` event, `0x05` ISO.
//!   5.4.1: a command is opcode (2, little-endian; OGF in bits 15..10,
//!   OCF in 9..0), parameter length (1), parameters. 5.4.4: an event is
//!   event code (1), parameter length (1), parameters, so at most 255
//!   parameter bytes. [`HciController::host_to_controller`] takes only
//!   commands: there are no connections, so ACL, SCO and ISO packets are
//!   refused like an unknown indicator, and a command whose length byte
//!   disagrees with the packet's length (truncated or oversized) is
//!   refused unanswered.
//! - **Replies.** 4.4: every command is answered, in order, and
//!   Num_HCI_Command_Packets (`1` here) is the host's credit. 7.7.14
//!   Command Complete (`0x0E`): Num_HCI_Command_Packets, Command_Opcode,
//!   return parameters (Status first). Every command this model knows is
//!   one the Core answers with Command Complete, so none uses 7.7.15
//!   Command Status. 4.5: an unknown opcode is answered with Unknown HCI
//!   Command (`0x01`); a known one with bad parameters with Invalid HCI
//!   Command Parameters (`0x12`). An error reply carries the Status alone.
//!   Error codes are Vol 1 Part F 1.3.
//! - **Reports.** 7.7.65 LE Meta (`0x3E`), subevent 7.7.65.13 LE Extended
//!   Advertising Report (`0x0D`), gated by the event masks: 7.3.1 bit 61
//!   (LE Meta) and 7.8.1 bit 12 (subevent `0x0D`, LE mask bit = subevent
//!   minus 1).
//!
//! | Opcode | Command | Reply | Core |
//! |---|---|---|---|
//! | `0x0C01` | Set Event Mask | CC: Status | 7.3.1 |
//! | `0x0C03` | Reset | CC: Status | 7.3.2 |
//! | `0x0C63` | Set Event Mask Page 2 | CC: Status | 7.3.69 |
//! | `0x1001` | Read Local Version Information | CC: Status, versions | 7.4.1 |
//! | `0x1002` | Read Local Supported Commands | CC: Status, 64-octet mask | 7.4.2 |
//! | `0x1003` | Read Local Supported Features | CC: Status, LMP features | 7.4.3 |
//! | `0x1005` | Read Buffer Size | CC: Status, ACL/SCO sizes | 7.4.5 |
//! | `0x1009` | Read BD_ADDR | CC: Status, BD_ADDR | 7.4.6 |
//! | `0x2001` | LE Set Event Mask | CC: Status | 7.8.1 |
//! | `0x2002` | LE Read Buffer Size [v1] | CC: Status, length, count | 7.8.2 |
//! | `0x2003` | LE Read Local Supported Features | CC: Status, LE features | 7.8.3 |
//! | `0x2005` | LE Set Random Address | CC: Status | 7.8.4 |
//! | `0x2018` | LE Rand | CC: Status, Random_Number (8) | 7.8.23 |
//! | `0x2027` | LE Add Device To Resolving List | CC: Status | 7.8.38 |
//! | `0x2029` | LE Clear Resolving List | CC: Status | 7.8.40 |
//! | `0x202D` | LE Set Address Resolution Enable | CC: Status | 7.8.44 |
//! | `0x2035` | LE Set Advertising Set Random Address | CC: Status | 7.8.52 |
//! | `0x2036` | LE Set Extended Advertising Parameters [v1] | CC: Status, Selected_TX_Power | 7.8.53 |
//! | `0x2037` | LE Set Extended Advertising Data | CC: Status | 7.8.54 |
//! | `0x2038` | LE Set Extended Scan Response Data | CC: Status | 7.8.55 |
//! | `0x2039` | LE Set Extended Advertising Enable | CC: Status | 7.8.56 |
//! | `0x203A` | LE Read Maximum Advertising Data Length | CC: Status, length | 7.8.57 |
//! | `0x203B` | LE Read Number of Supported Advertising Sets | CC: Status, count | 7.8.58 |
//! | `0x203C` | LE Remove Advertising Set | CC: Status | 7.8.59 |
//! | `0x2041` | LE Set Extended Scan Parameters | CC: Status | 7.8.64 |
//! | `0x2042` | LE Set Extended Scan Enable | CC: Status | 7.8.65 |
//! | `0x204E` | LE Set Privacy Mode | CC: Status | 7.8.77 |
//! | any other | — | CC: Unknown HCI Command (`0x01`) | 4.5 |
//!
//! ## What it reports about itself
//!
//! An LE-only Core 5.0 controller (HCI and LMP version `0x09`), company
//! identifier `0xFFFF` (Assigned Numbers: reserved for internal use and
//! testing, so it claims no vendor). LMP features (Vol 2 Part C 3.3):
//! bit 37 "BR/EDR Not Supported" and bit 38 "LE Supported (Controller)".
//! LE features (Vol 6 Part B 4.6): bit 12 "LE Extended Advertising" only.
//! Supported Commands (6.27) sets exactly the bits of the table above.
//! Buffers: [`ACL_PACKET_LEN`] bytes by [`ACL_PACKET_COUNT`] packets, both
//! LE and shared, non-zero as a host requires though nothing uses them.
//! [`MAX_ADV_DATA_LEN`] is 229, the firmware's single-report limit, and
//! [`MAX_ADV_SETS`] sets.
//!
//! ## Not modeled
//!
//! Air timing (advertising intervals, scan windows, durations and their
//! Advertising Set Terminated / Scan Timeout events), PHYs other than LE
//! 1M, filter accept lists (the filter policies are validated, then
//! ignored), privacy and resolvable addresses (own address types 2 and 3
//! act as 0 and 1; the resolving list keeps identities only, so LE Set
//! Privacy Mode can answer by membership, and nothing resolves), scan
//! requests and responses (scan response data is stored, never sent),
//! connections and ACL. LE Rand is deterministic (splitmix64 over a
//! per-BD_ADDR seed, restarted by Reset), as every emulator input is.

use std::collections::VecDeque;

/// The most events [`HciController`] holds for the host. Past it the
/// oldest advertising report is dropped first; a command reply never is.
pub const MAX_QUEUED_EVENTS: usize = 32;
/// The largest advertising payload: one LE Extended Advertising Report's
/// 255 parameter bytes minus its 26 bytes of fields, the single-report
/// limit the firmware enforces.
pub const MAX_ADV_DATA_LEN: usize = 229;
/// Advertising sets the model holds (LE Read Number of Supported
/// Advertising Sets).
pub const MAX_ADV_SETS: usize = 4;
/// ACL data packet length reported by both Read Buffer Size commands.
pub const ACL_PACKET_LEN: u16 = 251;
/// ACL data packet count reported by both Read Buffer Size commands.
pub const ACL_PACKET_COUNT: u8 = 4;
/// Distinct (address, type, data) triples the duplicate filter
/// remembers; the oldest is forgotten first, as in a real controller's
/// finite filter.
pub const DUPLICATE_FILTER_LEN: usize = 64;
/// The resolving list's capacity (7.8.38: Memory Capacity Exceeded past
/// it). NimBLE adds one entry, its own identity.
pub const RESOLVING_LIST_LEN: usize = 8;

pub const SET_EVENT_MASK: u16 = 0x0C01;
pub const RESET: u16 = 0x0C03;
pub const SET_EVENT_MASK_PAGE_2: u16 = 0x0C63;
pub const READ_LOCAL_VERSION: u16 = 0x1001;
pub const READ_LOCAL_SUPPORTED_COMMANDS: u16 = 0x1002;
pub const READ_LOCAL_SUPPORTED_FEATURES: u16 = 0x1003;
pub const READ_BUFFER_SIZE: u16 = 0x1005;
pub const READ_BD_ADDR: u16 = 0x1009;
pub const LE_SET_EVENT_MASK: u16 = 0x2001;
pub const LE_READ_BUFFER_SIZE: u16 = 0x2002;
pub const LE_READ_LOCAL_SUPPORTED_FEATURES: u16 = 0x2003;
pub const LE_SET_RANDOM_ADDR: u16 = 0x2005;
pub const LE_RAND: u16 = 0x2018;
pub const LE_ADD_DEVICE_TO_RESOLVING_LIST: u16 = 0x2027;
pub const LE_CLEAR_RESOLVING_LIST: u16 = 0x2029;
pub const LE_SET_ADDR_RESOLUTION_ENABLE: u16 = 0x202D;
pub const LE_SET_ADV_SET_RANDOM_ADDR: u16 = 0x2035;
pub const LE_SET_EXT_ADV_PARAMS: u16 = 0x2036;
pub const LE_SET_EXT_ADV_DATA: u16 = 0x2037;
pub const LE_SET_EXT_SCAN_RSP_DATA: u16 = 0x2038;
pub const LE_SET_EXT_ADV_ENABLE: u16 = 0x2039;
pub const LE_READ_MAX_ADV_DATA_LEN: u16 = 0x203A;
pub const LE_READ_NUM_SUPPORTED_ADV_SETS: u16 = 0x203B;
pub const LE_REMOVE_ADV_SET: u16 = 0x203C;
pub const LE_SET_EXT_SCAN_PARAMS: u16 = 0x2041;
pub const LE_SET_EXT_SCAN_ENABLE: u16 = 0x2042;
pub const LE_SET_PRIVACY_MODE: u16 = 0x204E;

/// H4 packet indicators (Vol 4 Part A 2).
const H4_COMMAND: u8 = 0x01;
const H4_EVENT: u8 = 0x04;
/// Event codes (7.7.14, 7.7.65) and the LE subevent (7.7.65.13).
const EVT_COMMAND_COMPLETE: u8 = 0x0E;
const EVT_LE_META: u8 = 0x3E;
const SUBEVT_EXT_ADV_REPORT: u8 = 0x0D;
/// Error codes (Vol 1 Part F 1.3).
const SUCCESS: u8 = 0x00;
const UNKNOWN_COMMAND: u8 = 0x01;
const UNKNOWN_CONNECTION: u8 = 0x02;
const MEMORY_CAPACITY_EXCEEDED: u8 = 0x07;
const COMMAND_DISALLOWED: u8 = 0x0C;
const INVALID_PARAMETERS: u8 = 0x12;
const UNKNOWN_ADV_IDENTIFIER: u8 = 0x42;

/// 7.3.1: the Event Mask after Reset, and its LE Meta bit.
const DEFAULT_EVENT_MASK: u64 = 0x0000_1FFF_FFFF_FFFF;
const EVENT_MASK_LE_META: u64 = 1 << 61;
/// 7.8.1: the LE Event Mask after Reset, and the Extended Advertising
/// Report's bit (subevent `0x0D`, bit 12).
const DEFAULT_LE_EVENT_MASK: u64 = 0x0000_0000_0000_001F;
const LE_EVENT_MASK_EXT_ADV_REPORT: u64 = 1 << 12;

/// Read Local Version Information's fields (7.4.1): Core 5.0, no vendor.
const HCI_VERSION_5_0: u8 = 0x09;
const COMPANY_ID_TESTING: u16 = 0xFFFF;

/// 7.7.65.13 Event_Type for a non-connectable, non-scannable,
/// undirected extended advertisement with complete data: bits 0
/// (connectable), 1 (scannable), 2 (directed), 3 (scan response), 4
/// (legacy) all clear, Data_Status (bits 6..5) `00` complete. The model's
/// advertisements are exactly that: it answers no scan requests and makes
/// no connections.
const EXT_ADV_EVENT_TYPE: u16 = 0x0000;
/// 7.7.65.13 Primary_PHY / Secondary_PHY `0x01`: LE 1M.
const PHY_LE_1M: u8 = 0x01;
/// 7.7.65.13 TX_Power `0x7F`: not available.
const TX_POWER_NOT_AVAILABLE: u8 = 0x7F;
/// 7.7.65.13 RSSI range, -127 to +20 dBm.
const RSSI_MIN: i8 = -127;
const RSSI_MAX: i8 = 20;
/// 7.7.65.13: the report's fields before Data.
const EXT_ADV_REPORT_HEADER: usize = 26;

/// The `(octet, bit)` of every command in the table above, in 6.27's
/// Supported Commands mask.
const SUPPORTED_COMMANDS: [(usize, u8); 26] = [
    (5, 6),  // Set Event Mask
    (5, 7),  // Reset
    (14, 3), // Read Local Version Information
    (14, 5), // Read Local Supported Features
    (14, 7), // Read Buffer Size
    (15, 1), // Read BD_ADDR
    (22, 2), // Set Event Mask Page 2
    (25, 0), // LE Set Event Mask
    (25, 1), // LE Read Buffer Size [v1]
    (25, 2), // LE Read Local Supported Features
    (25, 4), // LE Set Random Address
    (27, 7), // LE Rand
    (34, 3), // LE Add Device To Resolving List
    (34, 5), // LE Clear Resolving List
    (35, 1), // LE Set Address Resolution Enable
    (36, 1), // LE Set Advertising Set Random Address
    (36, 2), // LE Set Extended Advertising Parameters
    (36, 3), // LE Set Extended Advertising Data
    (36, 4), // LE Set Extended Scan Response Data
    (36, 5), // LE Set Extended Advertising Enable
    (36, 6), // LE Read Maximum Advertising Data Length
    (36, 7), // LE Read Number of Supported Advertising Sets
    (37, 0), // LE Remove Advertising Set
    (37, 5), // LE Set Extended Scan Parameters
    (37, 6), // LE Set Extended Scan Enable
    (39, 2), // LE Set Privacy Mode
];

/// An advertisement on the air: what the firmware's controller sends
/// ([`HciController::advertisement`]) and what a peer's becomes when
/// [`HciController::deliver`]ed. `addr` is in HCI (little-endian) byte
/// order; `addr_type` is 7.7.65.13's Address_Type (0 public, 1 random);
/// `data.len()` is at most [`MAX_ADV_DATA_LEN`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advertisement {
    pub addr: [u8; 6],
    pub addr_type: u8,
    pub data: Vec<u8>,
}

/// One advertising set (7.8.53).
#[derive(Clone, Debug, PartialEq, Eq)]
struct AdvSet {
    handle: u8,
    own_addr_type: u8,
    random_addr: Option<[u8; 6]>,
    adv_data: Fragmented,
    scan_rsp: Fragmented,
    enabled: bool,
}

/// Advertising or scan response data, which may arrive in fragments
/// (7.8.54, 7.8.55).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Fragmented {
    /// `None` until a complete or last fragment arrives.
    data: Option<Vec<u8>>,
    /// Fragments received so far (Operation `0x01`, then `0x00`).
    pending: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QueuedEvent {
    /// An advertising report, which may be dropped; a command reply never
    /// is.
    report: bool,
    /// H4-framed.
    bytes: Vec<u8>,
}

/// The controller. Fed H4 commands with
/// [`host_to_controller`](Self::host_to_controller), it answers each with
/// an event read with [`pop_event`](Self::pop_event); peers' advertisements
/// arrive through [`deliver`](Self::deliver). Malformed input is refused,
/// never a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HciController {
    public_addr: [u8; 6],
    random_addr: Option<[u8; 6]>,
    event_mask: u64,
    event_mask_page_2: u64,
    le_event_mask: u64,
    /// At most [`MAX_ADV_SETS`], sorted by handle.
    adv_sets: Vec<AdvSet>,
    scan_le_1m: bool,
    scanning: bool,
    filter_duplicates: bool,
    seen: VecDeque<Advertisement>,
    events: VecDeque<QueuedEvent>,
    /// LE Rand's generator state (7.8.23), seeded from the BD_ADDR.
    rand_state: u64,
    /// LE Set Address Resolution Enable's value. It only gates the
    /// resolving-list commands (7.8.38, 7.8.40, 7.8.77): nothing resolves,
    /// since privacy is not modeled.
    addr_resolution: bool,
    /// The resolving list's identities, (address type, address); the IRKs
    /// are not kept.
    resolving_list: Vec<(u8, [u8; 6])>,
}

impl HciController {
    /// A controller after power-on, whose BD_ADDR is `public_addr` (HCI
    /// byte order, as Read BD_ADDR returns it).
    pub fn new(public_addr: [u8; 6]) -> Self {
        Self {
            public_addr,
            random_addr: None,
            event_mask: DEFAULT_EVENT_MASK,
            event_mask_page_2: 0,
            le_event_mask: DEFAULT_LE_EVENT_MASK,
            adv_sets: Vec::new(),
            // 7.8.64: scanning on LE 1M until the host says otherwise.
            scan_le_1m: true,
            scanning: false,
            filter_duplicates: false,
            seen: VecDeque::new(),
            events: VecDeque::new(),
            rand_state: rand_seed(public_addr),
            resolving_list: Vec::new(),
            addr_resolution: false,
        }
    }

    /// Takes one H4 packet from the host. A command is executed and its
    /// Command Complete queued; `false` (nothing executed, nothing queued)
    /// for anything but a well-framed command, or when the queue is full
    /// of command replies, which only a host ignoring its command credit
    /// (4.4) can cause.
    pub fn host_to_controller(&mut self, h4: &[u8]) -> bool {
        let [H4_COMMAND, lo, hi, len, params @ ..] = h4 else {
            return false;
        };
        if params.len() != usize::from(*len) {
            return false;
        }
        if self.events.len() >= MAX_QUEUED_EVENTS && !self.events.iter().any(|e| e.report) {
            return false;
        }
        let opcode = u16::from_le_bytes([*lo, *hi]);
        let ret = self.execute(opcode, params);
        let mut ev = vec![1];
        ev.extend_from_slice(&opcode.to_le_bytes());
        ev.extend_from_slice(&ret);
        self.push(event(EVT_COMMAND_COMPLETE, &ev), false)
    }

    /// The next event for the host, H4-framed: `0x04`, event code,
    /// parameter length, parameters.
    pub fn pop_event(&mut self) -> Option<Vec<u8>> {
        self.events.pop_front().map(|e| e.bytes)
    }

    /// The oldest waiting command reply, H4-framed as [`Self::pop_event`];
    /// reports queued ahead of it stay queued. The hook that handed a
    /// command to the model delivers this one (`crate::radio`'s module
    /// doc), so a report never takes the place of the command's reply.
    pub fn pop_reply(&mut self) -> Option<Vec<u8>> {
        let i = self.events.iter().position(|e| !e.report)?;
        self.events.remove(i).map(|e| e.bytes)
    }

    /// Whether an event is waiting for the host ([`Self::pop_event`]).
    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    /// What the controller is advertising: the lowest-handle set that is
    /// enabled and has its data set.
    pub fn advertisement(&self) -> Option<Advertisement> {
        let set = self
            .adv_sets
            .iter()
            .find(|s| s.enabled && s.adv_data.data.is_some())?;
        let (addr, addr_type) = if set.own_addr_type & 1 == 0 {
            (self.public_addr, 0)
        } else {
            (set.random_addr.or(self.random_addr).unwrap_or([0; 6]), 1)
        };
        Some(Advertisement {
            addr,
            addr_type,
            data: set.adv_data.data.clone().unwrap_or_default(),
        })
    }

    /// Hears a peer's advertisement at `rssi` dBm (clamped to 7.7.65.13's
    /// -127..=20). `true` if it was queued as an LE Extended Advertising
    /// Report; `false` if the controller is not scanning on LE 1M, the
    /// event masks exclude the report, the duplicate filter suppresses it,
    /// the advertisement is malformed (data over [`MAX_ADV_DATA_LEN`],
    /// Address_Type over 3), or the queue is full of command replies.
    /// Nothing refused is held for later.
    pub fn deliver(&mut self, adv: &Advertisement, rssi: i8) -> bool {
        if !self.scanning || !self.scan_le_1m {
            return false;
        }
        if adv.data.len() > MAX_ADV_DATA_LEN || adv.addr_type > 3 {
            return false;
        }
        if self.event_mask & EVENT_MASK_LE_META == 0
            || self.le_event_mask & LE_EVENT_MASK_EXT_ADV_REPORT == 0
        {
            return false;
        }
        if self.filter_duplicates && self.seen.contains(adv) {
            return false;
        }
        let mut p = Vec::with_capacity(EXT_ADV_REPORT_HEADER + adv.data.len());
        p.push(SUBEVT_EXT_ADV_REPORT);
        p.push(1); // Num_Reports
        p.extend_from_slice(&EXT_ADV_EVENT_TYPE.to_le_bytes());
        p.push(adv.addr_type);
        p.extend_from_slice(&adv.addr);
        p.push(PHY_LE_1M); // Primary_PHY
        p.push(PHY_LE_1M); // Secondary_PHY
        p.push(0); // Advertising_SID
        p.push(TX_POWER_NOT_AVAILABLE);
        p.push(rssi.clamp(RSSI_MIN, RSSI_MAX) as u8);
        p.extend_from_slice(&[0, 0]); // Periodic_Advertising_Interval
        p.push(0); // Direct_Address_Type
        p.extend_from_slice(&[0; 6]); // Direct_Address
        p.push(adv.data.len() as u8);
        p.extend_from_slice(&adv.data);
        debug_assert_eq!(p.len(), EXT_ADV_REPORT_HEADER + adv.data.len());
        if !self.push(event(EVT_LE_META, &p), true) {
            return false;
        }
        if self.filter_duplicates {
            if self.seen.len() >= DUPLICATE_FILTER_LEN {
                self.seen.pop_front();
            }
            self.seen.push_back(adv.clone());
        }
        true
    }

    /// Whether LE Set Extended Scan Enable has scanning on.
    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    /// Queues an event. When full, the oldest advertising report makes
    /// room; with none to drop, the new event is refused (`false`).
    fn push(&mut self, bytes: Vec<u8>, report: bool) -> bool {
        if self.events.len() >= MAX_QUEUED_EVENTS {
            match self.events.iter().position(|e| e.report) {
                Some(i) => {
                    self.events.remove(i);
                }
                None => return false,
            }
        }
        self.events.push_back(QueuedEvent { report, bytes });
        true
    }

    /// Runs one command; returns its return parameters, Status first.
    fn execute(&mut self, opcode: u16, p: &[u8]) -> Vec<u8> {
        match opcode {
            SET_EVENT_MASK => mask(p).map_or(status(INVALID_PARAMETERS), |m| {
                self.event_mask = m;
                status(SUCCESS)
            }),
            RESET => {
                if !p.is_empty() {
                    return status(INVALID_PARAMETERS);
                }
                // 7.3.2: the controller returns to its power-on state; the
                // reply is the first event after it.
                *self = Self::new(self.public_addr);
                status(SUCCESS)
            }
            SET_EVENT_MASK_PAGE_2 => mask(p).map_or(status(INVALID_PARAMETERS), |m| {
                self.event_mask_page_2 = m;
                status(SUCCESS)
            }),
            READ_LOCAL_VERSION => read(p, || {
                let mut r = vec![HCI_VERSION_5_0];
                r.extend_from_slice(&0u16.to_le_bytes()); // HCI_Subversion
                r.push(HCI_VERSION_5_0); // LMP_Version
                r.extend_from_slice(&COMPANY_ID_TESTING.to_le_bytes());
                r.extend_from_slice(&0u16.to_le_bytes()); // LMP_Subversion
                r
            }),
            READ_LOCAL_SUPPORTED_COMMANDS => read(p, || {
                let mut r = vec![0u8; 64];
                for (octet, bit) in SUPPORTED_COMMANDS {
                    r[octet] |= 1 << bit;
                }
                r
            }),
            READ_LOCAL_SUPPORTED_FEATURES => read(p, || {
                // Bits 37 (BR/EDR Not Supported) and 38 (LE Supported).
                vec![0, 0, 0, 0, 0x60, 0, 0, 0]
            }),
            READ_BUFFER_SIZE => read(p, || {
                let mut r = ACL_PACKET_LEN.to_le_bytes().to_vec();
                r.push(0); // Synchronous_Data_Packet_Length
                r.extend_from_slice(&u16::from(ACL_PACKET_COUNT).to_le_bytes());
                r.extend_from_slice(&0u16.to_le_bytes()); // Total_Num_Synchronous
                r
            }),
            READ_BD_ADDR => {
                let addr = self.public_addr;
                read(p, || addr.to_vec())
            }
            LE_SET_EVENT_MASK => mask(p).map_or(status(INVALID_PARAMETERS), |m| {
                self.le_event_mask = m;
                status(SUCCESS)
            }),
            LE_READ_BUFFER_SIZE => read(p, || {
                let mut r = ACL_PACKET_LEN.to_le_bytes().to_vec();
                r.push(ACL_PACKET_COUNT);
                r
            }),
            LE_READ_LOCAL_SUPPORTED_FEATURES => read(p, || {
                // Bit 12: LE Extended Advertising.
                vec![0, 0x10, 0, 0, 0, 0, 0, 0]
            }),
            LE_SET_RANDOM_ADDR => {
                let Ok(addr) = <[u8; 6]>::try_from(p) else {
                    return status(INVALID_PARAMETERS);
                };
                // 7.8.4: disallowed while scanning.
                if self.scanning {
                    return status(COMMAND_DISALLOWED);
                }
                self.random_addr = Some(addr);
                status(SUCCESS)
            }
            LE_RAND => {
                // 7.8.23 asks for a random number from the controller's
                // generator; the emulator's must be deterministic (D-M5-3's
                // reasoning), so it is splitmix64 over a per-controller
                // seed, restarted by Reset.
                let state = &mut self.rand_state;
                read(p, || splitmix64(state).to_le_bytes().to_vec())
            }
            LE_ADD_DEVICE_TO_RESOLVING_LIST | LE_CLEAR_RESOLVING_LIST | LE_SET_PRIVACY_MODE => {
                self.resolving_list_command(opcode, p)
            }
            LE_REMOVE_ADV_SET => {
                let [handle] = p else {
                    return status(INVALID_PARAMETERS);
                };
                match self.adv_sets.iter().position(|s| s.handle == *handle) {
                    None => status(UNKNOWN_ADV_IDENTIFIER),
                    Some(i) if self.adv_sets[i].enabled => status(COMMAND_DISALLOWED),
                    Some(i) => {
                        self.adv_sets.remove(i);
                        status(SUCCESS)
                    }
                }
            }
            LE_SET_ADDR_RESOLUTION_ENABLE => match p {
                [enable @ (0 | 1)] => {
                    self.addr_resolution = *enable == 1;
                    status(SUCCESS)
                }
                _ => status(INVALID_PARAMETERS),
            },
            LE_SET_ADV_SET_RANDOM_ADDR => {
                let [handle, a @ ..] = p else {
                    return status(INVALID_PARAMETERS);
                };
                let Ok(addr) = <[u8; 6]>::try_from(a) else {
                    return status(INVALID_PARAMETERS);
                };
                match self.adv_set_mut(*handle) {
                    Some(set) => {
                        set.random_addr = Some(addr);
                        status(SUCCESS)
                    }
                    None => status(UNKNOWN_ADV_IDENTIFIER),
                }
            }
            LE_SET_EXT_ADV_PARAMS => self.set_ext_adv_params(p),
            LE_SET_EXT_ADV_DATA => self.set_ext_adv_data(p, false),
            LE_SET_EXT_SCAN_RSP_DATA => self.set_ext_adv_data(p, true),
            LE_SET_EXT_ADV_ENABLE => self.set_ext_adv_enable(p),
            LE_READ_MAX_ADV_DATA_LEN => {
                read(p, || (MAX_ADV_DATA_LEN as u16).to_le_bytes().to_vec())
            }
            LE_READ_NUM_SUPPORTED_ADV_SETS => read(p, || vec![MAX_ADV_SETS as u8]),
            LE_SET_EXT_SCAN_PARAMS => self.set_ext_scan_params(p),
            LE_SET_EXT_SCAN_ENABLE => self.set_ext_scan_enable(p),
            _ => status(UNKNOWN_COMMAND),
        }
    }

    /// 7.8.38 LE Add Device To Resolving List (identity address type 0 or
    /// 1, address, peer IRK, local IRK: 39 octets), 7.8.40 LE Clear
    /// Resolving List (none) and 7.8.77 LE Set Privacy Mode (identity
    /// address type, address, mode 0 or 1: 8 octets). All three are
    /// disallowed while address resolution is on and advertising or
    /// scanning is. The list holds identities only: nothing resolves,
    /// since privacy is not modeled (R-T5-8).
    fn resolving_list_command(&mut self, opcode: u16, p: &[u8]) -> Vec<u8> {
        let well_formed = match opcode {
            LE_CLEAR_RESOLVING_LIST => p.is_empty(),
            LE_ADD_DEVICE_TO_RESOLVING_LIST => p.len() == 39 && p[0] <= 1,
            _ => p.len() == 8 && p[0] <= 1 && p[7] <= 1,
        };
        if !well_formed {
            return status(INVALID_PARAMETERS);
        }
        let busy = self.scanning || self.adv_sets.iter().any(|s| s.enabled);
        if self.addr_resolution && busy {
            return status(COMMAND_DISALLOWED);
        }
        if opcode == LE_CLEAR_RESOLVING_LIST {
            self.resolving_list.clear();
            return status(SUCCESS);
        }
        let mut addr = [0u8; 6];
        addr.copy_from_slice(&p[1..7]);
        let listed = self.resolving_list.contains(&(p[0], addr));
        match opcode {
            LE_ADD_DEVICE_TO_RESOLVING_LIST if listed => status(INVALID_PARAMETERS),
            LE_ADD_DEVICE_TO_RESOLVING_LIST if self.resolving_list.len() >= RESOLVING_LIST_LEN => {
                status(MEMORY_CAPACITY_EXCEEDED)
            }
            LE_ADD_DEVICE_TO_RESOLVING_LIST => {
                self.resolving_list.push((p[0], addr));
                status(SUCCESS)
            }
            _ if listed => status(SUCCESS),
            _ => status(UNKNOWN_CONNECTION),
        }
    }

    fn adv_set_mut(&mut self, handle: u8) -> Option<&mut AdvSet> {
        self.adv_sets.iter_mut().find(|s| s.handle == handle)
    }

    /// 7.8.53: Advertising_Handle (1), Advertising_Event_Properties (2),
    /// Primary_Advertising_Interval_Min/Max (3 + 3),
    /// Primary_Advertising_Channel_Map (1), Own_Address_Type (1),
    /// Peer_Address_Type (1), Peer_Address (6),
    /// Advertising_Filter_Policy (1), Advertising_TX_Power (1),
    /// Primary_Advertising_PHY (1), Secondary_Advertising_Max_Skip (1),
    /// Secondary_Advertising_PHY (1), Advertising_SID (1),
    /// Scan_Request_Notification_Enable (1): 25 octets. Creates the set if
    /// it is new.
    fn set_ext_adv_params(&mut self, p: &[u8]) -> Vec<u8> {
        if p.len() != 25 {
            return status(INVALID_PARAMETERS);
        }
        let (handle, own_addr_type, tx_power) = (p[0], p[10], p[19]);
        if handle > 0xEF || own_addr_type > 3 {
            return status(INVALID_PARAMETERS);
        }
        match self.adv_set_mut(handle) {
            // Disallowed while that set advertises.
            Some(set) if set.enabled => return status(COMMAND_DISALLOWED),
            Some(set) => set.own_addr_type = own_addr_type,
            None => {
                if self.adv_sets.len() >= MAX_ADV_SETS {
                    return status(MEMORY_CAPACITY_EXCEEDED);
                }
                self.adv_sets.push(AdvSet {
                    handle,
                    own_addr_type,
                    random_addr: None,
                    adv_data: Fragmented::default(),
                    scan_rsp: Fragmented::default(),
                    enabled: false,
                });
                self.adv_sets.sort_by_key(|s| s.handle);
            }
        }
        // Selected_TX_Power: the request, or 0 dBm for "no preference".
        let selected = if tx_power == 0x7F { 0 } else { tx_power };
        vec![SUCCESS, selected]
    }

    /// 7.8.54 / 7.8.55: Advertising_Handle (1), Operation (1),
    /// Fragment_Preference (1), Data_Length (1), Data. Operation `0x00`
    /// intermediate, `0x01` first, `0x02` last fragment, `0x03` complete,
    /// `0x04` unchanged (advertising data only). Only complete data or
    /// "unchanged" while the set advertises (`0x0C` otherwise); more than
    /// [`MAX_ADV_DATA_LEN`] in all is `0x07` and discards the fragments.
    fn set_ext_adv_data(&mut self, p: &[u8], scan_rsp: bool) -> Vec<u8> {
        let [handle, op, _pref, len, data @ ..] = p else {
            return status(INVALID_PARAMETERS);
        };
        if data.len() != usize::from(*len) || *op > 0x04 || (scan_rsp && *op == 0x04) {
            return status(INVALID_PARAMETERS);
        }
        let Some(set) = self.adv_set_mut(*handle) else {
            return status(UNKNOWN_ADV_IDENTIFIER);
        };
        if set.enabled && *op < 0x03 {
            return status(COMMAND_DISALLOWED);
        }
        if *op == 0x04 {
            // Unchanged: no data, and only for a set that has some.
            return status(if data.is_empty() && set.adv_data.data.is_some() {
                SUCCESS
            } else {
                INVALID_PARAMETERS
            });
        }
        let target = if scan_rsp {
            &mut set.scan_rsp
        } else {
            &mut set.adv_data
        };
        let mut whole = match op {
            0x01 | 0x03 => Vec::new(),
            _ => match target.pending.take() {
                Some(v) => v,
                None => return status(INVALID_PARAMETERS),
            },
        };
        whole.extend_from_slice(data);
        if whole.len() > MAX_ADV_DATA_LEN {
            return status(MEMORY_CAPACITY_EXCEEDED);
        }
        if *op == 0x00 || *op == 0x01 {
            target.pending = Some(whole);
        } else {
            target.data = Some(whole);
        }
        status(SUCCESS)
    }

    /// 7.8.56: Enable (1), Num_Sets (1), then per set
    /// Advertising_Handle (1), Duration (2), Max_Extended_Advertising_Events
    /// (1). Disable with Num_Sets 0 disables every set. A set named by more
    /// than one entry is Invalid HCI Command Parameters, enabling or
    /// disabling (Core v5.4 7.8.56's wording; checked first, as there).
    /// Every handle is checked before any set changes. Duration and the
    /// event count are not modeled: a set advertises until disabled.
    fn set_ext_adv_enable(&mut self, p: &[u8]) -> Vec<u8> {
        let [enable, n, sets @ ..] = p else {
            return status(INVALID_PARAMETERS);
        };
        if *enable > 1 || sets.len() != usize::from(*n) * 4 || (*enable == 1 && *n == 0) {
            return status(INVALID_PARAMETERS);
        }
        if *n == 0 {
            for set in &mut self.adv_sets {
                set.enabled = false;
            }
            return status(SUCCESS);
        }
        let handles: Vec<u8> = sets.as_chunks::<4>().0.iter().map(|c| c[0]).collect();
        if (1..handles.len()).any(|i| handles[..i].contains(&handles[i])) {
            return status(INVALID_PARAMETERS);
        }
        for &h in &handles {
            let Some(set) = self.adv_sets.iter().find(|s| s.handle == h) else {
                return status(UNKNOWN_ADV_IDENTIFIER);
            };
            // A random own address needs one to send (7.8.56).
            if *enable == 1
                && set.own_addr_type & 1 == 1
                && set.random_addr.or(self.random_addr).is_none()
            {
                return status(INVALID_PARAMETERS);
            }
        }
        for h in handles {
            if let Some(set) = self.adv_set_mut(h) {
                set.enabled = *enable == 1;
            }
        }
        status(SUCCESS)
    }

    /// 7.8.64: Own_Address_Type (1), Scanning_Filter_Policy (1),
    /// Scanning_PHYs (1; bit 0 LE 1M, bit 2 LE Coded), then per PHY
    /// Scan_Type (1), Scan_Interval (2), Scan_Window (2). Disallowed while
    /// scanning.
    fn set_ext_scan_params(&mut self, p: &[u8]) -> Vec<u8> {
        let [own, policy, phys, per_phy @ ..] = p else {
            return status(INVALID_PARAMETERS);
        };
        if *own > 3 || *policy > 3 || *phys == 0 || *phys & !0b101 != 0 {
            return status(INVALID_PARAMETERS);
        }
        if per_phy.len() != phys.count_ones() as usize * 5
            || per_phy.as_chunks::<5>().0.iter().any(|c| c[0] > 1)
        {
            return status(INVALID_PARAMETERS);
        }
        if self.scanning {
            return status(COMMAND_DISALLOWED);
        }
        self.scan_le_1m = *phys & 1 == 1;
        status(SUCCESS)
    }

    /// 7.8.65: Enable (1), Filter_Duplicates (1; 0 off, 1 on, 2 on and
    /// reset each period, here as 1), Duration (2), Period (2). Enabling
    /// resets the duplicate filter, so a repeat is reported again once
    /// scanning is re-enabled. Duration and Period are not modeled, but
    /// Filter_Duplicates 2 needs both non-zero or is Invalid HCI Command
    /// Parameters, and only when enabling: with Enable 0 "the remaining
    /// parameters shall be ignored" (Core v5.4 7.8.65's wording).
    fn set_ext_scan_enable(&mut self, p: &[u8]) -> Vec<u8> {
        let &[enable, filter, ..] = p else {
            return status(INVALID_PARAMETERS);
        };
        if p.len() != 6 || enable > 1 || filter > 2 {
            return status(INVALID_PARAMETERS);
        }
        let (duration, period) = (&p[2..4], &p[4..6]);
        if enable == 1 && filter == 2 && (duration == [0, 0] || period == [0, 0]) {
            return status(INVALID_PARAMETERS);
        }
        self.scanning = enable == 1;
        if self.scanning {
            self.filter_duplicates = filter != 0;
            self.seen.clear();
        }
        status(SUCCESS)
    }
}

/// An H4 event packet: indicator, code, parameter length, parameters.
/// Every caller's parameters fit 255 bytes.
fn event(code: u8, params: &[u8]) -> Vec<u8> {
    debug_assert!(params.len() <= 255);
    let mut e = vec![H4_EVENT, code, params.len() as u8];
    e.extend_from_slice(params);
    e
}

fn status(s: u8) -> Vec<u8> {
    vec![s]
}

/// An 8-octet event mask, little-endian (7.3.1, 7.3.69, 7.8.1).
fn mask(p: &[u8]) -> Option<u64> {
    <[u8; 8]>::try_from(p).ok().map(u64::from_le_bytes)
}

/// A parameterless read: Status then `f`'s return parameters.
fn read(p: &[u8], f: impl FnOnce() -> Vec<u8>) -> Vec<u8> {
    if !p.is_empty() {
        return status(INVALID_PARAMETERS);
    }
    let mut r = status(SUCCESS);
    r.extend(f());
    r
}

/// LE Rand's seed: the BD_ADDR, so two controllers differ and one
/// repeats.
fn rand_seed(addr: [u8; 6]) -> u64 {
    let mut b = [0u8; 8];
    b[..6].copy_from_slice(&addr);
    u64::from_le_bytes(b) ^ 0x4C45_5F52_414E_4421
}

/// splitmix64 (Steele, Lea and Flood, "Fast splittable pseudorandom
/// number generators", 2014): one step of LE Rand's generator.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::collection::vec;
    use proptest::prelude::*;
    use proptest::sample::select;
    use proptest::test_runner::{Config, RngAlgorithm, RngSeed, TestRunner};
    use std::cell::RefCell;
    use std::collections::HashSet;

    /// The hacker test MAC (common context), in HCI (little-endian) order.
    const PUBLIC: [u8; 6] = [0x01, 0x10, 0x00, 0x00, 0x00, 0x02];
    /// A random static address (top two bits of the last byte set).
    const RANDOM: [u8; 6] = [0x01, 0x10, 0x00, 0x00, 0x00, 0xC2];
    const PEER: [u8; 6] = [0x02, 0x10, 0x00, 0x00, 0x00, 0xC2];

    fn cmd(opcode: u16, params: &[u8]) -> Vec<u8> {
        let mut p = vec![0x01, opcode as u8, (opcode >> 8) as u8, params.len() as u8];
        p.extend_from_slice(params);
        p
    }

    /// Sends one command and returns its Command Complete's return
    /// parameters, checking the framing and the echoed opcode.
    fn send(c: &mut HciController, opcode: u16, params: &[u8]) -> Vec<u8> {
        assert!(c.host_to_controller(&cmd(opcode, params)));
        let ev = c.pop_event().expect("a reply");
        assert_eq!(ev[0], 0x04, "H4 event packet");
        assert_eq!(ev[1], 0x0E, "Command Complete");
        assert_eq!(ev[2] as usize, ev.len() - 3, "parameter length");
        assert_eq!(ev[3], 1, "Num_HCI_Command_Packets");
        assert_eq!(u16::from_le_bytes([ev[4], ev[5]]), opcode);
        ev[6..].to_vec()
    }

    fn send_ok(c: &mut HciController, opcode: u16, params: &[u8]) -> Vec<u8> {
        let ret = send(c, opcode, params);
        assert_eq!(ret[0], 0x00, "opcode {opcode:#06x} status");
        ret
    }

    fn adv_params(handle: u8, own_addr_type: u8) -> Vec<u8> {
        let mut p = vec![handle, 0x00, 0x00]; // non-connectable, non-scannable
        p.extend_from_slice(&[0xA0, 0x00, 0x00, 0xA0, 0x00, 0x00]); // 100 ms
        p.push(0x07); // channel map
        p.push(own_addr_type);
        p.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0]); // peer
        p.push(0x00); // filter policy
        p.push(0x7F); // tx power: no preference
        p.extend_from_slice(&[0x01, 0x00, 0x01, 0x00, 0x00]); // 1M, skip, 1M, SID, notif
        assert_eq!(p.len(), 25);
        p
    }

    fn adv_data(handle: u8, data: &[u8]) -> Vec<u8> {
        let mut p = vec![handle, 0x03, 0x01, data.len() as u8];
        p.extend_from_slice(data);
        p
    }

    fn start_advertising(c: &mut HciController, data: &[u8]) {
        send_ok(c, LE_SET_EXT_ADV_PARAMS, &adv_params(0, 1));
        let mut a = vec![0];
        a.extend_from_slice(&RANDOM);
        send_ok(c, LE_SET_ADV_SET_RANDOM_ADDR, &a);
        send_ok(c, LE_SET_EXT_ADV_DATA, &adv_data(0, data));
        send_ok(c, LE_SET_EXT_ADV_ENABLE, &[1, 1, 0, 0, 0, 0]);
    }

    fn start_scanning(c: &mut HciController, filter_duplicates: u8) {
        send_ok(c, SET_EVENT_MASK, &[0xFF; 8]);
        send_ok(c, LE_SET_EVENT_MASK, &[0xFF; 8]);
        // Own type random, accept all, 1M only: passive, 10 ms / 10 ms.
        send_ok(
            c,
            LE_SET_EXT_SCAN_PARAMS,
            &[1, 0, 0x01, 0, 0x10, 0, 0x10, 0],
        );
        send_ok(
            c,
            LE_SET_EXT_SCAN_ENABLE,
            &[1, filter_duplicates, 0, 0, 0, 0],
        );
    }

    fn peer_adv(data: &[u8]) -> Advertisement {
        Advertisement {
            addr: PEER,
            addr_type: 1,
            data: data.to_vec(),
        }
    }

    #[test]
    fn reset_answers_command_complete_success() {
        let mut c = HciController::new(PUBLIC);
        assert_eq!(send(&mut c, RESET, &[]), vec![0x00]);
        assert_eq!(c.pop_event(), None);
        // The startup reads answer too, in order, each with its return
        // parameters (Vol 4 Part E 7.4.x, 7.8.x).
        assert_eq!(send_ok(&mut c, READ_BD_ADDR, &[])[1..], PUBLIC);
        assert_eq!(send_ok(&mut c, READ_LOCAL_VERSION, &[]).len(), 9);
        assert_eq!(
            send_ok(&mut c, READ_LOCAL_SUPPORTED_COMMANDS, &[]).len(),
            65
        );
        assert_eq!(send_ok(&mut c, READ_LOCAL_SUPPORTED_FEATURES, &[]).len(), 9);
        let le_feat = send_ok(&mut c, LE_READ_LOCAL_SUPPORTED_FEATURES, &[]);
        assert_eq!(le_feat.len(), 9);
        assert_ne!(le_feat[2] & 0x10, 0, "LE Extended Advertising (bit 12)");
        let le_buf = send_ok(&mut c, LE_READ_BUFFER_SIZE, &[]);
        assert_ne!(u16::from_le_bytes([le_buf[1], le_buf[2]]), 0);
        assert_ne!(le_buf[3], 0);
        let buf = send_ok(&mut c, READ_BUFFER_SIZE, &[]);
        assert_eq!(buf.len(), 8);
        assert_ne!(u16::from_le_bytes([buf[1], buf[2]]), 0);
        assert_ne!(u16::from_le_bytes([buf[4], buf[5]]), 0);
        send_ok(&mut c, SET_EVENT_MASK, &[0xFF; 8]);
        send_ok(&mut c, SET_EVENT_MASK_PAGE_2, &[0; 8]);
        send_ok(&mut c, LE_SET_EVENT_MASK, &[0xFF; 8]);
        send_ok(&mut c, LE_SET_RANDOM_ADDR, &RANDOM);
        assert_eq!(
            send_ok(&mut c, LE_READ_MAX_ADV_DATA_LEN, &[]),
            vec![0, MAX_ADV_DATA_LEN as u8, 0]
        );
        assert_eq!(
            send_ok(&mut c, LE_READ_NUM_SUPPORTED_ADV_SETS, &[]),
            vec![0, MAX_ADV_SETS as u8]
        );
        // A Reset after setup clears it and still answers success.
        start_advertising(&mut c, &[0x02, 0x01, 0x06]);
        assert!(c.advertisement().is_some());
        assert_eq!(send(&mut c, RESET, &[]), vec![0x00]);
        assert_eq!(c.advertisement(), None);
    }

    #[test]
    fn unknown_opcode_answers_unknown_command() {
        let mut c = HciController::new(PUBLIC);
        // A vendor command (OGF 0x3F) and an unassigned LE command.
        assert_eq!(send(&mut c, 0xFC01, &[1, 2, 3]), vec![0x01]);
        assert_eq!(send(&mut c, 0x20FF, &[]), vec![0x01]);
        // A known command with the wrong parameter length is invalid, not
        // unknown (Vol 4 Part E 4.5; error 0x12).
        assert_eq!(send(&mut c, RESET, &[0]), vec![0x12]);
        assert_eq!(send(&mut c, LE_SET_RANDOM_ADDR, &[0; 5]), vec![0x12]);
        assert_eq!(c.pop_event(), None);
    }

    #[test]
    fn ext_adv_enable_exposes_the_advertisement() {
        let mut c = HciController::new(PUBLIC);
        let data = [0x02, 0x01, 0x06, 0x03, 0xFF, 0xAB, 0xCD];
        send_ok(&mut c, LE_SET_EXT_ADV_PARAMS, &adv_params(0, 1));
        let mut a = vec![0];
        a.extend_from_slice(&RANDOM);
        send_ok(&mut c, LE_SET_ADV_SET_RANDOM_ADDR, &a);
        // Enabled without data: nothing to expose yet.
        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[1, 1, 0, 0, 0, 0]);
        assert_eq!(c.advertisement(), None);
        send_ok(&mut c, LE_SET_EXT_ADV_DATA, &adv_data(0, &data));
        assert_eq!(
            c.advertisement(),
            Some(Advertisement {
                addr: RANDOM,
                addr_type: 1,
                data: data.to_vec(),
            })
        );
        // Own address type public advertises the public address.
        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[0, 0]);
        send_ok(&mut c, LE_SET_EXT_ADV_PARAMS, &adv_params(0, 0));
        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[1, 1, 0, 0, 0, 0]);
        let adv = c.advertisement().unwrap();
        assert_eq!((adv.addr, adv.addr_type), (PUBLIC, 0));
        // Data over the 229-byte single-report limit is refused (0x07),
        // and the set keeps its data.
        assert_eq!(
            send(&mut c, LE_SET_EXT_ADV_DATA, &adv_data(0, &[0; 230])),
            vec![0x07]
        );
        assert_eq!(c.advertisement().unwrap().data, data);
        send_ok(&mut c, LE_SET_EXT_ADV_DATA, &adv_data(0, &[0x5A; 229]));
        assert_eq!(c.advertisement().unwrap().data.len(), 229);
        // Fragments are refused while the set advertises (0x0C), and
        // joined (first, then last) while it does not.
        assert_eq!(
            send(&mut c, LE_SET_EXT_ADV_DATA, &[0, 0x01, 0x01, 1, 0xAA]),
            vec![0x0C]
        );
        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[0, 0]);
        send_ok(&mut c, LE_SET_EXT_ADV_DATA, &[0, 0x01, 0x01, 2, 0xAA, 0xBB]);
        send_ok(&mut c, LE_SET_EXT_ADV_DATA, &[0, 0x02, 0x01, 1, 0xCC]);
        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[1, 1, 0, 0, 0, 0]);
        assert_eq!(c.advertisement().unwrap().data, vec![0xAA, 0xBB, 0xCC]);
        // An unknown set is an Unknown Advertising Identifier (0x42).
        assert_eq!(
            send(&mut c, LE_SET_EXT_ADV_DATA, &adv_data(3, &data)),
            vec![0x42]
        );
        assert_eq!(
            send(&mut c, LE_SET_EXT_ADV_ENABLE, &[1, 1, 3, 0, 0, 0]),
            vec![0x42]
        );
    }

    #[test]
    fn ext_adv_disable_clears_it() {
        let mut c = HciController::new(PUBLIC);
        start_advertising(&mut c, &[0x02, 0x01, 0x06]);
        assert!(c.advertisement().is_some());
        // Disable the one set by handle.
        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[0, 1, 0, 0, 0, 0]);
        assert_eq!(c.advertisement(), None);
        // Enable again, then disable all sets (Num_Sets 0).
        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[1, 1, 0, 0, 0, 0]);
        assert!(c.advertisement().is_some());
        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[0, 0]);
        assert_eq!(c.advertisement(), None);
        // A Reset takes the set away entirely.
        send_ok(&mut c, RESET, &[]);
        assert_eq!(
            send(&mut c, LE_SET_EXT_ADV_ENABLE, &[1, 1, 0, 0, 0, 0]),
            vec![0x42]
        );
        assert_eq!(c.advertisement(), None);
    }

    #[test]
    fn scan_enable_turns_delivery_into_ext_adv_reports() {
        let mut c = HciController::new(PUBLIC);
        assert!(!c.is_scanning());
        start_scanning(&mut c, 0);
        assert!(c.is_scanning());
        let data = [0x02, 0x01, 0x06, 0x03, 0xFF, 0x12, 0x34];
        assert!(c.deliver(&peer_adv(&data), -40));
        let ev = c.pop_event().unwrap();
        // H4 event, LE Meta (0x3E), length.
        assert_eq!(ev[..2], [0x04, 0x3E]);
        assert_eq!(ev[2] as usize, ev.len() - 3);
        let p = &ev[3..];
        assert_eq!(p[0], 0x0D, "Subevent_Code: LE Extended Advertising Report");
        assert_eq!(p[1], 1, "Num_Reports");
        assert_eq!(u16::from_le_bytes([p[2], p[3]]), 0x0000, "Event_Type");
        assert_eq!(p[4], 1, "Address_Type");
        assert_eq!(p[5..11], PEER, "Address");
        assert_eq!(p[11], 0x01, "Primary_PHY: LE 1M");
        assert_eq!(p[12], 0x01, "Secondary_PHY: LE 1M");
        assert_eq!(p[13], 0x00, "Advertising_SID");
        assert_eq!(p[14], 0x7F, "TX_Power: not available");
        assert_eq!(p[15] as i8, -40, "RSSI");
        assert_eq!(
            u16::from_le_bytes([p[16], p[17]]),
            0,
            "Periodic_Advertising_Interval"
        );
        assert_eq!(p[18], 0, "Direct_Address_Type");
        assert_eq!(p[19..25], [0; 6], "Direct_Address");
        assert_eq!(p[25] as usize, data.len(), "Data_Length");
        assert_eq!(p[26..], data, "Data");
        assert_eq!(c.pop_event(), None);

        // A full 229-byte payload fills the 255-byte parameter limit.
        assert!(c.deliver(&peer_adv(&[0xEE; 229]), -60));
        let ev = c.pop_event().unwrap();
        assert_eq!(ev[2], 255);
        assert_eq!(ev.len(), 258);
        // Over the limit, or a bad address type, is rejected.
        assert!(!c.deliver(&peer_adv(&[0xEE; 230]), -60));
        let mut bad = peer_adv(&data);
        bad.addr_type = 7;
        assert!(!c.deliver(&bad, -60));

        // Filter duplicates off: a repeat is reported again.
        assert!(c.deliver(&peer_adv(&data), -40));
        assert!(c.deliver(&peer_adv(&data), -41));
        assert_eq!(
            (c.pop_event().is_some(), c.pop_event().is_some()),
            (true, true)
        );

        // Filter duplicates on: a repeat is suppressed, new data is not, and
        // re-enabling scanning resets the filter.
        send_ok(&mut c, LE_SET_EXT_SCAN_ENABLE, &[1, 1, 0, 0, 0, 0]);
        assert!(c.deliver(&peer_adv(&data), -40));
        assert!(!c.deliver(&peer_adv(&data), -45));
        assert!(c.deliver(&peer_adv(&[0x01]), -45));
        assert_eq!(
            (c.pop_event().is_some(), c.pop_event().is_some()),
            (true, true)
        );
        send_ok(&mut c, LE_SET_EXT_SCAN_ENABLE, &[0, 0, 0, 0, 0, 0]);
        send_ok(&mut c, LE_SET_EXT_SCAN_ENABLE, &[1, 1, 0, 0, 0, 0]);
        assert!(c.deliver(&peer_adv(&data), -40));
        assert!(c.pop_event().is_some());

        // The event masks gate the report (Vol 4 Part E 7.3.1, 7.8.1).
        send_ok(&mut c, LE_SET_EVENT_MASK, &[0x1F, 0, 0, 0, 0, 0, 0, 0]);
        assert!(!c.deliver(&peer_adv(&[0x02]), -40));
    }

    #[test]
    fn advertisement_without_scanning_is_dropped() {
        let mut c = HciController::new(PUBLIC);
        // Masks open, parameters set, but scanning never enabled.
        send_ok(&mut c, SET_EVENT_MASK, &[0xFF; 8]);
        send_ok(&mut c, LE_SET_EVENT_MASK, &[0xFF; 8]);
        send_ok(
            &mut c,
            LE_SET_EXT_SCAN_PARAMS,
            &[1, 0, 0x01, 0, 0x10, 0, 0x10, 0],
        );
        assert!(!c.deliver(&peer_adv(&[0x02, 0x01, 0x06]), -40));
        assert_eq!(c.pop_event(), None);
        // Enabled then disabled: dropped again, and nothing was held back
        // for later.
        start_scanning(&mut c, 0);
        send_ok(&mut c, LE_SET_EXT_SCAN_ENABLE, &[0, 0, 0, 0, 0, 0]);
        assert!(!c.is_scanning());
        assert!(!c.deliver(&peer_adv(&[0x02, 0x01, 0x06]), -40));
        send_ok(&mut c, LE_SET_EXT_SCAN_ENABLE, &[1, 0, 0, 0, 0, 0]);
        assert_eq!(c.pop_event(), None);
        // A Reset stops scanning too.
        send_ok(&mut c, RESET, &[]);
        assert!(!c.is_scanning());
        assert!(!c.deliver(&peer_adv(&[0x02, 0x01, 0x06]), -40));
        assert_eq!(c.pop_event(), None);
    }

    #[test]
    fn event_queue_is_bounded() {
        let mut c = HciController::new(PUBLIC);
        start_scanning(&mut c, 0);
        // Reports past the bound drop the oldest report.
        for i in 0..(MAX_QUEUED_EVENTS + 8) {
            assert!(c.deliver(&peer_adv(&[i as u8]), -40));
            assert!(c.events.len() <= MAX_QUEUED_EVENTS);
        }
        assert_eq!(c.events.len(), MAX_QUEUED_EVENTS);
        // A command reply into a full queue drops the oldest report, never
        // a reply.
        assert!(c.host_to_controller(&cmd(READ_BD_ADDR, &[])));
        assert_eq!(c.events.len(), MAX_QUEUED_EVENTS);
        let first = c.pop_event().unwrap();
        assert_eq!(first[3..5], [0x0D, 1]);
        assert_eq!(first[29], 9, "reports 0..8 were dropped, oldest first");
        // Drain: 30 more reports, then the reply, last.
        let mut rest = Vec::new();
        while let Some(ev) = c.pop_event() {
            rest.push(ev);
        }
        assert_eq!(rest.len(), MAX_QUEUED_EVENTS - 1);
        assert_eq!(rest.last().unwrap()[1], 0x0E);

        // A queue full of replies: a report is dropped, and a further
        // command is refused unexecuted (no reply is owed for it).
        for _ in 0..MAX_QUEUED_EVENTS {
            assert!(c.host_to_controller(&cmd(READ_BD_ADDR, &[])));
        }
        assert!(!c.deliver(&peer_adv(&[0x01]), -40));
        assert!(!c.host_to_controller(&cmd(LE_SET_EXT_SCAN_ENABLE, &[0, 0, 0, 0, 0, 0])));
        assert!(c.is_scanning(), "the refused command did not run");
        assert_eq!(c.events.len(), MAX_QUEUED_EVENTS);
        assert!(c.events.iter().all(|e| !e.report && e.bytes[1] == 0x0E));
    }

    #[test]
    fn oversized_or_truncated_packets_are_rejected() {
        let mut c = HciController::new(PUBLIC);
        assert!(!c.host_to_controller(&[]));
        assert!(!c.host_to_controller(&[0x01]));
        assert!(!c.host_to_controller(&[0x01, 0x03, 0x0C]));
        // Length says 2, one byte follows; length says 0, one extra byte.
        assert!(!c.host_to_controller(&[0x01, 0x01, 0x10, 2, 0]));
        assert!(!c.host_to_controller(&[0x01, 0x03, 0x0C, 0, 0]));
        // Unknown or unsupported H4 packet types: none answer.
        assert!(!c.host_to_controller(&[0x00, 0x03, 0x0C, 0]));
        assert!(!c.host_to_controller(&[0x02, 0x00, 0x00, 0x00, 0x00]));
        assert!(!c.host_to_controller(&[0x04, 0x0E, 0x00]));
        assert!(!c.host_to_controller(&[0xFF; 300]));
        assert_eq!(c.pop_event(), None);
        // A well-framed one still works afterwards.
        assert_eq!(send(&mut c, RESET, &[]), vec![0x00]);
    }

    /// Core v5.4, Vol 4 Part E, 7.8.56: the same set named by more than one
    /// entry is Invalid HCI Command Parameters, for enabling and disabling
    /// alike (the sentence is before "The remainder of this section only
    /// applies if Enable is set to 0x01"), and nothing changes.
    #[test]
    fn ext_adv_enable_rejects_duplicate_handles() {
        let mut c = HciController::new(PUBLIC);
        start_advertising(&mut c, &[0x02, 0x01, 0x06]);
        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[0, 0]);
        let enable_twice = [1, 2, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(send(&mut c, LE_SET_EXT_ADV_ENABLE, &enable_twice), [0x12]);
        assert_eq!(
            c.advertisement(),
            None,
            "the refused enable changed nothing"
        );

        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[1, 1, 0, 0, 0, 0]);
        let disable_twice = [0, 2, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(send(&mut c, LE_SET_EXT_ADV_ENABLE, &disable_twice), [0x12]);
        assert!(
            c.advertisement().is_some(),
            "the refused disable changed nothing"
        );

        // The duplicate rule comes first: an unknown handle named twice is
        // 0x12, not 7.8.56's Unknown Advertising Identifier.
        let unknown_twice = [1, 2, 9, 0, 0, 0, 9, 0, 0, 0];
        assert_eq!(send(&mut c, LE_SET_EXT_ADV_ENABLE, &unknown_twice), [0x12]);

        // Duplicates anywhere in the array, not only next to each other.
        send_ok(&mut c, LE_SET_EXT_ADV_PARAMS, &adv_params(1, 0));
        let apart = [1, 3, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(send(&mut c, LE_SET_EXT_ADV_ENABLE, &apart), [0x12]);

        // Two different sets are not duplicates.
        let two_sets = [1, 2, 0, 0, 0, 0, 1, 0, 0, 0];
        assert_eq!(send(&mut c, LE_SET_EXT_ADV_ENABLE, &two_sets), [SUCCESS]);
    }

    /// Core v5.4, Vol 4 Part E, 7.8.65: Filter_Duplicates `0x02` "shall only
    /// be used if both Period and Duration are non-zero", else Invalid HCI
    /// Command Parameters; with Enable `0x00` "the remaining parameters
    /// shall be ignored".
    #[test]
    fn ext_scan_enable_rejects_filter_duplicates_2_with_period_0() {
        let mut c = HciController::new(PUBLIC);
        for (name, enable) in [
            ("period 0", [1, 2, 1, 0, 0, 0]),
            ("duration 0", [1, 2, 0, 0, 2, 0]),
            ("both 0", [1, 2, 0, 0, 0, 0]),
        ] {
            assert_eq!(
                send(&mut c, LE_SET_EXT_SCAN_ENABLE, &enable),
                [0x12],
                "{name}"
            );
            assert!(!c.is_scanning(), "{name}: refused, so not scanning");
        }

        // Both non-zero is the accepted form; a refusal later leaves that
        // scan running.
        send_ok(&mut c, LE_SET_EXT_SCAN_ENABLE, &[1, 2, 1, 0, 2, 0]);
        assert!(c.is_scanning());
        assert_eq!(
            send(&mut c, LE_SET_EXT_SCAN_ENABLE, &[1, 2, 1, 0, 0, 0]),
            [0x12]
        );
        assert!(c.is_scanning(), "the refused command changed nothing");

        // Disabling ignores the rest.
        send_ok(&mut c, LE_SET_EXT_SCAN_ENABLE, &[0, 2, 0, 0, 0, 0]);
        assert!(!c.is_scanning());
        // The other filter values have no such rule.
        for filter in [0, 1] {
            send_ok(&mut c, LE_SET_EXT_SCAN_ENABLE, &[1, filter, 0, 0, 0, 0]);
        }
    }

    /// The proptest cases of the HCI fuzz test; each is 1 to 128 operations.
    const HCI_FUZZ_CASES: u32 = 512;

    const KNOWN: [u16; 27] = [
        RESET,
        LE_SET_PRIVACY_MODE,
        LE_ADD_DEVICE_TO_RESOLVING_LIST,
        LE_CLEAR_RESOLVING_LIST,
        LE_REMOVE_ADV_SET,
        LE_RAND,
        LE_SET_ADDR_RESOLUTION_ENABLE,
        SET_EVENT_MASK,
        SET_EVENT_MASK_PAGE_2,
        READ_LOCAL_VERSION,
        READ_LOCAL_SUPPORTED_COMMANDS,
        READ_LOCAL_SUPPORTED_FEATURES,
        READ_BUFFER_SIZE,
        READ_BD_ADDR,
        LE_SET_EVENT_MASK,
        LE_READ_BUFFER_SIZE,
        LE_READ_LOCAL_SUPPORTED_FEATURES,
        LE_SET_RANDOM_ADDR,
        LE_SET_ADV_SET_RANDOM_ADDR,
        LE_SET_EXT_ADV_PARAMS,
        LE_SET_EXT_ADV_DATA,
        LE_SET_EXT_SCAN_RSP_DATA,
        LE_SET_EXT_ADV_ENABLE,
        LE_READ_MAX_ADV_DATA_LEN,
        LE_READ_NUM_SUPPORTED_ADV_SETS,
        LE_SET_EXT_SCAN_PARAMS,
        LE_SET_EXT_SCAN_ENABLE,
    ];

    /// Task 6's trace: NimBLE's startup asks for 8 random octets (LE Rand,
    /// for its local IRK) right after Read BD_ADDR. The answer is
    /// deterministic for a given BD_ADDR and number of calls, changes from
    /// call to call, and starts over after Reset.
    #[test]
    fn le_rand_returns_eight_deterministic_octets() {
        let mut c = HciController::new(PUBLIC);
        let first = send_ok(&mut c, LE_RAND, &[]);
        assert_eq!(first.len(), 9, "Status + Random_Number[8]");
        let second = send_ok(&mut c, LE_RAND, &[]);
        assert_ne!(first, second, "a new number each call");
        let mut twin = HciController::new(PUBLIC);
        assert_eq!(send_ok(&mut twin, LE_RAND, &[]), first, "deterministic");
        send_ok(&mut c, RESET, &[]);
        assert_eq!(send_ok(&mut c, LE_RAND, &[]), first, "Reset starts over");
        let mut other = HciController::new(PEER);
        assert_ne!(send_ok(&mut other, LE_RAND, &[]), first, "per controller");
        assert_eq!(send(&mut c, LE_RAND, &[0])[..], [INVALID_PARAMETERS]);
    }

    /// Task 6's trace: NimBLE's startup turns address resolution off
    /// (`0x202D`, Address_Resolution_Enable `0x00`). Privacy is not
    /// modeled (R-T5-8): both values succeed, anything else is invalid.
    #[test]
    fn le_set_address_resolution_enable_accepts_on_and_off() {
        let mut c = HciController::new(PUBLIC);
        assert_eq!(send(&mut c, LE_SET_ADDR_RESOLUTION_ENABLE, &[0]), [SUCCESS]);
        assert_eq!(send(&mut c, LE_SET_ADDR_RESOLUTION_ENABLE, &[1]), [SUCCESS]);
        for bad in [&[2u8][..], &[], &[0, 0]] {
            assert_eq!(
                send(&mut c, LE_SET_ADDR_RESOLUTION_ENABLE, bad),
                [INVALID_PARAMETERS],
                "{bad:?}"
            );
        }
    }

    /// Task 6's trace: NimBLE's startup clears the resolving list
    /// (`0x2029`) after turning address resolution off. Privacy is not
    /// modeled, so the list only ever holds the identities added to it
    /// (R-T6-1); 7.8.40 disallows the command only while address
    /// resolution is on and advertising or scanning is.
    #[test]
    fn le_clear_resolving_list_succeeds_unless_resolution_is_busy() {
        let mut c = HciController::new(PUBLIC);
        assert_eq!(send(&mut c, LE_CLEAR_RESOLVING_LIST, &[]), [SUCCESS]);
        assert_eq!(
            send(&mut c, LE_CLEAR_RESOLVING_LIST, &[0]),
            [INVALID_PARAMETERS]
        );
        send_ok(&mut c, LE_SET_ADDR_RESOLUTION_ENABLE, &[1]);
        start_advertising(&mut c, &[0x02, 0x01, 0x06]);
        assert_eq!(
            send(&mut c, LE_CLEAR_RESOLVING_LIST, &[]),
            [COMMAND_DISALLOWED]
        );
        send_ok(&mut c, LE_SET_ADDR_RESOLUTION_ENABLE, &[0]);
        assert_eq!(send(&mut c, LE_CLEAR_RESOLVING_LIST, &[]), [SUCCESS]);
    }

    /// Task 6's trace: Connect's home screen stops advertising and removes
    /// its set (`0x203C`, handle 0). 7.8.59: an unknown handle is Unknown
    /// Advertising Identifier, an enabled set is Command Disallowed, and a
    /// removed set no longer advertises or accepts data.
    #[test]
    fn le_remove_advertising_set_removes_a_disabled_set() {
        let mut c = HciController::new(PUBLIC);
        assert_eq!(
            send(&mut c, LE_REMOVE_ADV_SET, &[0]),
            [UNKNOWN_ADV_IDENTIFIER]
        );
        start_advertising(&mut c, &[0x02, 0x01, 0x06]);
        assert_eq!(send(&mut c, LE_REMOVE_ADV_SET, &[0]), [COMMAND_DISALLOWED]);
        assert!(c.advertisement().is_some());
        send_ok(&mut c, LE_SET_EXT_ADV_ENABLE, &[0, 1, 0, 0, 0, 0]);
        assert_eq!(send(&mut c, LE_REMOVE_ADV_SET, &[0]), [SUCCESS]);
        assert_eq!(c.advertisement(), None);
        assert_eq!(
            send(
                &mut c,
                LE_SET_EXT_ADV_DATA,
                &adv_data(0, &[0x02, 0x01, 0x06])
            ),
            [UNKNOWN_ADV_IDENTIFIER]
        );
        assert_eq!(send(&mut c, LE_REMOVE_ADV_SET, &[]), [INVALID_PARAMETERS]);
        // The handle is free again.
        send_ok(&mut c, LE_SET_EXT_ADV_PARAMS, &adv_params(0, 1));
    }

    /// Task 6's trace: with address resolution on, NimBLE adds its own
    /// identity and IRK to the resolving list (`0x2027`, 39 parameter
    /// octets). Privacy is not modeled (R-T5-8): a well-formed entry is
    /// accepted and its identity kept (the IRKs are not; R-T6-1); 7.8.38 disallows the command while address
    /// resolution is on and advertising or scanning is.
    #[test]
    fn le_add_device_to_resolving_list_accepts_a_well_formed_entry() {
        let mut c = HciController::new(PUBLIC);
        let mut entry = vec![0x01];
        entry.extend_from_slice(&RANDOM);
        entry.extend_from_slice(&[0x11; 16]); // Peer_IRK
        entry.extend_from_slice(&[0x22; 16]); // Local_IRK
        assert_eq!(entry.len(), 39);
        assert_eq!(
            send(&mut c, LE_ADD_DEVICE_TO_RESOLVING_LIST, &entry),
            [SUCCESS]
        );
        assert_eq!(
            send(&mut c, LE_ADD_DEVICE_TO_RESOLVING_LIST, &entry[..38]),
            [INVALID_PARAMETERS]
        );
        let mut bad_type = entry.clone();
        bad_type[0] = 0x02;
        assert_eq!(
            send(&mut c, LE_ADD_DEVICE_TO_RESOLVING_LIST, &bad_type),
            [INVALID_PARAMETERS]
        );
        send_ok(&mut c, LE_SET_ADDR_RESOLUTION_ENABLE, &[1]);
        let mut peer = entry.clone();
        peer[1..7].copy_from_slice(&PEER);
        assert_eq!(
            send(&mut c, LE_ADD_DEVICE_TO_RESOLVING_LIST, &peer),
            [SUCCESS]
        );
        start_advertising(&mut c, &[0x02, 0x01, 0x06]);
        let mut third = entry.clone();
        third[0] = 0x00;
        assert_eq!(
            send(&mut c, LE_ADD_DEVICE_TO_RESOLVING_LIST, &third),
            [COMMAND_DISALLOWED]
        );
        assert_eq!(
            send(&mut c, LE_CLEAR_RESOLVING_LIST, &[]),
            [COMMAND_DISALLOWED]
        );
    }

    /// Task 6's trace: after adding its own identity, NimBLE sets that
    /// identity's privacy mode (`0x204E`). 7.8.77: a device not on the
    /// resolving list is Unknown Connection Identifier (`0x02`); modes are
    /// 0 (network) and 1 (device). Clear Resolving List forgets entries,
    /// and adding the same identity twice is invalid (7.8.38).
    #[test]
    fn le_set_privacy_mode_needs_the_device_on_the_resolving_list() {
        let mut c = HciController::new(PUBLIC);
        let mut entry = vec![0x01];
        entry.extend_from_slice(&RANDOM);
        entry.extend_from_slice(&[0x11; 32]);
        let mut mode = vec![0x01];
        mode.extend_from_slice(&RANDOM);
        mode.push(0x01);
        assert_eq!(
            send(&mut c, LE_SET_PRIVACY_MODE, &mode),
            [UNKNOWN_CONNECTION]
        );
        send_ok(&mut c, LE_ADD_DEVICE_TO_RESOLVING_LIST, &entry);
        assert_eq!(
            send(&mut c, LE_ADD_DEVICE_TO_RESOLVING_LIST, &entry),
            [INVALID_PARAMETERS],
            "already listed"
        );
        assert_eq!(send(&mut c, LE_SET_PRIVACY_MODE, &mode), [SUCCESS]);
        let mut bad = mode.clone();
        bad[7] = 2;
        assert_eq!(
            send(&mut c, LE_SET_PRIVACY_MODE, &bad),
            [INVALID_PARAMETERS]
        );
        assert_eq!(
            send(&mut c, LE_SET_PRIVACY_MODE, &mode[..7]),
            [INVALID_PARAMETERS]
        );
        send_ok(&mut c, LE_CLEAR_RESOLVING_LIST, &[]);
        assert_eq!(
            send(&mut c, LE_SET_PRIVACY_MODE, &mode),
            [UNKNOWN_CONNECTION]
        );
    }

    /// One thing the host or a peer does to the controller.
    #[derive(Clone, Debug)]
    enum FuzzOp {
        /// H4 packets from the host, in order.
        Packets(Vec<Vec<u8>>),
        /// A peer's advertisement, heard at an RSSI.
        Hear(Advertisement, i8),
        /// The host reads every waiting event.
        Drain,
    }

    /// Pure noise, any length up to 300.
    fn noise() -> impl Strategy<Value = FuzzOp> {
        vec(any::<u8>(), 0..=300).prop_map(|p| FuzzOp::Packets(vec![p]))
    }

    /// A well-framed command with a known opcode and random parameters, so
    /// the handlers see garbage.
    fn garbage_command() -> impl Strategy<Value = FuzzOp> {
        (select(&KNOWN[..]), vec(any::<u8>(), 0..40))
            .prop_map(|(op, params)| FuzzOp::Packets(vec![cmd(op, &params)]))
    }

    /// A command frame whose length field may lie.
    fn lying_frame() -> impl Strategy<Value = FuzzOp> {
        (any::<u16>(), any::<u8>(), vec(any::<u8>(), 0..8)).prop_map(|(op, len, tail)| {
            let mut p = cmd(op, &[]);
            p[3] = len;
            p.extend(tail);
            FuzzOp::Packets(vec![p])
        })
    }

    /// The packets that turn scanning on, so the report path is reached:
    /// both event masks with the report's bits set, parameters on LE 1M,
    /// and the enable. Everything else is drawn.
    fn open_scanning() -> impl Strategy<Value = FuzzOp> {
        (
            (any::<u64>(), any::<u64>()),
            (0u8..=3, 0u8..=3, 0u8..=1),
            (0u8..=2, any::<[u8; 4]>()),
        )
            .prop_map(|((mask, le_mask), (own, policy, kind), (filter, times))| {
                let [d0, d1, p0, p1] = times;
                FuzzOp::Packets(vec![
                    cmd(SET_EVENT_MASK, &(mask | EVENT_MASK_LE_META).to_le_bytes()),
                    cmd(
                        LE_SET_EVENT_MASK,
                        &(le_mask | LE_EVENT_MASK_EXT_ADV_REPORT).to_le_bytes(),
                    ),
                    cmd(
                        LE_SET_EXT_SCAN_PARAMS,
                        &[own, policy, 0x01, kind, 0x10, 0, 0x10, 0],
                    ),
                    cmd(LE_SET_EXT_SCAN_ENABLE, &[1, filter, d0, d1, p0, p1]),
                ])
            })
    }

    /// The packets that start one set advertising, with a public own
    /// address so that no random address is needed. Up to 251 data bytes,
    /// some of them over [`MAX_ADV_DATA_LEN`].
    fn open_advertising() -> impl Strategy<Value = FuzzOp> {
        (0u8..=5, vec(any::<u8>(), 0..=251)).prop_map(|(h, data)| {
            FuzzOp::Packets(vec![
                cmd(LE_SET_EXT_ADV_PARAMS, &adv_params(h, 0)),
                cmd(LE_SET_EXT_ADV_DATA, &adv_data(h, &data)),
                cmd(LE_SET_EXT_ADV_ENABLE, &[1, 1, h, 0, 0, 0]),
            ])
        })
    }

    /// A peer's advertisement with every address byte drawn. Address type
    /// 4 and data over [`MAX_ADV_DATA_LEN`] are malformed.
    fn hear() -> impl Strategy<Value = FuzzOp> {
        (
            any::<[u8; 6]>(),
            0u8..=4,
            vec(any::<u8>(), 0..240),
            any::<i8>(),
        )
            .prop_map(|(addr, addr_type, data, rssi)| {
                FuzzOp::Hear(
                    Advertisement {
                        addr,
                        addr_type,
                        data,
                    },
                    rssi,
                )
            })
    }

    /// What the generated cases reached, over the whole run.
    #[derive(Default)]
    struct FuzzCoverage {
        /// Reports queued by `deliver`.
        reports: u32,
        /// Reports whose address is not one byte repeated.
        mixed_addrs: u32,
        /// The values seen at each address byte, in reports.
        addr_bytes: [HashSet<u8>; 6],
        /// Times the queue was seen at [`MAX_QUEUED_EVENTS`].
        full_queues: u32,
        /// Times a set was advertising.
        advertising: u32,
    }

    /// Whole H4 packets and peers' advertisements, in weighted random
    /// order, from a fixed seed (`rng_seed` and `rng_algorithm`, so every
    /// run is the same one; no failure file is written). A hostile packet
    /// may turn scanning off again, so [`open_scanning`] stays in the mix.
    #[test]
    fn fuzzed_packets_and_advertisements_stay_bounded() {
        let ops = vec(
            prop_oneof![
                2 => noise(),
                4 => garbage_command(),
                2 => lying_frame(),
                1 => open_scanning(),
                1 => open_advertising(),
                8 => hear(),
                1 => Just(FuzzOp::Drain),
            ],
            1..=128,
        );
        let mut runner = TestRunner::new(Config {
            cases: HCI_FUZZ_CASES,
            failure_persistence: None,
            rng_algorithm: RngAlgorithm::ChaCha,
            rng_seed: RngSeed::Fixed(0x1BAD_B002),
            ..Config::default()
        });
        let coverage = RefCell::new(FuzzCoverage::default());
        let outcome = runner.run(&ops, |ops| {
            let cov = &mut *coverage.borrow_mut();
            let mut c = HciController::new(PUBLIC);
            for (i, op) in ops.into_iter().enumerate() {
                match op {
                    FuzzOp::Packets(packets) => {
                        for packet in packets {
                            c.host_to_controller(&packet);
                        }
                    }
                    FuzzOp::Hear(adv, rssi) => {
                        if c.deliver(&adv, rssi) {
                            let report = &c.events.back().expect("just queued").bytes;
                            assert_eq!(report[..2], [0x04, 0x3E], "op {i}");
                            assert_eq!(report[8..14], adv.addr, "op {i}");
                            cov.reports += 1;
                            cov.mixed_addrs +=
                                u32::from(adv.addr.iter().any(|b| *b != adv.addr[0]));
                            for (seen, byte) in cov.addr_bytes.iter_mut().zip(adv.addr) {
                                seen.insert(byte);
                            }
                        }
                    }
                    FuzzOp::Drain => {
                        while let Some(ev) = c.pop_event() {
                            assert_eq!(ev[0], 0x04, "op {i}");
                            assert_eq!(ev[2] as usize + 3, ev.len(), "op {i}");
                        }
                    }
                }
                assert!(c.events.len() <= MAX_QUEUED_EVENTS, "op {i}");
                cov.full_queues += u32::from(c.events.len() == MAX_QUEUED_EVENTS);
                if let Some(a) = c.advertisement() {
                    assert!(a.data.len() <= MAX_ADV_DATA_LEN, "op {i}");
                    cov.advertising += 1;
                }
            }
            Ok(())
        });
        outcome.unwrap();
        let cov = coverage.into_inner();
        // A floor, not `> 0`: hostile LE Set Event Mask packets alone open
        // the masks often enough for a handful of reports.
        assert!(cov.reports > 1_000, "the fuzz reached the report path");
        assert!(cov.mixed_addrs > 0, "more than one byte value per address");
        for (i, seen) in cov.addr_bytes.iter().enumerate() {
            assert!(seen.len() > 1, "address byte {i} varied");
        }
        assert!(cov.full_queues > 0, "the fuzz filled the queue");
        assert!(cov.advertising > 0, "the fuzz reached an advertising set");
    }
}
