// C interface between the Rust CLI and Apple's simulator frameworks.
//
// Every function takes the Xcode developer directory and the simulator UDID, returns 0 (or a
// non-NULL string) on success, and on failure sets *err to a malloc'd message the caller frees
// with ap_free. No Objective-C exception ever crosses this boundary.

#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// JSON: {"width": points, "height": points, "scale": n}
char *ap_screen(const char *dev_dir, const char *udid, char **err);

// JSON tree of the frontmost app's accessibility elements. Frames are in screen points.
char *ap_snapshot(const char *dev_dir, const char *udid, double timeout_s, char **err);

// A single-finger gesture through n points (screen points). The finger goes down at the first
// point, moves through the rest with step_ms between them, and lifts at the last.
// hold_ms is how long it stays down at the first point before moving (a long press when n == 1).
int ap_touch(const char *dev_dir, const char *udid, const double *xs, const double *ys, int n,
             int hold_ms, int step_ms, char **err);

// Hardware buttons: 0 = home, 1 = lock.
int ap_button(const char *dev_dir, const char *udid, int button, char **err);

// Keyboard: n HID usages (page 7) with directions (1 = down, 2 = up), sent in order.
int ap_keys(const char *dev_dir, const char *udid, const uint32_t *usages,
            const uint8_t *directions, int n, char **err);

// The screen as JPEG (quality 0-1) or PNG (quality <= 0) bytes; *len is set. Free with ap_free.
#include <stddef.h>
char *ap_frame(const char *dev_dir, const char *udid, double quality, size_t *len, char **err);

void ap_free(char *s);

#ifdef __cplusplus
}
#endif
