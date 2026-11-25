// A real iPhone through macOS iPhone Mirroring.
//
// The mirroring window is a video stream: macOS accessibility sees nothing inside it, so the
// screen is read with Vision text recognition and input is synthesized as Mac mouse and keyboard
// events, which only reach the phone while the window is frontmost. Window discovery, session
// detection, the focus checks and the input details follow phone-harness (MIT); see
// THIRD_PARTY_NOTICES.md.

#import "mirror.h"

#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <ImageIO/ImageIO.h>
#import <Vision/Vision.h>

static NSString *const kBundleID = @"com.apple.ScreenContinuity";
static NSString *const kAppPath = @"/System/Applications/iPhone Mirroring.app";

void ap_free(char *s);

static char *m_strdup(NSString *s) { return strdup((s ?: @"").UTF8String); }

static void m_err(char **err, NSString *message) {
  if (err) {
    *err = m_strdup(message);
  }
}

static char *m_json(id object) {
  NSData *data = [NSJSONSerialization dataWithJSONObject:object options:0 error:nil];
  return m_strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding]);
}

#pragma mark - App and window

static NSRunningApplication *mirror_app(void) {
  return [NSRunningApplication runningApplicationsWithBundleIdentifier:kBundleID].firstObject;
}

static NSArray<NSDictionary *> *window_list(CGWindowListOption options) {
  return CFBridgingRelease(CGWindowListCopyWindowInfo(options, kCGNullWindowID)) ?: @[];
}

static int layer_of(NSDictionary *w) { return [w[(id)kCGWindowLayer] intValue]; }
static pid_t owner_of(NSDictionary *w) { return [w[(id)kCGWindowOwnerPID] intValue]; }

static CGRect bounds_of(NSDictionary *w) {
  CGRect r = CGRectZero;
  CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)w[(id)kCGWindowBounds], &r);
  return r;
}

// The phone window, matched by the owner's pid rather than the (localized) app name. No size
// filter: Stage Manager parks an inactive window as a small on-screen thumbnail.
static NSDictionary *mirror_window(BOOL onScreenOnly) {
  NSRunningApplication *app = mirror_app();
  if (!app) {
    return nil;
  }
  pid_t pid = app.processIdentifier;
  NSArray *all = window_list(onScreenOnly ? kCGWindowListOptionOnScreenOnly : kCGWindowListOptionAll);
  NSMutableArray *candidates = [NSMutableArray array];
  for (NSDictionary *w in all) {
    if (owner_of(w) == pid && layer_of(w) == 0) {
      [candidates addObject:w];
    }
  }
  if (!onScreenOnly) {
    // Off the active Space the app also lists its settings sheet and toolbar strips; the phone
    // window is the one titled with the app's own (equally localized) name.
    NSMutableArray *named = [NSMutableArray array];
    for (NSDictionary *w in candidates) {
      id name = w[(id)kCGWindowName];
      if (name && [name isEqual:w[(id)kCGWindowOwnerName]]) {
        [named addObject:w];
      }
    }
    if (named.count) {
      candidates = named;
    }
  }
  return candidates.firstObject;
}

static NSDictionary *window_json(NSDictionary *w) {
  CGRect r = bounds_of(w);
  return @{
    @"x" : @(r.origin.x), @"y" : @(r.origin.y), @"w" : @(r.size.width), @"h" : @(r.size.height),
    @"id" : w[(id)kCGWindowNumber] ?: @0
  };
}

// The frontmost normal-layer window, from the live window list (no run loop needed).
static NSDictionary *frontmost_window(void) {
  for (NSDictionary *w in window_list(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements)) {
    if (layer_of(w) == 0) {
      return w;
    }
  }
  return nil;
}

#pragma mark - Accessibility helpers

static id ax_copy(AXUIElementRef element, CFStringRef attribute) {
  CFTypeRef value = NULL;
  if (AXUIElementCopyAttributeValue(element, attribute, &value) != kAXErrorSuccess) {
    return nil;
  }
  return CFBridgingRelease(value);
}

