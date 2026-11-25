// Native bridge to CoreSimulator, SimulatorKit and AccessibilityPlatformTranslation.
//
// These are private Apple frameworks, loaded at runtime with dlopen and messaged through informal
// protocols, so nothing here links against them. The Indigo HID message layout and the
// AXPTranslator bridge pattern follow facebook/idb (MIT); see THIRD_PARTY_NOTICES.md.

#import "shim.h"

#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <dlfcn.h>
#import <mach/mach_time.h>
#import <malloc/malloc.h>

#pragma mark - Private API declarations

@protocol APSimServiceContext <NSObject>
+ (id)sharedServiceContextForDeveloperDir:(NSString *)dir error:(NSError **)error;
- (id)defaultDeviceSetWithError:(NSError **)error;
@end

@protocol APSimDeviceSet <NSObject>
- (NSArray *)devices;
@end

@protocol APSimDeviceType <NSObject>
- (CGSize)mainScreenSize;
- (float)mainScreenScale;
@end

@protocol APSimDevice <NSObject>
- (NSUUID *)UDID;
- (unsigned long long)state;
- (id)deviceType;
- (void)sendAccessibilityRequestAsync:(id)request
                      completionQueue:(dispatch_queue_t)queue
                    completionHandler:(void (^)(id response))handler;
@end

@protocol APHIDClient <NSObject>
- (instancetype)initWithDevice:(id)device error:(NSError **)error;
- (void)sendWithMessage:(void *)message
           freeWhenDone:(BOOL)freeWhenDone
        completionQueue:(dispatch_queue_t)queue
             completion:(void (^)(NSError *error))completion;
@end

@protocol APTranslationObject <NSObject>
@property (nonatomic, retain) NSString *bridgeDelegateToken;
@property (nonatomic) int pid;
@end

@protocol APTranslator <NSObject>
+ (id)sharedInstance;
@property (nonatomic, weak) id bridgeTokenDelegate;
@property (nonatomic) BOOL supportsDelegateTokens;
- (id)frontmostApplicationWithDisplayId:(unsigned int)displayId bridgeDelegateToken:(NSString *)token;
- (id)macPlatformElementFromTranslation:(id)translation;
@end

@protocol APTranslatorResponse <NSObject>
+ (id)emptyResponse;
@end

@protocol APPlatformElement <NSObject>
- (id)translation;
- (NSArray *)accessibilityChildren;
- (NSString *)accessibilityRole;
- (NSString *)accessibilitySubrole;
- (NSString *)accessibilityLabel;
- (id)accessibilityValue;
- (NSString *)accessibilityIdentifier;
- (NSString *)accessibilityPlaceholderValue;
- (NSString *)accessibilityTitle;
- (CGRect)accessibilityFrame;
- (BOOL)isAccessibilityEnabled;
- (BOOL)isAccessibilityFocused;
- (BOOL)isAccessibilitySelected;
@end

#pragma mark - Errors

static char *ap_strdup(NSString *s) { return strdup((s ?: @"").UTF8String); }

static void set_err(char **err, NSString *message) {
  if (err) {
    *err = ap_strdup(message);
  }
}

void ap_free(char *s) { free(s); }

#pragma mark - Frameworks and devices

static NSString *load_frameworks(NSString *devDir) {
  static NSString *failure;
  static dispatch_once_t once;
  dispatch_once(&once, ^{
    NSString *xcodeContents = [devDir stringByDeletingLastPathComponent];
    NSArray<NSArray<NSString *> *> *candidates = @[
      @[ @"/Library/Developer/PrivateFrameworks/CoreSimulator.framework/CoreSimulator" ],
      @[
        [xcodeContents stringByAppendingPathComponent:@"SharedFrameworks/SimulatorKit.framework/SimulatorKit"],
        [devDir stringByAppendingPathComponent:@"Library/PrivateFrameworks/SimulatorKit.framework/SimulatorKit"],
      ],
      @[ @"/System/Library/PrivateFrameworks/AccessibilityPlatformTranslation.framework/AccessibilityPlatformTranslation" ],
    ];
    for (NSArray<NSString *> *paths in candidates) {
      BOOL loaded = NO;
      NSString *lastError = nil;
      for (NSString *path in paths) {
        if (dlopen(path.fileSystemRepresentation, RTLD_NOW | RTLD_GLOBAL)) {
          loaded = YES;
          break;
        }
        const char *e = dlerror();
        lastError = e ? @(e) : path;
      }
      if (!loaded) {
        failure = [NSString stringWithFormat:@"could not load %@: %@", paths.firstObject.lastPathComponent, lastError];
        return;
      }
    }
  });
  return failure;
}

