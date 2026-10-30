//! The v392 app-image addresses the fake BLE controller hooks (Milestone
//! 10 Task 6), one constant per bt.c function, each with its evidence.
//!
//! The firmware (`frontend/public/firmware/firmware.bin`,
//! `v0.1.2-392-gd3089c4`) has no symbols, so each address was identified
//! from a disassembly of its segments (llvm-objdump) against ESP-IDF
//! v5.5.3's sources, as the spec's "Measured on v392" records:
//! `components/bt/controller/esp32c3/bt.c` (the controller wrappers),
//! `components/bt/host/nimble/esp-hci/src/esp_nimble_hci.c` (their only
//! callers), `components/freertos/FreeRTOS-Kernel/tasks.c` and
//! `components/esp_system/freertos_hooks.c` (the idle hook). The call
//! graph starts at the Milestone 9 fault: `RA 0x420806fe`, in
//! `btdm_controller_mem_init` (`0x420806f2`), called from
//! `esp_bt_controller_init` (below). Every bt.c wrapper below loads
//! [`BTDM_CONTROLLER_STATUS`] (`lui a5, 0x3fca1; lw a4, -0x764(a5)`) and
//! compares it with the `esp_bt_controller_status_t` values its source
//! tests, in source order. A firmware swap re-identifies them all.

/// `static esp_bt_controller_status_t btdm_controller_status` (bt.c line
/// 491, `DRAM_ATTR`): `lui a5, 0x3fca1; lw a4, -0x764(a5)` in every bt.c
/// wrapper below and in `esp_bt_controller_get_status` (`0x42081218`).
/// It is `.data` (initialized to IDLE), so every boot, soft resets
/// included, reloads it as IDLE.
pub const BTDM_CONTROLLER_STATUS: u32 = 0x3fca_089c;

/// `ESP_BT_CONTROLLER_STATUS_IDLE`, `_INITED`, `_ENABLED`
/// (`components/bt/include/esp32c3/include/esp_bt.h` lines 512 to 514).
pub const STATUS_IDLE: u32 = 0;
pub const STATUS_INITED: u32 = 1;
pub const STATUS_ENABLED: u32 = 2;

/// `ESP_OK`, `ESP_FAIL` and `ESP_ERR_INVALID_STATE`
/// (`components/esp_common/include/esp_err.h`), as bt.c returns them.
pub const ESP_OK: u32 = 0;
pub const ESP_FAIL: u32 = u32::MAX;
pub const ESP_ERR_INVALID_STATE: u32 = 0x103;

/// `bool esp_vhci_host_check_send_available(void)` (bt.c line 1404): loads
/// the status, `li a5, 0x2; beq` to `jal 0x42098f32`
/// (`API_vhci_host_check_send_available`), else `li a0, 0`. Its one
/// caller is `0x42087bb2`, in `esp_nimble_hci.c`'s
/// `ble_hci_trans_hs_cmd_tx`/`_acl_tx` group.
pub const ESP_VHCI_HOST_CHECK_SEND_AVAILABLE: u32 = 0x4208_0c64;

/// `void esp_vhci_host_send_packet(uint8_t *data, uint16_t len)` (bt.c
/// line 1412): saves `a0`/`a1` in `s0`/`s1`, returns at once unless the
/// status is 2, then `async_wakeup_request` and `API_vhci_host_send_packet`
/// (`0x42098f76`). Its one caller is `0x42087b8c`
/// (`esp_vhci_host_send_packet_wrapper`).
pub const ESP_VHCI_HOST_SEND_PACKET: u32 = 0x4208_0c84;

/// `esp_err_t esp_vhci_host_register_callback(const
/// esp_vhci_host_callback_t *callback)` (bt.c line 1424): `li a0, -1`
/// unless the status is 2, else `jal 0x42099048`
/// (`API_vhci_host_register_callback`), mapping non-zero to `-1`. Its one
/// caller is `0x42087d68` (`esp_nimble_hci_init`, registering
/// `vhci_host_cb = {controller_rcv_pkt_ready, host_rcv_pkt}`).
pub const ESP_VHCI_HOST_REGISTER_CALLBACK: u32 = 0x4208_0cbe;

/// `esp_err_t esp_bt_controller_init(esp_bt_controller_config_t *cfg)`
/// (bt.c line 1782): the status-IDLE check, then the config checks in
/// source order (task priority `0x17`, stack at least `0x1000`, BLE-only,
/// `ble_max_act`, sleep mode, magic `0x5a5aa5a5`), and the call to
/// `btdm_controller_mem_init` (`0x420806f2`) whose ROM call faulted in
/// Milestone 9. Called once, from `nimble_port_init` (`0x4208588e`).
pub const ESP_BT_CONTROLLER_INIT: u32 = 0x4208_0ce4;

/// `esp_err_t esp_bt_controller_enable(esp_bt_mode_t mode)` (bt.c line
/// 2031): `li s0, 0x103` unless the status is 1 (INITED). Called once,
/// from `nimble_port_init` (`0x42085898`), right after the init.
pub const ESP_BT_CONTROLLER_ENABLE: u32 = 0x4208_0fc8;

/// `void esp_vApplicationIdleHook(void)` (`freertos_hooks.c` line 41),
/// the idle task's per-pass hook: it walks the eight `idle_cb` slots
/// (`li a5, 0x7; blt a5, s0`, table `0x3fcb5704`), then, with
/// `CONFIG_PM_ENABLE`, `esp_pm_impl_idle_hook` (`0x420fcf68`) and
/// `esp_pm_impl_waiti` (`0x420fcfb0`), which calls `0x4038b8a4`, the only
/// `wfi` in the image (`0x4038b8b8`). Its one caller is `prvIdleTask`
/// (`0x40390a76`: `prvCheckTasksWaitingTermination` at `0x4039092c`, then
/// this, forever; `tasks.c` line 4284), whose address is passed with the
/// name `"IDLE"` (`0x3c133f3c`) to the task creation at `0x403926d0`.
/// The idle task's stack is `0x600` bytes (`vApplicationGetIdleTaskMemory`
/// at `0x4038fbe2`: `li a5, 0x600; sw a5, 0(s1)`).
pub const ESP_V_APPLICATION_IDLE_HOOK: u32 = 0x4212_ac42;