static BOOL ax_rect(AXUIElementRef element, CGRect *out) {
  id position = ax_copy(element, kAXPositionAttribute);
  id size = ax_copy(element, kAXSizeAttribute);
  CGPoint p;
  CGSize s;
  if (!position || !size || !AXValueGetValue((__bridge AXValueRef)position, kAXValueTypeCGPoint, &p) ||
      !AXValueGetValue((__bridge AXValueRef)size, kAXValueTypeCGSize, &s)) {
    return NO;
  }
  *out = (CGRect){p, s};
  return YES;
}

// The accessibility element of the phone window, matched by geometry.
static id ax_phone_window(NSRunningApplication *app, NSDictionary *window) {
  if (!app || !window) {
    return nil;
  }
  AXUIElementRef appElement = AXUIElementCreateApplication(app.processIdentifier);
  NSArray *windows = ax_copy(appElement, kAXWindowsAttribute);
  CFRelease(appElement);
  CGRect target = bounds_of(window);
  for (id w in windows) {
    CGRect r;
    if (ax_rect((__bridge AXUIElementRef)w, &r) && fabs(r.origin.x - target.origin.x) < 4 &&
        fabs(r.origin.y - target.origin.y) < 4 && fabs(r.size.width - target.size.width) < 4 &&
        fabs(r.size.height - target.size.height) < 4) {
      return w;
    }
  }
  return nil;
}

// Native Mac UI drawn inside the phone window, as {role, text, element}. A live session has none,
// because the phone is a video stream; "iPhone in Use", "Paused", "Connect" and the like are
// ordinary Mac views. Detecting them by their presence works in every language.
static void collect_nodes(id node, int depth, NSMutableArray *out) {
  static NSSet *roles;
  static dispatch_once_t once;
  dispatch_once(&once, ^{
    roles = [NSSet setWithArray:@[ @"AXStaticText", @"AXButton", @"AXTextField", @"AXSecureTextField", @"AXImage" ]];
  });
  if (depth > 6 || out.count > 40) {
    return;
  }
  AXUIElementRef el = (__bridge AXUIElementRef)node;
  NSString *role = ax_copy(el, kAXRoleAttribute) ?: @"";
  if ([roles containsObject:role]) {
    NSMutableArray *parts = [NSMutableArray array];
    for (NSString *attribute in @[ (id)kAXTitleAttribute, (id)kAXValueAttribute, (id)kAXDescriptionAttribute ]) {
      id v = ax_copy(el, (__bridge CFStringRef)attribute);
      if ([v isKindOfClass:NSString.class] && [v length]) {
        [parts addObject:[v stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet]];
      }
    }
    [out addObject:@{@"role" : role, @"text" : [parts componentsJoinedByString:@" "], @"element" : node}];
  }
  for (id child in ax_copy(el, kAXChildrenAttribute) ?: @[]) {
    collect_nodes(child, depth + 1, out);
  }
}

static NSArray<NSDictionary *> *interstitial_nodes(NSRunningApplication *app, NSDictionary *window) {
  id root = ax_phone_window(app, window);
  if (!root) {
    return @[];
  }
  NSMutableArray *out = [NSMutableArray array];
  collect_nodes(root, 0, out);
  return out;
}

static BOOL is_frontmost(NSRunningApplication *app) {
  if (!app) {
    return NO;
  }
  // Both signals must agree: AXFrontmost (focus) and the window list's front window (z-order).
  // Either alone has been seen reporting the phone as in front while another app gets the input.
  AXUIElementRef appElement = AXUIElementCreateApplication(app.processIdentifier);
  id frontmost = ax_copy(appElement, kAXFrontmostAttribute);
  CFRelease(appElement);
  if (![frontmost boolValue]) {
    return NO;
  }
  NSDictionary *front = frontmost_window();
  return front && owner_of(front) == app.processIdentifier;
}

#pragma mark - Status