static id<APSimDevice> find_device(const char *dev_dir, const char *udid, char **err) {
  NSString *devDir = @(dev_dir);
  NSString *failure = load_frameworks(devDir);
  if (failure) {
    set_err(err, failure);
    return nil;
  }
  Class contextClass = NSClassFromString(@"SimServiceContext");
  if (!contextClass) {
    set_err(err, @"CoreSimulator has no SimServiceContext; is Xcode installed and opened once?");
    return nil;
  }
  NSError *error = nil;
  id context = [(id<APSimServiceContext>)contextClass sharedServiceContextForDeveloperDir:devDir error:&error];
  id set = [context defaultDeviceSetWithError:&error];
  if (!set) {
    set_err(err, [NSString stringWithFormat:@"could not open the simulator device set: %@", error.localizedDescription]);
    return nil;
  }
  NSUUID *wanted = [[NSUUID alloc] initWithUUIDString:@(udid)];
  for (id<APSimDevice> device in [(id<APSimDeviceSet>)set devices]) {
    if ([device.UDID isEqual:wanted]) {
      if (device.state != 3) {
        set_err(err, [NSString stringWithFormat:@"simulator %s is not booted", udid]);
        return nil;
      }
      return device;
    }
  }
  set_err(err, [NSString stringWithFormat:@"no simulator with UDID %s", udid]);
  return nil;
}

static CGSize screen_points(id<APSimDevice> device, float *scaleOut) {
  id<APSimDeviceType> type = device.deviceType;
  CGSize pixels = type.mainScreenSize;
  float scale = type.mainScreenScale ?: 1;
  if (scaleOut) {
    *scaleOut = scale;
  }
  return CGSizeMake(pixels.width / scale, pixels.height / scale);
}

char *ap_screen(const char *dev_dir, const char *udid, char **err) {
  @autoreleasepool {
    @try {
      id<APSimDevice> device = find_device(dev_dir, udid, err);
      if (!device) {
        return NULL;
      }
      float scale = 1;
      CGSize size = screen_points(device, &scale);
      return ap_strdup([NSString stringWithFormat:@"{\"width\":%g,\"height\":%g,\"scale\":%g}",
                                                  size.width, size.height, scale]);
    } @catch (NSException *exception) {
      set_err(err, exception.reason);
      return NULL;
    }
  }
}

#pragma mark - HID

typedef void *(*MouseFn)(CGPoint *, CGPoint *, uint32_t, NSUInteger, CGSize, uint32_t);
typedef void *(*ButtonFn)(int32_t, int32_t, int32_t);
typedef void *(*KeyboardFn)(int32_t, int32_t);

static void *simkit_symbol(const char *name) {
  // RTLD_DEFAULT finds the symbol once SimulatorKit has been loaded with RTLD_GLOBAL.
  return dlsym(RTLD_DEFAULT, name);
}

static id<APHIDClient> hid_client(id<APSimDevice> device, char **err) {
  Class clientClass = NSClassFromString(@"SimulatorKit.SimDeviceLegacyHIDClient");
  if (!clientClass) {
    set_err(err, @"SimulatorKit.SimDeviceLegacyHIDClient is not available in this Xcode");
    return nil;
  }
  NSError *error = nil;
  id<APHIDClient> client = [(id<APHIDClient>)[clientClass alloc] initWithDevice:device error:&error];
  if (!client) {
    set_err(err, [NSString stringWithFormat:@"could not connect to the simulator's HID service: %@",
                                            error.localizedDescription]);
  }
  return client;
}

