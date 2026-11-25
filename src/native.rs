//! Safe wrappers around the Objective-C shim in `native/shim.m`.

use anyhow::{anyhow, Result};
use std::ffi::{c_char, c_double, c_int, CStr, CString};

extern "C" {
    fn ap_screen(dev_dir: *const c_char, udid: *const c_char, err: *mut *mut c_char) -> *mut c_char;
    fn ap_snapshot(
        dev_dir: *const c_char,
        udid: *const c_char,
        timeout_s: c_double,
        err: *mut *mut c_char,
    ) -> *mut c_char;
    fn ap_touch(
        dev_dir: *const c_char,
        udid: *const c_char,
        xs: *const c_double,
        ys: *const c_double,
        n: c_int,
        hold_ms: c_int,
        step_ms: c_int,
        err: *mut *mut c_char,
    ) -> c_int;
    fn ap_button(dev_dir: *const c_char, udid: *const c_char, button: c_int, err: *mut *mut c_char) -> c_int;
    fn ap_keys(
        dev_dir: *const c_char,
        udid: *const c_char,
        usages: *const u32,
        directions: *const u8,
        n: c_int,
        err: *mut *mut c_char,
    ) -> c_int;
    fn ap_free(s: *mut c_char);
}

/// The simulator a native call talks to.
pub struct Target {
    dev_dir: CString,
    udid: CString,
}

#[derive(Clone, Copy)]
pub enum Button {
    Home = 0,
    Lock = 1,
}

impl Target {
    pub fn new(dev_dir: &str, udid: &str) -> Result<Self> {
        Ok(Self { dev_dir: CString::new(dev_dir)?, udid: CString::new(udid)? })
    }

    pub fn screen(&self) -> Result<Screen> {
        let json = self.string_call(|err| unsafe { ap_screen(self.dev_dir.as_ptr(), self.udid.as_ptr(), err) })?;
        Ok(serde_json::from_str(&json)?)
    }

    /// The frontmost app's accessibility tree, as JSON.
    pub fn snapshot(&self, timeout_s: f64) -> Result<String> {
        self.string_call(|err| unsafe { ap_snapshot(self.dev_dir.as_ptr(), self.udid.as_ptr(), timeout_s, err) })
    }

    /// A finger down at `points[0]`, moved through the rest, and lifted at the last.
    pub fn touch(&self, points: &[(f64, f64)], hold_ms: u32, step_ms: u32) -> Result<()> {
        let xs: Vec<f64> = points.iter().map(|p| p.0).collect();
        let ys: Vec<f64> = points.iter().map(|p| p.1).collect();
        self.status_call(|err| unsafe {
            ap_touch(
                self.dev_dir.as_ptr(),
                self.udid.as_ptr(),
                xs.as_ptr(),
                ys.as_ptr(),
                points.len() as c_int,
                hold_ms as c_int,
                step_ms as c_int,
                err,
            )
        })
    }

    pub fn button(&self, button: Button) -> Result<()> {
        self.status_call(|err| unsafe { ap_button(self.dev_dir.as_ptr(), self.udid.as_ptr(), button as c_int, err) })
    }

    /// HID keyboard usages, each with a direction (1 = down, 2 = up).
    pub fn keys(&self, events: &[(u32, u8)]) -> Result<()> {
        let usages: Vec<u32> = events.iter().map(|e| e.0).collect();
        let directions: Vec<u8> = events.iter().map(|e| e.1).collect();
        self.status_call(|err| unsafe {
            ap_keys(
                self.dev_dir.as_ptr(),
                self.udid.as_ptr(),
                usages.as_ptr(),
                directions.as_ptr(),
                events.len() as c_int,
                err,
            )
        })
    }

    fn string_call(&self, call: impl FnOnce(*mut *mut c_char) -> *mut c_char) -> Result<String> {
        let mut err: *mut c_char = std::ptr::null_mut();
        let out = call(&mut err);
        if out.is_null() {
            return Err(take_error(err));
        }
        let text = unsafe { CStr::from_ptr(out) }.to_string_lossy().into_owned();
        unsafe { ap_free(out) };
        Ok(text)
    }

    fn status_call(&self, call: impl FnOnce(*mut *mut c_char) -> c_int) -> Result<()> {
        let mut err: *mut c_char = std::ptr::null_mut();
        if call(&mut err) == 0 {
            return Ok(());
        }
        Err(take_error(err))
    }
}

fn take_error(err: *mut c_char) -> anyhow::Error {
    if err.is_null() {
        return anyhow!("native call failed");
    }
    let message = unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned();
    unsafe { ap_free(err) };
    anyhow!(message)
}

#[derive(serde::Deserialize, Clone, Copy)]
pub struct Screen {
    pub width: f64,
    pub height: f64,
}