static NSDictionary *status(void) {
  NSMutableDictionary *s = [NSMutableDictionary dictionary];
  s[@"accessibility"] = @((BOOL)AXIsProcessTrusted());
  s[@"screen_recording"] = @((BOOL)CGPreflightScreenCaptureAccess());
  NSRunningApplication *app = mirror_app();
  s[@"running"] = @((BOOL)(app != nil));
  s[@"window"] = NSNull.null;
  s[@"blocked_text"] = @[];
  s[@"buttons"] = @[];
  s[@"frontmost"] = @NO;
  if (!app) {
    s[@"state"] = @"not-running";
    return s;
  }
  NSDictionary *window = mirror_window(NO);
  if (!window) {
    s[@"state"] = @"no-window";
    return s;
  }
  s[@"window"] = window_json(mirror_window(YES) ?: window);
  s[@"on_screen"] = @((BOOL)(mirror_window(YES) != nil));
  s[@"frontmost"] = @((BOOL)is_frontmost(app));
  NSArray *nodes = AXIsProcessTrusted() ? interstitial_nodes(app, window) : @[];
  NSMutableArray *texts = [NSMutableArray array];
  NSMutableArray *buttons = [NSMutableArray array];
  for (NSDictionary *node in nodes) {
    if ([node[@"text"] length] == 0) {
      continue;
    }
    [([node[@"role"] isEqual:@"AXButton"] ? buttons : texts) addObject:node[@"text"]];
  }
  s[@"blocked_text"] = texts;
  s[@"buttons"] = buttons;
  s[@"state"] = !AXIsProcessTrusted() ? @"unknown" : (nodes.count ? @"blocked" : @"ready");
  return s;
}

char *ap_mirror_status(char **err) {
  @autoreleasepool {
    @try {
      return m_json(status());
    } @catch (NSException *exception) {
      m_err(err, exception.reason);
      return NULL;
    }
  }
}

static void run_open(NSArray<NSString *> *arguments) {
  NSTask *task = [NSTask new];
  task.executableURL = [NSURL fileURLWithPath:@"/usr/bin/open"];
  task.arguments = arguments;
  task.standardOutput = NSFileHandle.fileHandleWithNullDevice;
  task.standardError = NSFileHandle.fileHandleWithNullDevice;
  if ([task launchAndReturnError:nil]) {
    [task waitUntilExit];
  }
}

char *ap_mirror_launch(double timeout_s, char **err) {
  @autoreleasepool {
    @try {
      if (!mirror_app()) {
        run_open(@[ @"-g", @"-a", kAppPath ]);  // -g: don't take the user's focus
      }
      NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:timeout_s];
      while (!mirror_window(NO) && deadline.timeIntervalSinceNow > 0) {
        usleep(400 * 1000);
      }
      return m_json(status());
    } @catch (NSException *exception) {
      m_err(err, exception.reason);
      return NULL;
    }
  }
}

#pragma mark - Focus

static NSString *activate(double timeout) {
  NSRunningApplication *app = mirror_app();
  if (!app) {
    return @"iPhone Mirroring isn't running. Run `agent-phone --device iphone connect`";
  }
  if (!AXIsProcessTrusted()) {
    return @"this terminal has no Accessibility permission. Run `agent-phone --device iphone doctor`";
  }
  if (is_frontmost(app)) {
    return nil;
  }
  NSDictionary *window = mirror_window(NO);
  id element = ax_phone_window(app, window);
  if (element) {
    id minimized = ax_copy((__bridge AXUIElementRef)element, kAXMinimizedAttribute);
    if ([minimized boolValue]) {
      AXUIElementSetAttributeValue((__bridge AXUIElementRef)element, kAXMinimizedAttribute, kCFBooleanFalse);
    }
  }
  AXUIElementRef appElement = AXUIElementCreateApplication(app.processIdentifier);
  NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:timeout];
  // Re-assert on every pass: another app can take focus straight back, and once the app reports
  // itself frontmost, activation alone no longer raises the window, while AXRaise does.
  while (deadline.timeIntervalSinceNow > 0) {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    [app activateWithOptions:NSApplicationActivateIgnoringOtherApps];
#pragma clang diagnostic pop
    AXUIElementSetAttributeValue(appElement, kAXFrontmostAttribute, kCFBooleanTrue);
    if (element) {
      AXUIElementPerformAction((__bridge AXUIElementRef)element, kAXRaiseAction);
    }
    usleep(80 * 1000);
    if (is_frontmost(app)) {
      CFRelease(appElement);
      return nil;
    }
  }
  CFRelease(appElement);
  // Last resort: Launch Services also switches to the window's Space.
  run_open(@[ @"-a", kAppPath ]);
  for (int i = 0; i < 15; i++) {
    usleep(100 * 1000);
    if (is_frontmost(app)) {
      return nil;
    }
  }
  NSString *owner = frontmost_window()[(id)kCGWindowOwnerName] ?: @"another app";
  return [NSString stringWithFormat:@"could not bring iPhone Mirroring in front; %@ still has it. Nothing was sent", owner];
}