// Sends one malloc'd message and waits for the simulator to acknowledge it.
static BOOL hid_send(id<APHIDClient> client, void *message, char **err) {
  static dispatch_queue_t queue;
  static dispatch_once_t once;
  dispatch_once(&once, ^{
    queue = dispatch_queue_create("agent-phone.hid", DISPATCH_QUEUE_SERIAL);
  });
  dispatch_semaphore_t done = dispatch_semaphore_create(0);
  __block NSError *failure = nil;
  [client sendWithMessage:message
             freeWhenDone:YES
          completionQueue:queue
               completion:^(NSError *error) {
                 failure = error;
                 dispatch_semaphore_signal(done);
               }];
  if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC)) != 0) {
    set_err(err, @"the simulator did not acknowledge the input within 5s");
    return NO;
  }
  if (failure) {
    set_err(err, [NSString stringWithFormat:@"the simulator rejected the input: %@", failure.localizedDescription]);
    return NO;
  }
  return YES;
}

// A single-touch message. SimulatorKit only builds multi-touch messages, so this takes the touch
// payload from its builder and re-envelopes it as a single-touch message, as idb does.
static void *touch_message(MouseFn mouse, CGPoint ratio, BOOL down) {
  CGPoint point = ratio;
  uint8_t *source = mouse(&point, NULL, 0x32, down ? 1 : 2, CGSizeMake(1, 1), 0);
  if (!source) {
    return NULL;
  }
  memcpy(source + 0x3c, &ratio.x, sizeof(double));
  memcpy(source + 0x44, &ratio.y, sizeof(double));

  const size_t payloadSize = 0x90;
  const size_t messageSize = 0xB0 + payloadSize;
  const size_t touchSize = 0x70;
  uint8_t *message = calloc(1, messageSize);
  uint32_t innerSize = (uint32_t)payloadSize;
  memcpy(message + 0x18, &innerSize, 4);
  message[0x1c] = 2;  // single touch
  uint32_t eventKind = 0xB;
  memcpy(message + 0x20, &eventKind, 4);
  uint64_t timestamp = mach_absolute_time();
  memcpy(message + 0x24, &timestamp, 8);
  memcpy(message + 0x30, source + 0x30, touchSize);
  free(source);

  memcpy(message + 0x20 + payloadSize, message + 0x20, payloadSize);
  uint32_t one = 1, two = 2;
  memcpy(message + 0x20 + payloadSize + 0x10, &one, 4);
  memcpy(message + 0x20 + payloadSize + 0x14, &two, 4);
  return message;
}

int ap_touch(const char *dev_dir, const char *udid, const double *xs, const double *ys, int n,
             int hold_ms, int step_ms, char **err) {
  @autoreleasepool {
    @try {
      if (n < 1) {
        set_err(err, @"a touch needs at least one point");
        return 1;
      }
      id<APSimDevice> device = find_device(dev_dir, udid, err);
      if (!device) {
        return 1;
      }
      MouseFn mouse = (MouseFn)simkit_symbol("IndigoHIDMessageForMouseNSEvent");
      if (!mouse) {
        set_err(err, @"IndigoHIDMessageForMouseNSEvent is missing from SimulatorKit");
        return 1;
      }
      id<APHIDClient> client = hid_client(device, err);
      if (!client) {
        return 1;
      }
      CGSize size = screen_points(device, NULL);
      for (int i = 0; i < n; i++) {
        CGPoint ratio = CGPointMake(xs[i] / size.width, ys[i] / size.height);
        if (!hid_send(client, touch_message(mouse, ratio, YES), err)) {
          return 1;
        }
        int wait = i == 0 ? hold_ms : step_ms;
        if (wait > 0) {
          usleep((useconds_t)wait * 1000);
        }
      }
      CGPoint last = CGPointMake(xs[n - 1] / size.width, ys[n - 1] / size.height);
      return hid_send(client, touch_message(mouse, last, NO), err) ? 0 : 1;
    } @catch (NSException *exception) {
      set_err(err, exception.reason);
      return 1;
    }
  }
}

