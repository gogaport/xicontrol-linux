# XiControl Linux

Native Linux control center, background daemon, KDE Plasma tray app, CLI, and DKMS WMI driver for **Xiaomi Book Pro 14 (`TM2424`)** and compatible Bitland MIFS Xiaomi/Redmi laptops.

> Ported and adapted for Linux from [Oksion/XiControl](https://github.com/Oksion/XiControl) (MIFS WMI protocol & BLTP7853 haptic touchpad research).

---

## Features

- **Performance Profiles (`xiaomi_mifs` DKMS):**
  - Direct EC control (`Eco`, `Quiet`, `Auto`, `Turbo`, `FullSpeed`, plus `Balance` on models whose firmware supports it) via `/dev/xiaomi_mifs` and `/sys/kernel/xiaomi_mifs/`.
  - Automatic AC/Battery profile switching and smart battery-mode awareness (on `TM2424` battery power, the EC firmware only retains `Eco` and `Auto`; `Quiet` / `Turbo` / `FullSpeed` require AC power and are safely blocked from battery cycle/fail loops).
  - Customizable cycle list (`Fn+K` / WMI key / tray / CLI).
- **Battery Care & Keep Awake:**
  - EC charge thresholds (`40%`, `50%`, `60%`, `70%`, `80%`, `100%`), **Travel Mode** (temporarily 100% without losing your saved limit), and **ChargeGuard** (periodic + post-suspend re-verification).
  - Optional webhook (`POST` JSON via `curl`) on charge limit changes.
  - **Owl Mode** (`systemd-inhibit` wrapper to prevent idle/sleep/lid-switch).
- **Haptic Touchpad (`BLTP7853`, I2C + hidraw + evdev/uinput):**
  - Click vibration presets (`Low`, `Medium`, `High`), click pressure threshold (`100` / `125` / `140`), edge-swipe click strength (`0x58`), hardware heavy-press 2nd click (`0x59`), and test motor pulse (`0x32`).
  - Software edge sliders from `hidraw` (left edge: screen brightness, right edge: PipeWire volume) with haptic feedback ticks.
  - Software heavy-press action trigger with live pressure meter.
  - **Bottom Dead Zone (`evdev` + `uinput`)**: palm rejection strip at the bottom of the touchpad that drops new touches inside the dead zone while preserving drags started above and physical clicks; uses matching device identity (`BLTP7853:00 347D:7853 Touchpad`, `phys=i2c-BLTP7853:00`) so KDE libinput settings (`NaturalScroll`, acceleration, click method) stay intact.
  - KWin input toggles for Touchpad & Touchscreen.
- **Display & OLED Protection (KDE Wayland):**
  - Refresh rate (`60 Hz` / `120 Hz`), resolution presets (`3120×2080 2x`, `1920×1280 1.25x`), **Auto-Hz** on AC/Battery switch, and **Hold-Hz** (restores expected Hz if externally reset).
  - Per-power brightness memory and **OLED brightness cap** (yields for 2 hours if raised manually).
- **Keys, OSD, Monitor, Tray & HTTP API:**
  - Hardware WMI key listener (`Redmibook WMI keys`) + `xicontrol trigger <slot>` for KDE Global Shortcuts + KDE Plasma OSD notifications.
  - Real-time 60s telemetry charts for CPU (`coretemp` / RAPL W), Intel Arc iGPU (`xe`), NPU (`intel_vpu`), RAM, and Battery power.
  - Two KDE StatusNotifier tray icons (status badge + optional live numeric metric badge).
  - Optional local/LAN HTTP API (`GET /status`, `POST /mode|care|travel|owl`) with SHA-256 Bearer token auth.

---

## Requirements

- **OS:** Linux (tested on Arch / CachyOS with kernel `7.3.4-1-cachyos`)
- **Desktop Environment:** KDE Plasma 6 (Wayland) — uses `kscreen-doctor`, `qdbus6`, `org.kde.KWin`, `org.kde.Solid.PowerManagement`, `org.kde.plasmashell`
- **Audio:** PipeWire (`wpctl`)
- **Build & Driver Tools:** Rust (`cargo`), kernel headers, `dkms`, `make`, `clang`/`llvm` (if kernel was built with Clang) or `gcc`

---

## Installation

```bash
git clone https://github.com/gogaport/xicontrol-linux.git
cd xicontrol-linux
chmod +x setup-xicontrol.sh
sudo ./setup-xicontrol.sh
```

What `setup-xicontrol.sh` does:
1. Builds `cargo build --release` (if not already built) and installs `/usr/local/bin/xicontrol` (and `~/.local/bin/xicontrol` if present).
2. Registers and installs the `xiaomi_mifs` kernel module (`kmod/xiaomi_mifs.c`) via **DKMS** and `/etc/modules-load.d/xiaomi-mifs.conf`.
3. Installs udev rules in `/etc/udev/rules.d/72-xiaomi-xicontrol.rules` for `/dev/xiaomi_mifs`, `BLTP7853` `/dev/hidraw*`, `/dev/i2c-1`, `Redmibook WMI keys`, touchpad `evdev`, and `/dev/uinput`.
4. Runs `xicontrol status` to verify communication with EC and BLTP7853.

To enable the background daemon & tray icon at login (`systemd --user`):

```bash
xicontrol autostart on
```

---

## CLI Usage

```bash
xicontrol                      # Open GUI (starts background services if daemon is not running)
xicontrol daemon               # Run background daemon + KDE tray icons
xicontrol status               # Print live telemetry, EC mode, charge limit, display & BLTP7853 state
xicontrol set-mode <mode>      # eco | quiet | balance | auto | turbo | fullspeed
xicontrol set-charge <pct>     # 40 | 50 | 60 | 70 | 80 | 100
xicontrol care <on|off>        # Toggle Battery Care
xicontrol travel <on|off>      # Toggle Travel Mode (100%)
xicontrol owl <on|off>         # Toggle Owl Mode (keep awake)
xicontrol set-hz <hz>          # 60 | 120
xicontrol touchpad <on|off|toggle>
xicontrol touchscreen <on|off|toggle>
xicontrol trigger <slot>       # heavy_press | wmi_perf | wmi_hz | wmi_tp | custom_1 | custom_2
xicontrol set-vibration <low|medium|high>
xicontrol set-pressure <100|125|140>
xicontrol pulse                # Send test haptic click
xicontrol autostart <on|off|status>
```

---

## Notes for Xiaomi Book Pro 14 (`TM2424`)

- **Battery performance modes:** On battery power, the `TM2424` EC firmware only keeps `Eco` (`0x0A`) and `Auto` (`0x09`); `Quiet` (`0x02`), `Turbo` (`0x03`), and `FullSpeed` (`0x04`) are reverted by EC after ~150 ms unless AC power is connected, and `Balance` (`0x01`) is unsupported on `TM2424`.
- **Keyboard WMI vs `AT Translated Set 2`:** On `TM2424`, `Fn+K` and the Settings key emit `Redmibook WMI keys` scancodes, while `Fn+S` / `Fn+F` send regular `s`/`f` keycodes on the PS/2 keyboard under Linux. Bind any KDE shortcut to `xicontrol trigger <slot>` to map extra actions.

---

## Credits & License

- Hardware reverse engineering, WMI MIFS command map, and BLTP7853 I2C/HID protocol documentation: **[Oksion/XiControl](https://github.com/Oksion/XiControl)**.
- User-space application: **GPL-3.0** (see [`LICENSE`](LICENSE)).
- Kernel module (`kmod/xiaomi_mifs.c`): **GPL-2.0**.
