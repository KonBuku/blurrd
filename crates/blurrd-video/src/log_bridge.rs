//! Routes FFmpeg's log output into `tracing`.
//!
//! FFmpeg calls the callback from its own threads (frame threads, the
//! demuxer). The callback must never unwind into C, so its body runs inside
//! `catch_unwind`; formatting uses FFmpeg's own `av_log_format_line2`, which
//! consumes the `va_list` without Rust having to interpret it.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

use ffmpeg_the_third::ffi;

/// Installs the callback. Messages below `AV_LOG_WARNING` are dropped by
/// FFmpeg before formatting.
pub(crate) fn install() {
    // SAFETY: both calls only store a level and a function pointer in
    // libavutil's globals; `callback` has the exact signature FFmpeg expects
    // and is valid for the whole program.
    unsafe {
        ffi::av_log_set_level(ffi::AV_LOG_WARNING as c_int);
        ffi::av_log_set_callback(Some(callback));
    }
}

/// The FFmpeg log callback.
unsafe extern "C" fn callback(
    avcl: *mut c_void,
    level: c_int,
    fmt: *const c_char,
    args: ffi::va_list,
) {
    // Formatting and `tracing` must not unwind into FFmpeg; a panic here is
    // dropped together with the message.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: FFmpeg checks the level itself before calling, but the
        // callback is global, so filter again.
        if level > unsafe { ffi::av_log_get_level() } {
            return;
        }
        let mut line = [0 as c_char; 1024];
        let mut print_prefix: c_int = 1;
        // SAFETY: `line` is a writable buffer of the given size; `avcl`,
        // `fmt` and `args` are passed through unchanged from FFmpeg, which
        // guarantees they match. The result is always NUL-terminated.
        let written = unsafe {
            ffi::av_log_format_line2(
                avcl,
                level,
                fmt,
                args,
                line.as_mut_ptr(),
                c_int::try_from(line.len()).unwrap_or(c_int::MAX),
                &raw mut print_prefix,
            )
        };
        if written < 0 {
            return;
        }
        // SAFETY: `av_log_format_line2` NUL-terminates `line`.
        let message = unsafe { CStr::from_ptr(line.as_ptr()) }.to_string_lossy();
        let message = message.trim_end();
        if message.is_empty() {
            return;
        }
        if level <= ffi::AV_LOG_ERROR as c_int {
            tracing::error!(target: "ffmpeg", "{message}");
        } else {
            tracing::warn!(target: "ffmpeg", "{message}");
        }
    }));
}