int ap_button(const char *dev_dir, const char *udid, int button, char **err) {
  @autoreleasepool {
    @try {
      id<APSimDevice> device = find_device(dev_dir, udid, err);
      if (!device) {
        return 1;
      }
      ButtonFn make = (ButtonFn)simkit_symbol("IndigoHIDMessageForButton");
      if (!make) {
        set_err(err, @"IndigoHIDMessageForButton is missing from SimulatorKit");
        return 1;
      }
      id<APHIDClient> client = hid_client(device, err);
      if (!client) {
        return 1;
      }
      int32_t source = button == 1 ? 0x1 : 0x0;
      if (!hid_send(client, make(source, 1, 0x33), err)) {
        return 1;
      }
      usleep(80 * 1000);
      return hid_send(client, make(source, 2, 0x33), err) ? 0 : 1;
    } @catch (NSException *exception) {
      set_err(err, exception.reason);
      return 1;
    }
  }
}

int ap_keys(const char *dev_dir, const char *udid, const uint32_t *usages,
            const uint8_t *directions, int n, char **err) {
  @autoreleasepool {
    @try {
      id<APSimDevice> device = find_device(dev_dir, udid, err);
      if (!device) {
        return 1;
      }
      KeyboardFn make = (KeyboardFn)simkit_symbol("IndigoHIDMessageForKeyboardArbitrary");
      if (!make) {
        set_err(err, @"IndigoHIDMessageForKeyboardArbitrary is missing from SimulatorKit");
        return 1;
      }
      id<APHIDClient> client = hid_client(device, err);
      if (!client) {
        return 1;
      }
      for (int i = 0; i < n; i++) {
        if (!hid_send(client, make((int32_t)usages[i], directions[i]), err)) {
          return 1;
        }
        usleep(4 * 1000);
      }
      return 0;
    } @catch (NSException *exception) {
      set_err(err, exception.reason);
      return 1;
    }
  }
}

#pragma mark - Accessibility

// Answers AXPTranslator's requests for element data by forwarding them to the simulator through
// CoreSimulator and waiting for the reply.
@interface APTranslationBridge : NSObject
@property (nonatomic, strong) id<APSimDevice> device;
@property (nonatomic, assign) double timeout;
@property (nonatomic, strong) dispatch_queue_t replyQueue;
@end

@implementation APTranslationBridge

- (id (^)(id))callback {
  id<APSimDevice> device = self.device;
  dispatch_queue_t queue = self.replyQueue;
  double timeout = self.timeout;
  return ^id(id request) {
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block id response = nil;
    [device sendAccessibilityRequestAsync:request
                          completionQueue:queue
                        completionHandler:^(id reply) {
                          response = reply;
                          dispatch_semaphore_signal(done);
                        }];
    if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(timeout * NSEC_PER_SEC))) != 0) {
      return [(id<APTranslatorResponse>)NSClassFromString(@"AXPTranslatorResponse") emptyResponse];
    }
    return response;
  };
}

- (id)accessibilityTranslationDelegateBridgeCallbackWithToken:(NSString *)token { return [self callback]; }
- (id)accessibilityTranslationDelegateBridgeCallback { return [self callback]; }
- (CGRect)accessibilityTranslationConvertPlatformFrameToSystem:(CGRect)rect withToken:(NSString *)token { return rect; }
- (CGRect)accessibilityTranslationConvertPlatformFrameToSystem:(CGRect)rect withContext:(id)c postProcess:(id)p { return rect; }
- (id)accessibilityTranslationRootParentWithToken:(NSString *)token { return nil; }
- (id)accessibilityTranslationRootParent { return nil; }

@end

static NSString *const kToken = @"agent-phone";

static id string_or_null(id value) {
  if ([value isKindOfClass:NSString.class]) {
    return [(NSString *)value length] ? value : NSNull.null;
  }
  if ([value isKindOfClass:NSNumber.class]) {
    return [(NSNumber *)value stringValue];
  }
  if ([value isKindOfClass:NSAttributedString.class]) {
    return [(NSAttributedString *)value string];
  }
  return NSNull.null;
}

static void tag(id element) {
  @try {
    id translation = [(id<APPlatformElement>)element translation];
    [(id<APTranslationObject>)translation setBridgeDelegateToken:kToken];
  } @catch (NSException *exception) {
  }
}