int ap_mirror_activate(double timeout_s, char **err) {
  @autoreleasepool {
    NSString *failure = activate(timeout_s > 0 ? timeout_s : 2.5);
    if (failure) {
      m_err(err, failure);
      return 1;
    }
    return 0;
  }
}

char *ap_mirror_press_button(const char *title, char **err) {
  @autoreleasepool {
    NSRunningApplication *app = mirror_app();
    NSDictionary *window = mirror_window(NO);
    NSMutableArray *buttons = [NSMutableArray array];
    for (NSDictionary *node in interstitial_nodes(app, window)) {
      if ([node[@"role"] isEqual:@"AXButton"]) {
        [buttons addObject:node];
      }
    }
    if (title) {
      NSString *wanted = @(title).lowercaseString;
      [buttons filterUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(NSDictionary *b, id _) {
                 return [[b[@"text"] lowercaseString] isEqual:wanted];
               }]];
    } else if (buttons.count != 1) {
      m_err(err, buttons.count ? @"several buttons are showing; name the one to press" : @"no button is showing");
      return NULL;
    }
    if (!buttons.count) {
      m_err(err, [NSString stringWithFormat:@"no \"%s\" button is showing", title]);
      return NULL;
    }
    NSDictionary *button = buttons.firstObject;
    if (AXUIElementPerformAction((__bridge AXUIElementRef)button[@"element"], kAXPressAction) != kAXErrorSuccess) {
      m_err(err, @"the button did not respond");
      return NULL;
    }
    return m_strdup(button[@"text"]);
  }
}

#pragma mark - Text recognition

char *ap_mirror_ocr(const char *png_path, double window_w, double window_h, char **err) {
  @autoreleasepool {
    @try {
      NSURL *url = [NSURL fileURLWithPath:@(png_path)];
      CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url, NULL);
      if (!source) {
        m_err(err, @"could not read the screenshot");
        return NULL;
      }
      NSDictionary *props = CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source, 0, NULL));
      CFRelease(source);
      double imageW = [props[(id)kCGImagePropertyPixelWidth] doubleValue];
      double imageH = [props[(id)kCGImagePropertyPixelHeight] doubleValue];
      if (imageW <= 0 || imageH <= 0) {
        m_err(err, @"the screenshot is empty");
        return NULL;
      }
      VNRecognizeTextRequest *request = [VNRecognizeTextRequest new];
      request.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
      VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithURL:url options:@{}];
      NSError *error = nil;
      if (![handler performRequests:@[ request ] error:&error]) {
        m_err(err, [NSString stringWithFormat:@"text recognition failed: %@", error.localizedDescription]);
        return NULL;
      }
      double sx = window_w / imageW, sy = window_h / imageH;
      NSMutableArray *out = [NSMutableArray array];
      for (VNRecognizedTextObservation *observation in request.results) {
        VNRecognizedText *best = [observation topCandidates:1].firstObject;
        if (!best) {
          continue;
        }
        // Vision boxes are normalized with a bottom-left origin.
        CGRect box = observation.boundingBox;
        double px = box.origin.x * imageW;
        double pyTop = (1.0 - box.origin.y - box.size.height) * imageH;
        double pw = box.size.width * imageW;
        double ph = box.size.height * imageH;
        [out addObject:@{
          @"text" : best.string,
          @"confidence" : @(best.confidence),
          @"frame" : @[ @(px * sx), @(pyTop * sy), @(pw * sx), @(ph * sy) ],
        }];
      }
      return m_json(out);
    } @catch (NSException *exception) {
      m_err(err, exception.reason);
      return NULL;
    }
  }
}

