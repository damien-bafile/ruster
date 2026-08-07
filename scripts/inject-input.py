#!/usr/bin/env python3
"""Inject key presses and pointer motion through /dev/uinput.

Why this exists: `wtype` uploads its own keymap to the compositor it talks to,
and a *nested* ruster resolves the raw keycodes it receives with its own keymap
instead — so wtype's keys arrive as the wrong symbols, or as nothing. Going in
at the uinput level means the events travel the same path a real keyboard's do:
uinput -> evdev -> libinput -> the compositor. That is also the only path that
exists at all on a DRM boot, where there is no outer compositor to type into.

Needs write access to /dev/uinput (an ACL grants it on this machine; otherwise
`sudo`). The virtual device is created, used and destroyed per run.

    scripts/inject-input.py key super+h key super+shift+q
    scripts/inject-input.py move 300 200 click left
    scripts/inject-input.py --delay 0.3 key super+1 key super+2

Key names are the lowercase evdev suffixes: `a`, `1`, `f9`, `space`, `enter`.
Modifiers: super/meta/logo, shift, ctrl, alt.
"""

import argparse
import ctypes
import fcntl
import os
import struct
import time

UINPUT = "/dev/uinput"

EV_SYN, EV_KEY, EV_REL, EV_ABS = 0x00, 0x01, 0x02, 0x03
SYN_REPORT = 0
REL_X, REL_Y, REL_WHEEL = 0x00, 0x01, 0x08

UI_DEV_CREATE = 0x5501
UI_DEV_DESTROY = 0x5502
UI_SET_EVBIT = 0x40045564
UI_SET_KEYBIT = 0x40045565
UI_SET_RELBIT = 0x40045566

BTN_LEFT, BTN_RIGHT, BTN_MIDDLE = 0x110, 0x111, 0x112

# Linux keycodes (include/uapi/linux/input-event-codes.h). Only the ones worth
# typing at a window manager; extend as needed.
KEYS = {
    "esc": 1, "escape": 1,
    "1": 2, "2": 3, "3": 4, "4": 5, "5": 6,
    "6": 7, "7": 8, "8": 9, "9": 10, "0": 11,
    "minus": 12, "equal": 13, "backspace": 14, "tab": 15,
    "q": 16, "w": 17, "e": 18, "r": 19, "t": 20, "y": 21,
    "u": 22, "i": 23, "o": 24, "p": 25,
    "enter": 28, "return": 28, "ctrl": 29, "control": 29,
    "a": 30, "s": 31, "d": 32, "f": 33, "g": 34,
    "h": 35, "j": 36, "k": 37, "l": 38,
    "semicolon": 39, "apostrophe": 40, "grave": 41,
    "shift": 42,
    "backslash": 43, "z": 44, "x": 45, "c": 46, "v": 47,
    "b": 48, "n": 49, "m": 50, "comma": 51, "dot": 52, "slash": 53,
    "rightshift": 54, "alt": 56, "space": 57, "capslock": 58,
    "f1": 59, "f2": 60, "f3": 61, "f4": 62, "f5": 63, "f6": 64,
    "f7": 65, "f8": 66, "f9": 67, "f10": 68, "f11": 87, "f12": 88,
    "up": 103, "left": 105, "right": 106, "down": 108,
    "super": 125, "meta": 125, "logo": 125,
}

BUTTONS = {"left": BTN_LEFT, "right": BTN_RIGHT, "middle": BTN_MIDDLE}


class UinputUserDev(ctypes.Structure):
    _fields_ = [
        ("name", ctypes.c_char * 80),
        ("id_bustype", ctypes.c_uint16),
        ("id_vendor", ctypes.c_uint16),
        ("id_product", ctypes.c_uint16),
        ("id_version", ctypes.c_uint16),
        ("ff_effects_max", ctypes.c_uint32),
        ("absmax", ctypes.c_int32 * 64),
        ("absmin", ctypes.c_int32 * 64),
        ("absfuzz", ctypes.c_int32 * 64),
        ("absflat", ctypes.c_int32 * 64),
    ]