static NSDictionary *describe(id<APPlatformElement> element, int depth) {
  tag(element);
  NSMutableDictionary *node = [NSMutableDictionary dictionary];
  node[@"role"] = string_or_null(element.accessibilityRole);
  node[@"subrole"] = string_or_null(element.accessibilitySubrole);
  node[@"label"] = string_or_null(element.accessibilityLabel);
  node[@"value"] = string_or_null(element.accessibilityValue);
  node[@"identifier"] = string_or_null(element.accessibilityIdentifier);
  node[@"placeholder"] = string_or_null(
      [element respondsToSelector:@selector(accessibilityPlaceholderValue)] ? element.accessibilityPlaceholderValue : nil);
  node[@"enabled"] = @([element respondsToSelector:@selector(isAccessibilityEnabled)] ? element.isAccessibilityEnabled : YES);
  node[@"focused"] = @([element respondsToSelector:@selector(isAccessibilityFocused)] ? element.isAccessibilityFocused : NO);
  node[@"selected"] = @([element respondsToSelector:@selector(isAccessibilitySelected)] ? element.isAccessibilitySelected : NO);
  CGRect frame = element.accessibilityFrame;
  node[@"frame"] = @[ @(frame.origin.x), @(frame.origin.y), @(frame.size.width), @(frame.size.height) ];
  NSMutableArray *children = [NSMutableArray array];
  if (depth < 64) {
    for (id child in element.accessibilityChildren ?: @[]) {
      [children addObject:describe(child, depth + 1)];
    }
  }
  node[@"children"] = children;
  return node;
}

char *ap_snapshot(const char *dev_dir, const char *udid, double timeout_s, char **err) {
  @autoreleasepool {
    id<APSimDevice> device = find_device(dev_dir, udid, err);
    if (!device) {
      return NULL;
    }
    if (![device respondsToSelector:@selector(sendAccessibilityRequestAsync:completionQueue:completionHandler:)]) {
      set_err(err, @"this CoreSimulator has no accessibility request API");
      return NULL;
    }
    Class translatorClass = NSClassFromString(@"AXPTranslator");
    if (!translatorClass) {
      set_err(err, @"AXPTranslator is not available on this Mac");
      return NULL;
    }

    static APTranslationBridge *bridge;  // the translator only holds it weakly
    bridge = [APTranslationBridge new];
    bridge.device = device;
    bridge.timeout = timeout_s > 0 ? timeout_s : 5;
    bridge.replyQueue = dispatch_queue_create("agent-phone.ax.reply", DISPATCH_QUEUE_SERIAL);

    // The translator asserts it is not on the main queue, and its callbacks block, so walk the tree
    // on a worker queue and wait here.
    __block char *result = NULL;
    __block NSString *failure = nil;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
      @autoreleasepool {
        @try {
          id<APTranslator> translator = [(id<APTranslator>)translatorClass sharedInstance];
          translator.bridgeTokenDelegate = bridge;
          if ([translator respondsToSelector:@selector(setSupportsDelegateTokens:)]) {
            translator.supportsDelegateTokens = YES;
          }
          id app = [translator frontmostApplicationWithDisplayId:0 bridgeDelegateToken:kToken];
          if (!app) {
            failure = @"no frontmost application; is the simulator still starting up?";
          } else {
            [(id<APTranslationObject>)app setBridgeDelegateToken:kToken];
            id root = [translator macPlatformElementFromTranslation:app];
            if (!root) {
              failure = @"could not read the frontmost application's accessibility element";
            } else {
              NSMutableDictionary *tree = [describe(root, 0) mutableCopy];
              tree[@"pid"] = @([(id<APTranslationObject>)app pid]);
              NSData *json = [NSJSONSerialization dataWithJSONObject:tree options:0 error:nil];
              result = strdup([[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding].UTF8String);
            }
          }
        } @catch (NSException *exception) {
          failure = [NSString stringWithFormat:@"accessibility read failed: %@", exception.reason];
        }
      }
      dispatch_semaphore_signal(done);
    });
    dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
    if (!result) {
      set_err(err, failure ?: @"accessibility read failed");
    }
    return result;
  }
}