#pragma mark - Input

// Window-relative points to global screen points, refusing a point that another window covers.
static NSString *global_point(double x, double y, CGPoint *out) {
  NSRunningApplication *app = mirror_app();
  NSDictionary *window = mirror_window(YES);
  if (!app || !window) {
    return @"the iPhone Mirroring window isn't on screen";
  }
  CGRect r = bounds_of(window);
  if (x < 0 || y < 0 || x > r.size.width || y > r.size.height) {
    return [NSString stringWithFormat:@"%.0f,%.0f is outside the phone screen (%.0fx%.0f)", x, y, r.size.width, r.size.height];
  }
  CGPoint p = CGPointMake(r.origin.x + x, r.origin.y + y);
  // The topmost normal window under the point must be the phone, or the event lands in that app.
  for (NSDictionary *w in window_list(kCGWindowListOptionOnScreenOnly)) {
    if (layer_of(w) != 0 || !CGRectContainsPoint(bounds_of(w), p)) {
      continue;
    }
    if (owner_of(w) != app.processIdentifier) {
      return [NSString stringWithFormat:@"%@ covers that part of the phone; nothing was sent", w[(id)kCGWindowOwnerName]];
    }
    break;
  }
  *out = p;
  return nil;
}

static void post_mouse(CGEventType type, CGPoint p) {
  CGEventRef event = CGEventCreateMouseEvent(NULL, type, p, kCGMouseButtonLeft);
  CGEventPost(kCGHIDEventTap, event);
  CFRelease(event);
}

int ap_mirror_touch(const double *xs, const double *ys, int n, int hold_ms, int step_ms, char **err) {
  @autoreleasepool {
    if (n < 1) {
      m_err(err, @"a touch needs at least one point");
      return 1;
    }
    NSString *failure = activate(2.5);
    if (failure) {
      m_err(err, failure);
      return 1;
    }
    CGPoint points[n];
    for (int i = 0; i < n; i++) {
      if ((failure = global_point(xs[i], ys[i], &points[i]))) {
        m_err(err, failure);
        return 1;
      }
    }
    post_mouse(kCGEventMouseMoved, points[0]);
    usleep(100 * 1000);
    post_mouse(kCGEventLeftMouseDown, points[0]);
    usleep((useconds_t)MAX(hold_ms, 60) * 1000);
    for (int i = 1; i < n; i++) {
      post_mouse(kCGEventLeftMouseDragged, points[i]);
      usleep((useconds_t)MAX(step_ms, 1) * 1000);
    }
    post_mouse(kCGEventLeftMouseUp, points[n - 1]);
    return 0;
  }
}

int ap_mirror_scroll(double x, double y, double dx, double dy, char **err) {
  @autoreleasepool {
    NSString *failure = activate(2.5);
    CGPoint p;
    if (failure || (failure = global_point(x, y, &p))) {
      m_err(err, failure);
      return 1;
    }
    post_mouse(kCGEventMouseMoved, p);
    usleep(100 * 1000);
    const int steps = 6;
    for (int i = 0; i < steps; i++) {
      CGEventRef event = CGEventCreateScrollWheelEvent(NULL, kCGScrollEventUnitPixel, 2,
                                                       (int32_t)(dy / steps), (int32_t)(dx / steps));
      CGEventSetLocation(event, p);
      CGEventPost(kCGHIDEventTap, event);
      CFRelease(event);
      usleep(30 * 1000);
    }
    return 0;
  }
}

int ap_mirror_keys(const uint16_t *codes, const uint8_t *downs, const uint64_t *flags, int n, char **err) {
  @autoreleasepool {
    NSString *failure = activate(2.5);
    if (failure) {
      m_err(err, failure);
      return 1;
    }
    for (int i = 0; i < n; i++) {
      CGEventRef event = CGEventCreateKeyboardEvent(NULL, codes[i], downs[i] != 0);
      CGEventSetFlags(event, (CGEventFlags)flags[i]);
      CGEventPost(kCGHIDEventTap, event);
      CFRelease(event);
      usleep(12 * 1000);
    }
    return 0;
  }
}
