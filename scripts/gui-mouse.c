// Send real mouse events to the running raylib window (macOS).
//
//   scripts/gui-mouse <command>...
//
//     move <x> <y>              move the pointer to a screen point
//     click <x> <y> [button]    press and release; button = left|right|middle
//     down  <x> <y> [button]    press and hold
//     up    <x> <y> [button]    release
//     drag  <x> <y>             move with the button held (between down and up)
//     wheel <x> <y> <notches>   scroll; positive is up
//     sleep <ms>
//
// Why this exists: the mouse surface only exists between real pointer events,
// and neither of the other drivers can produce one. `ruster.cmd` queues ex
// commands, gui-keys.sh sends keystrokes, and scripts/inject-input.py goes
// through /dev/uinput, which is Linux-only. CoreGraphics is the macOS
// equivalent and ships with the OS, so this needs nothing installed —
// build it with scripts/gui-mouse-build.sh.
//
// PERMISSION: posting events into another application requires Accessibility
// access for whatever runs this (Terminal, iTerm, your IDE). Without it the
// calls silently do nothing — the capture then shows an untouched editor rather
// than an error, so this program probes for the permission up front and refuses
// to run without it.
//
// Coordinates are screen points with the origin at the top-left, which is what
// CGEvent wants and what the window's own origin is reported in.

#include <ApplicationServices/ApplicationServices.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static CGMouseButton button_from(const char *name) {
    if (strcmp(name, "right") == 0) return kCGMouseButtonRight;
    if (strcmp(name, "middle") == 0) return kCGMouseButtonCenter;
    return kCGMouseButtonLeft;
}

static CGEventType down_event(CGMouseButton b) {
    if (b == kCGMouseButtonRight) return kCGEventRightMouseDown;
    if (b == kCGMouseButtonCenter) return kCGEventOtherMouseDown;
    return kCGEventLeftMouseDown;
}

static CGEventType up_event(CGMouseButton b) {
    if (b == kCGMouseButtonRight) return kCGEventRightMouseUp;
    if (b == kCGMouseButtonCenter) return kCGEventOtherMouseUp;
    return kCGEventLeftMouseUp;
}

static CGEventType drag_event(CGMouseButton b) {
    if (b == kCGMouseButtonRight) return kCGEventRightMouseDragged;
    if (b == kCGMouseButtonCenter) return kCGEventOtherMouseDragged;
    return kCGEventLeftMouseDragged;
}

// Post one event and let the target app's run loop pick it up. Without the
// pause a burst of events can be coalesced or delivered out of order, which
// turns a scripted double-click into an unpredictable one.
static void post(CGEventRef e) {
    if (!e) return;
    CGEventPost(kCGHIDEventTap, e);
    CFRelease(e);
    usleep(40 * 1000);
}

static void mouse_event(CGEventType type, CGPoint at, CGMouseButton b, int clicks) {
    CGEventRef e = CGEventCreateMouseEvent(NULL, type, at, b);
    if (e && clicks > 1) {
        // A double- or triple-click is one event carrying its count, not two
        // or three separate clicks — the OS is what decides they are a streak.
        CGEventSetIntegerValueField(e, kCGMouseEventClickState, clicks);
    }
    post(e);
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: gui-mouse <move|click|down|up|drag|wheel|sleep> ...\n");
        return 2;
    }

    // Refuse rather than no-op: a silent failure here produces a verification
    // artifact that looks like a bug in the editor.
    if (!AXIsProcessTrusted()) {
        fprintf(stderr,
                "gui-mouse: no Accessibility permission.\n"
                "Grant it to the app running this (Terminal/iTerm/your IDE) in\n"
                "System Settings > Privacy & Security > Accessibility, then retry.\n");
        return 3;
    }

    int i = 1;
    while (i < argc) {
        const char *cmd = argv[i++];

        if (strcmp(cmd, "sleep") == 0) {
            if (i >= argc) goto missing;
            usleep((useconds_t)(atoi(argv[i++]) * 1000));
            continue;
        }

        if (strcmp(cmd, "move") == 0 || strcmp(cmd, "drag") == 0) {
            if (i + 1 >= argc) goto missing;
            CGPoint at = CGPointMake(atof(argv[i]), atof(argv[i + 1]));
            i += 2;
            if (strcmp(cmd, "move") == 0) {
                mouse_event(kCGEventMouseMoved, at, kCGMouseButtonLeft, 1);
            } else {
                mouse_event(drag_event(kCGMouseButtonLeft), at, kCGMouseButtonLeft, 1);
            }
            continue;
        }

        if (strcmp(cmd, "wheel") == 0) {
            if (i + 2 >= argc) goto missing;
            CGPoint at = CGPointMake(atof(argv[i]), atof(argv[i + 1]));
            int notches = atoi(argv[i + 2]);
            i += 3;
            // Move first: the wheel goes to whatever is under the pointer.
            mouse_event(kCGEventMouseMoved, at, kCGMouseButtonLeft, 1);
            int step = notches > 0 ? 1 : -1;
            for (int n = 0; n != notches; n += step) {
                post(CGEventCreateScrollWheelEvent(NULL, kCGScrollEventUnitLine, 1, step));
            }
            continue;
        }

        if (strcmp(cmd, "click") == 0 || strcmp(cmd, "down") == 0 ||
            strcmp(cmd, "up") == 0) {
            if (i + 1 >= argc) goto missing;
            CGPoint at = CGPointMake(atof(argv[i]), atof(argv[i + 1]));
            i += 2;
            CGMouseButton b = kCGMouseButtonLeft;
            int clicks = 1;
            // Optional trailing button name, and for `click` an optional count.
            if (i < argc && (strcmp(argv[i], "left") == 0 || strcmp(argv[i], "right") == 0 ||
                             strcmp(argv[i], "middle") == 0)) {
                b = button_from(argv[i++]);
            }
            if (strcmp(cmd, "click") == 0 && i < argc && argv[i][0] >= '1' && argv[i][0] <= '9' &&
                argv[i][1] == '\0') {
                clicks = atoi(argv[i++]);
            }

            // Move first, always.
            //
            // GLFW — which raylib sits on — reports a button press at the
            // cursor position it has cached from motion events, not at the
            // position carried by the press itself. A press posted without a
            // preceding move is therefore delivered wherever the pointer
            // physically happened to be, which is usually not the window at
            // all: the click appears to do nothing.
            mouse_event(kCGEventMouseMoved, at, kCGMouseButtonLeft, 1);

            if (strcmp(cmd, "down") == 0) {
                mouse_event(down_event(b), at, b, 1);
            } else if (strcmp(cmd, "up") == 0) {
                mouse_event(up_event(b), at, b, 1);
            } else {
                // Each click of a streak is a full press/release carrying its
                // ordinal, which is how the OS reports a real double-click.
                for (int c = 1; c <= clicks; c++) {
                    mouse_event(down_event(b), at, b, c);
                    mouse_event(up_event(b), at, b, c);
                }
            }
            continue;
        }

        fprintf(stderr, "gui-mouse: unknown command %s\n", cmd);
        return 2;
    }
    return 0;

missing:
    fprintf(stderr, "gui-mouse: missing arguments\n");
    return 2;
}