#pragma mark - Framebuffer

#import <IOSurface/IOSurface.h>
#import <ImageIO/ImageIO.h>

// The main display's framebuffer, from the device's IO ports (as Simulator.app shows it).
static IOSurfaceRef main_surface(id<APSimDevice> device, char **err) {
  id io = [(id)device respondsToSelector:@selector(io)] ? [(id)device performSelector:@selector(io)] : nil;
  NSArray *ports = [io respondsToSelector:@selector(ioPorts)] ? [io performSelector:@selector(ioPorts)] : nil;
  IOSurfaceRef best = NULL;
  size_t bestArea = 0;
  for (id port in ports ?: @[]) {
    id descriptor = [port respondsToSelector:@selector(descriptor)] ? [port performSelector:@selector(descriptor)] : nil;
    for (NSString *name in @[ @"framebufferSurface", @"ioSurface" ]) {
      SEL sel = NSSelectorFromString(name);
      if (![descriptor respondsToSelector:sel]) {
        continue;
      }
      IOSurfaceRef (*getter)(id, SEL) = (IOSurfaceRef(*)(id, SEL))[descriptor methodForSelector:sel];
      IOSurfaceRef surface = getter(descriptor, sel);
      if (!surface) {
        continue;
      }
      size_t area = IOSurfaceGetWidth(surface) * IOSurfaceGetHeight(surface);
      if (area > bestArea) {
        best = surface;
        bestArea = area;
      }
    }
  }
  if (!best) {
    set_err(err, @"the simulator exposes no display surface");
  }
  return best;
}

// The screen as an image file: JPEG at `quality` (0-1), or PNG when quality <= 0.
// Returns a malloc'd buffer and sets *len; free it with ap_free.
char *ap_frame(const char *dev_dir, const char *udid, double quality, size_t *len, char **err) {
  @autoreleasepool {
    @try {
      id<APSimDevice> device = find_device(dev_dir, udid, err);
      if (!device) {
        return NULL;
      }
      IOSurfaceRef surface = main_surface(device, err);
      if (!surface) {
        return NULL;
      }
      CFRetain(surface);
      IOSurfaceLock(surface, kIOSurfaceLockReadOnly, NULL);
      size_t w = IOSurfaceGetWidth(surface), h = IOSurfaceGetHeight(surface), row = IOSurfaceGetBytesPerRow(surface);
      CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
      CGContextRef ctx = CGBitmapContextCreate(IOSurfaceGetBaseAddress(surface), w, h, 8, row, space,
                                               kCGBitmapByteOrder32Little | (CGBitmapInfo)kCGImageAlphaPremultipliedFirst);
      CGImageRef image = ctx ? CGBitmapContextCreateImage(ctx) : NULL;
      if (ctx) {
        CGContextRelease(ctx);
      }
      CGColorSpaceRelease(space);
      IOSurfaceUnlock(surface, kIOSurfaceLockReadOnly, NULL);
      CFRelease(surface);
      if (!image) {
        set_err(err, @"could not read the display surface");
        return NULL;
      }
      NSMutableData *data = [NSMutableData data];
      CFStringRef type = quality > 0 ? CFSTR("public.jpeg") : CFSTR("public.png");
      CGImageDestinationRef dest = CGImageDestinationCreateWithData((__bridge CFMutableDataRef)data, type, 1, NULL);
      NSDictionary *props = quality > 0 ? @{(id)kCGImageDestinationLossyCompressionQuality : @(quality)} : @{};
      CGImageDestinationAddImage(dest, image, (__bridge CFDictionaryRef)props);
      BOOL ok = CGImageDestinationFinalize(dest);
      CFRelease(dest);
      CGImageRelease(image);
      if (!ok) {
        set_err(err, @"could not encode the frame");
        return NULL;
      }
      char *out = malloc(data.length);
      memcpy(out, data.bytes, data.length);
      *len = data.length;
      return out;
    } @catch (NSException *exception) {
      set_err(err, exception.reason);
      return NULL;
    }
  }
}