class VirtualDevice:
    """A uinput keyboard+pointer, alive for the duration of the `with` block."""

    def __init__(self, name=b"ruster-inject"):
        self.name = name
        self.fd = None

    def __enter__(self):
        self.fd = os.open(UINPUT, os.O_WRONLY | os.O_NONBLOCK)
        for ev in (EV_KEY, EV_REL, EV_SYN):
            fcntl.ioctl(self.fd, UI_SET_EVBIT, ev)
        for code in set(KEYS.values()) | set(BUTTONS.values()):
            fcntl.ioctl(self.fd, UI_SET_KEYBIT, code)
        for rel in (REL_X, REL_Y, REL_WHEEL):
            fcntl.ioctl(self.fd, UI_SET_RELBIT, rel)

        dev = UinputUserDev()
        dev.name = self.name
        dev.id_bustype = 0x03  # BUS_USB
        dev.id_vendor = 0x1234
        dev.id_product = 0x5678
        dev.id_version = 1
        os.write(self.fd, bytes(dev))
        fcntl.ioctl(self.fd, UI_DEV_CREATE)
        # udev has to notice the device and libinput has to open it before
        # anything sent is seen. Without this pause the first events vanish.
        time.sleep(0.4)
        return self

    def __exit__(self, *_):
        if self.fd is not None:
            fcntl.ioctl(self.fd, UI_DEV_DESTROY)
            os.close(self.fd)

    def emit(self, etype, code, value):
        # struct input_event: two time fields (64-bit each here), type, code, value
        os.write(self.fd, struct.pack("llHHi", 0, 0, etype, code, value))

    def sync(self):
        self.emit(EV_SYN, SYN_REPORT, 0)

    def chord(self, spec, hold=0.03):
        """Press `super+shift+q`-style chords: modifiers down, key, all up."""
        parts = [p.strip().lower() for p in spec.split("+") if p.strip()]
        codes = []
        for part in parts:
            if part not in KEYS:
                raise SystemExit(f"unknown key: {part!r}")
            codes.append(KEYS[part])
        for code in codes:
            self.emit(EV_KEY, code, 1)
            self.sync()
        time.sleep(hold)
        for code in reversed(codes):
            self.emit(EV_KEY, code, 0)
            self.sync()

    def move(self, dx, dy):
        self.emit(EV_REL, REL_X, dx)
        self.emit(EV_REL, REL_Y, dy)
        self.sync()

    def click(self, button):
        code = BUTTONS.get(button)
        if code is None:
            raise SystemExit(f"unknown button: {button!r}")
        self.emit(EV_KEY, code, 1)
        self.sync()
        time.sleep(0.03)
        self.emit(EV_KEY, code, 0)
        self.sync()

    def scroll(self, amount):
        self.emit(EV_REL, REL_WHEEL, amount)
        self.sync()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--delay", type=float, default=0.25,
        help="seconds between actions (default 0.25)",
    )
    parser.add_argument(
        "actions", nargs="+",
        help="key <chord> | move <dx> <dy> | click <button> | scroll <n> | sleep <s>",
    )
    args = parser.parse_args()

    if not os.access(UINPUT, os.W_OK):
        raise SystemExit(f"{UINPUT} is not writable; run with sudo or fix the ACL")

    with VirtualDevice() as dev:
        argv = list(args.actions)
        while argv:
            verb = argv.pop(0)
            if verb == "key":
                spec = argv.pop(0)
                dev.chord(spec)
                print(f"key {spec}", flush=True)
            elif verb == "move":
                dx, dy = int(argv.pop(0)), int(argv.pop(0))
                dev.move(dx, dy)
                print(f"move {dx} {dy}", flush=True)
            elif verb == "click":
                button = argv.pop(0)
                dev.click(button)
                print(f"click {button}", flush=True)
            elif verb == "scroll":
                amount = int(argv.pop(0))
                dev.scroll(amount)
                print(f"scroll {amount}", flush=True)
            elif verb == "sleep":
                time.sleep(float(argv.pop(0)))
                continue
            else:
                raise SystemExit(f"unknown action: {verb!r}")
            time.sleep(args.delay)


if __name__ == "__main__":
    main()
