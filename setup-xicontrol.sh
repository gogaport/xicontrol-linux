#!/usr/bin/env bash
# ==============================================================================
# XiControl Linux installer: xiaomi_mifs DKMS module + udev rules + binary
# Tested on Xiaomi Book Pro 14 (TM2424) / CachyOS Linux (KDE Plasma Wayland)
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

if [[ $EUID -ne 0 ]]; then
   echo "Please run as root: sudo $SCRIPT_DIR/setup-xicontrol.sh"
   exit 1
fi

BIN_PATH=""
if [[ -x "$SCRIPT_DIR/target/release/xicontrol" ]]; then
   BIN_PATH="$SCRIPT_DIR/target/release/xicontrol"
elif [[ -x "$SCRIPT_DIR/xicontrol" ]]; then
   BIN_PATH="$SCRIPT_DIR/xicontrol"
else
   echo "==> Building xicontrol release binary..."
   if [[ -n "${SUDO_USER:-}" ]]; then
      sudo -u "$SUDO_USER" bash -lc "export PATH=\"\$HOME/.cargo/bin:\$PATH\" && cd \"$SCRIPT_DIR\" && cargo build --release"
   else
      (cd "$SCRIPT_DIR" && cargo build --release)
   fi
   BIN_PATH="$SCRIPT_DIR/target/release/xicontrol"
fi

echo "==> [1/4] Installing xicontrol binary to /usr/local/bin..."
install -Dm755 "$BIN_PATH" /usr/local/bin/xicontrol

if [[ -n "${SUDO_USER:-}" ]]; then
   USER_HOME="$(getent passwd "$SUDO_USER" | cut -d: -f6 || true)"
   if [[ -n "$USER_HOME" && -d "$USER_HOME/.local/bin" ]]; then
      install -Dm755 -o "$SUDO_USER" -g "$SUDO_USER" "$BIN_PATH" "$USER_HOME/.local/bin/xicontrol"
   fi
fi

echo "==> [2/4] Registering xiaomi_mifs kernel module with DKMS..."
rm -rf /usr/src/xiaomi-mifs-1.0.0
mkdir -p /usr/src/xiaomi-mifs-1.0.0
cp "$SCRIPT_DIR/kmod/"{xiaomi_mifs.c,Makefile,dkms.conf} /usr/src/xiaomi-mifs-1.0.0/

# Switch off LLVM=1 automatically if the running kernel was not built with Clang
if ! grep -qi clang /proc/version 2>/dev/null; then
   sed -i 's/ LLVM=1//g' /usr/src/xiaomi-mifs-1.0.0/dkms.conf /usr/src/xiaomi-mifs-1.0.0/Makefile
fi

dkms remove -m xiaomi-mifs -v 1.0.0 --all 2>/dev/null || true
dkms add -m xiaomi-mifs -v 1.0.0
dkms build -m xiaomi-mifs -v 1.0.0
dkms install -m xiaomi-mifs -v 1.0.0

echo "xiaomi_mifs" > /etc/modules-load.d/xiaomi-mifs.conf
rmmod bitland_mifs_wmi 2>/dev/null || true
rmmod xiaomi_mifs 2>/dev/null || true
modprobe xiaomi_mifs

echo "==> [3/4] Installing udev rules (/etc/udev/rules.d/72-xiaomi-xicontrol.rules)..."
cat << 'EOF' > /etc/udev/rules.d/72-xiaomi-xicontrol.rules
# Xiaomi MIFS WMI interface
KERNEL=="xiaomi_mifs", MODE="0666", TAG+="uaccess"
# Xiaomi Book Pro 14 Haptic Touchpad BLTP7853 (347D:7853)
KERNEL=="hidraw*", SUBSYSTEM=="hidraw", DEVPATH=="*347D:7853*", MODE="0666", TAG+="uaccess"
# I2C bus for BLTP7853 direct commands
KERNEL=="i2c-1", SUBSYSTEM=="i2c-dev", MODE="0666", TAG+="uaccess"
# Redmibook / Xiaomi WMI hotkeys
SUBSYSTEM=="input", ATTRS{name}=="Redmibook WMI keys", MODE="0644", TAG+="uaccess"
SUBSYSTEM=="input", ATTRS{name}=="Xiaomi WMI keys", MODE="0644", TAG+="uaccess"
# BLTP7853 Touchpad evdev + uinput (for Bottom Dead Zone palm filter)
SUBSYSTEM=="input", ATTRS{name}=="BLTP7853:00 347D:7853 Touchpad", MODE="0660", TAG+="uaccess"
KERNEL=="uinput", MODE="0660", TAG+="uaccess"
EOF

udevadm control --reload-rules
udevadm trigger --subsystem-match=misc --subsystem-match=hidraw --subsystem-match=i2c-dev --subsystem-match=input
chmod 0666 /dev/xiaomi_mifs /dev/i2c-1 2>/dev/null || true
for h in /sys/class/hidraw/hidraw*; do
   if grep -q "347D:00007853" "$h/device/uevent" 2>/dev/null; then
      chmod 0666 "/dev/$(basename "$h")"
   fi
done
for ev in /sys/class/input/event*; do
   if grep -qE "Redmibook WMI|Xiaomi WMI" "$ev/device/name" 2>/dev/null; then
      chmod 0644 "/dev/input/$(basename "$ev")"
   fi
   if grep -q "BLTP7853:00" "$ev/device/name" 2>/dev/null; then
      chmod 0660 "/dev/input/$(basename "$ev")"
   fi
done

echo "==> [4/4] Verifying hardware connection..."
echo ""
if [[ -n "${SUDO_USER:-}" ]]; then
   sudo -u "$SUDO_USER" /usr/local/bin/xicontrol status || true
else
   /usr/local/bin/xicontrol status || true
fi
