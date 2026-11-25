// Draws ref labels over a screenshot, so an agent that looks at the picture can act on @e refs.

#import <CoreGraphics/CoreGraphics.h>
#import <CoreText/CoreText.h>
#import <Foundation/Foundation.h>
#import <ImageIO/ImageIO.h>

static void annotate_err(char **err, NSString *message) {
  if (err) {
    *err = strdup(message.UTF8String);
  }
}

static CGColorRef rgb(double r, double g, double b, double a) {
  CGColorSpaceRef space = CGColorSpaceCreateDeviceRGB();
  CGFloat parts[] = {r, g, b, a};
  CGColorRef color = CGColorCreate(space, parts);
  CGColorSpaceRelease(space);
  return color;
}

// `boxes_json` is [{"ref": "e3", "frame": [x, y, w, h]}] in screen points; `scale` is pixels per point.
int ap_annotate(const char *in_path, const char *out_path, const char *boxes_json, double scale, char **err) {
  @autoreleasepool {
    NSURL *inURL = [NSURL fileURLWithPath:@(in_path)];
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)inURL, NULL);
    if (!source) {
      annotate_err(err, @"could not read the screenshot");
      return 1;
    }
    CGImageRef image = CGImageSourceCreateImageAtIndex(source, 0, NULL);
    CFRelease(source);
    if (!image) {
      annotate_err(err, @"could not decode the screenshot");
      return 1;
    }
    NSData *data = [NSData dataWithBytes:boxes_json length:strlen(boxes_json)];
    NSArray *boxes = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    size_t width = CGImageGetWidth(image), height = CGImageGetHeight(image);
    CGColorSpaceRef space = CGColorSpaceCreateDeviceRGB();
    CGContextRef ctx = CGBitmapContextCreate(NULL, width, height, 8, 0, space, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
    CGColorSpaceRelease(space);
    if (!ctx) {
      CGImageRelease(image);
      annotate_err(err, @"could not make a drawing context");
      return 1;
    }
    CGContextDrawImage(ctx, CGRectMake(0, 0, width, height), image);
    CGImageRelease(image);

    CGColorRef stroke = rgb(1.0, 0.18, 0.33, 0.95);
    CGColorRef fill = rgb(1.0, 0.18, 0.33, 0.92);
    CGColorRef white = rgb(1, 1, 1, 1);
    CGFloat fontSize = MAX(11.0, 11.0 * scale);
    CTFontRef font = CTFontCreateWithName(CFSTR("Menlo-Bold"), fontSize, NULL);
    CGFloat line = MAX(1.5, scale);

    for (NSDictionary *box in [boxes isKindOfClass:NSArray.class] ? boxes : @[]) {
      NSArray *f = box[@"frame"];
      NSString *ref = box[@"ref"];
      if (![f isKindOfClass:NSArray.class] || f.count != 4 || ![ref isKindOfClass:NSString.class]) {
        continue;
      }
      // Flip from top-left points to bottom-left pixels.
      CGFloat x = [f[0] doubleValue] * scale, y = [f[1] doubleValue] * scale;
      CGFloat w = [f[2] doubleValue] * scale, h = [f[3] doubleValue] * scale;
      CGRect rect = CGRectMake(x, height - y - h, w, h);
      CGContextSetStrokeColorWithColor(ctx, stroke);
      CGContextSetLineWidth(ctx, line);
      CGContextStrokeRect(ctx, CGRectInset(rect, line / 2, line / 2));

      NSDictionary *attrs = @{(id)kCTFontAttributeName : (__bridge id)font, (id)kCTForegroundColorAttributeName : (__bridge id)white};
      NSAttributedString *text = [[NSAttributedString alloc] initWithString:ref attributes:attrs];
      CTLineRef ctLine = CTLineCreateWithAttributedString((__bridge CFAttributedStringRef)text);
      CGFloat ascent, descent;
      double textWidth = CTLineGetTypographicBounds(ctLine, &ascent, &descent, NULL);
      CGFloat pad = 2 * MAX(1.0, scale / 2);
      CGFloat tagW = textWidth + pad * 2, tagH = ascent + descent + pad * 2;
      // The tag sits on the box's top-left corner, inside the image.
      CGFloat tagX = MIN(MAX(0, rect.origin.x), width - tagW);
      CGFloat tagY = MIN(MAX(0, CGRectGetMaxY(rect) - tagH), height - tagH);
      CGContextSetFillColorWithColor(ctx, fill);
      CGContextFillRect(ctx, CGRectMake(tagX, tagY, tagW, tagH));
      CGContextSetTextPosition(ctx, tagX + pad, tagY + pad + descent);
      CTLineDraw(ctLine, ctx);
      CFRelease(ctLine);
    }
    CFRelease(font);
    CGColorRelease(stroke);
    CGColorRelease(fill);
    CGColorRelease(white);

    CGImageRef result = CGBitmapContextCreateImage(ctx);
    CGContextRelease(ctx);
    NSURL *outURL = [NSURL fileURLWithPath:@(out_path)];
    CGImageDestinationRef dest = CGImageDestinationCreateWithURL((__bridge CFURLRef)outURL, CFSTR("public.png"), 1, NULL);
    if (!dest) {
      CGImageRelease(result);
      annotate_err(err, [NSString stringWithFormat:@"could not write %s", out_path]);
      return 1;
    }
    CGImageDestinationAddImage(dest, result, NULL);
    BOOL ok = CGImageDestinationFinalize(dest);
    CFRelease(dest);
    CGImageRelease(result);
    if (!ok) {
      annotate_err(err, [NSString stringWithFormat:@"could not write %s", out_path]);
      return 1;
    }
    return 0;
  }
}

