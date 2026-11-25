//! `screenshot --annotate`: ref labels drawn over the picture (see `native/annotate.m`).

use crate::snapshot::Element;
use anyhow::{anyhow, Result};
use std::ffi::{c_char, c_double, c_int, CStr, CString};

extern "C" {
    fn ap_annotate(input: *const c_char, output: *const c_char, boxes: *const c_char, scale: c_double, err: *mut *mut c_char) -> c_int;
    fn ap_image_width(path: *const c_char) -> c_double;
    fn ap_free(s: *mut c_char);
}

/// Draws a box and its ref over each element in the PNG at `path`, in place. `screen_width` is
/// the screen's width in points, which sets how points map to the image's pixels.
pub fn draw(path: &str, elements: &[&Element], screen_width: f64) -> Result<()> {
    let c_path = CString::new(path)?;
    let pixels = unsafe { ap_image_width(c_path.as_ptr()) };
    if pixels <= 0.0 || screen_width <= 0.0 {
        return Err(anyhow!("could not read the screenshot at {path}"));
    }
    let boxes: Vec<_> = elements.iter().map(|e| serde_json::json!({ "ref": e.reference, "frame": e.frame })).collect();
    let boxes = CString::new(serde_json::to_string(&boxes)?)?;
    let mut err: *mut c_char = std::ptr::null_mut();
    if unsafe { ap_annotate(c_path.as_ptr(), c_path.as_ptr(), boxes.as_ptr(), pixels / screen_width, &mut err) } == 0 {
        return Ok(());
    }
    if err.is_null() {
        return Err(anyhow!("could not annotate the screenshot"));
    }
    let message = unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned();
    unsafe { ap_free(err) };
    Err(anyhow!(message))
}

extern "C" {
    fn ap_image_diff(a: *const c_char, b: *const c_char, out: *const c_char, threshold: c_int, ratio: *mut c_double, err: *mut *mut c_char) -> c_int;
}

/// The share of pixels (0 to 1) that differ between two PNGs of the same size. With `out`, also
/// writes a picture of the second image with the changes in red.
pub fn diff(a: &str, b: &str, out: Option<&str>, threshold: u8) -> Result<f64> {
    let (a, b, out) = (CString::new(a)?, CString::new(b)?, CString::new(out.unwrap_or(""))?);
    let mut ratio = 0.0;
    let mut err: *mut c_char = std::ptr::null_mut();
    if unsafe { ap_image_diff(a.as_ptr(), b.as_ptr(), out.as_ptr(), threshold as c_int, &mut ratio, &mut err) } == 0 {
        return Ok(ratio);
    }
    if err.is_null() {
        return Err(anyhow!("could not compare the images"));
    }
    let message = unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned();
    unsafe { ap_free(err) };
    Err(anyhow!(message))
}
