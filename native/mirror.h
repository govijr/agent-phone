// C interface to a real iPhone through macOS iPhone Mirroring.
//
// Coordinates passed in and out are points relative to the top-left of the mirroring window,
// which shows the phone's screen edge to edge. Functions return 0 (or a non-NULL string) on
// success and otherwise set *err to a message the caller frees with ap_free.

#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// JSON: {"running", "state": "not-running|no-window|blocked|ready", "window": {x,y,w,h,id} | null,
//        "blocked_text": [..], "buttons": [..], "accessibility": bool, "screen_recording": bool,
//        "frontmost": bool}
char *ap_mirror_status(char **err);

// Opens iPhone Mirroring in the background (if it is not running) and waits up to timeout_s for
// its phone window. Returns the status JSON.
char *ap_mirror_launch(double timeout_s, char **err);

// Brings the mirroring window frontmost and confirms it, or fails without sending anything.
int ap_mirror_activate(double timeout_s, char **err);

// Presses the button with this title (case-insensitive) on a Mirroring interstitial, such as
// "Connect" or "Try Again". With NULL, presses the only button if there is exactly one.
char *ap_mirror_press_button(const char *title, char **err);

// Vision text recognition over a PNG of the window. JSON: [{text, confidence, frame:[x,y,w,h]}]
// with frames in window points. window_w/window_h are the window's size in points.
char *ap_mirror_ocr(const char *png_path, double window_w, double window_h, char **err);

// A finger down at the first point, dragged through the rest, lifted at the last.
int ap_mirror_touch(const double *xs, const double *ys, int n, int hold_ms, int step_ms, char **err);

// Scroll-wheel gesture at (x, y). dy > 0 reveals content further down, dx > 0 further right.
int ap_mirror_scroll(double x, double y, double dx, double dy, char **err);

// Mac virtual key codes with down/up and the modifier flags held at that moment.
int ap_mirror_keys(const uint16_t *codes, const uint8_t *downs, const uint64_t *flags, int n, char **err);

#ifdef __cplusplus
}
#endif