// Pixel width of a PNG, to work out its scale against the screen in points.
double ap_image_width(const char *path) {
  @autoreleasepool {
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:@(path)], NULL);
    if (!source) {
      return 0;
    }
    CGImageRef image = CGImageSourceCreateImageAtIndex(source, 0, NULL);
    CFRelease(source);
    if (!image) {
      return 0;
    }
    double w = (double)CGImageGetWidth(image);
    CGImageRelease(image);
    return w;
  }
}

static uint8_t *rgba_pixels(const char *path, size_t *w, size_t *h) {
  CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:@(path)], NULL);
  if (!source) {
    return NULL;
  }
  CGImageRef image = CGImageSourceCreateImageAtIndex(source, 0, NULL);
  CFRelease(source);
  if (!image) {
    return NULL;
  }
  *w = CGImageGetWidth(image);
  *h = CGImageGetHeight(image);
  uint8_t *pixels = calloc(*w * *h, 4);
  CGColorSpaceRef space = CGColorSpaceCreateDeviceRGB();
  CGContextRef ctx = CGBitmapContextCreate(pixels, *w, *h, 8, *w * 4, space, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
  CGColorSpaceRelease(space);
  CGContextDrawImage(ctx, CGRectMake(0, 0, *w, *h), image);
  CGContextRelease(ctx);
  CGImageRelease(image);
  return pixels;
}

// Compares two PNGs pixel by pixel. Writes `out_path` (if given): the second image faded, with
// changed pixels in red. Sets `ratio` to the share of pixels that changed.
int ap_image_diff(const char *a_path, const char *b_path, const char *out_path, int threshold, double *ratio, char **err) {
  @autoreleasepool {
    size_t aw, ah, bw, bh;
    uint8_t *a = rgba_pixels(a_path, &aw, &ah);
    uint8_t *b = rgba_pixels(b_path, &bw, &bh);
    if (!a || !b) {
      free(a);
      free(b);
      annotate_err(err, [NSString stringWithFormat:@"could not read %s", a ? b_path : a_path]);
      return 1;
    }
    if (aw != bw || ah != bh) {
      free(a);
      free(b);
      annotate_err(err, [NSString stringWithFormat:@"the images differ in size (%zux%zu and %zux%zu)", aw, ah, bw, bh]);
      return 1;
    }
    size_t changed = 0, n = aw * ah;
    for (size_t i = 0; i < n; i++) {
      uint8_t *pa = a + i * 4, *pb = b + i * 4;
      int d = 0;
      for (int c = 0; c < 3; c++) {
        d = MAX(d, abs((int)pa[c] - (int)pb[c]));
      }
      if (d > threshold) {
        changed++;
        pb[0] = 255, pb[1] = 30, pb[2] = 60, pb[3] = 255;
      } else {
        for (int c = 0; c < 3; c++) {
          pb[c] = (uint8_t)(255 - (255 - pb[c]) / 4);
        }
      }
    }
    *ratio = n ? (double)changed / (double)n : 0;
    free(a);
    int status = 0;
    if (out_path && *out_path) {
      CGColorSpaceRef space = CGColorSpaceCreateDeviceRGB();
      CGContextRef ctx = CGBitmapContextCreate(b, bw, bh, 8, bw * 4, space, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
      CGColorSpaceRelease(space);
      CGImageRef result = CGBitmapContextCreateImage(ctx);
      CGContextRelease(ctx);
      CGImageDestinationRef dest = CGImageDestinationCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:@(out_path)], CFSTR("public.png"), 1, NULL);
      if (dest) {
        CGImageDestinationAddImage(dest, result, NULL);
        status = CGImageDestinationFinalize(dest) ? 0 : 1;
        CFRelease(dest);
      } else {
        status = 1;
      }
      CGImageRelease(result);
      if (status) {
        annotate_err(err, [NSString stringWithFormat:@"could not write %s", out_path]);
      }
    }
    free(b);
    return status;
  }
}
