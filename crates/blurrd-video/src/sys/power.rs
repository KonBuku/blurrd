//! Keeping Windows from slowing a running job down.
//!
//! Windows 11 throttles processes it considers background work (EcoQoS):
//! their threads run on efficiency cores or at lower clocks, which halves
//! an analysis or an export on a laptop, especially while blurrd's window
//! is minimised. It also lets the machine sleep during a long export when
//! nobody touches it. A [`JobPower`] guard covers one job: while any guard
//! lives, the process opts out of execution-speed throttling, and while
//! any guard of a thread lives, that thread tells Windows that the system
//! is required (`ES_SYSTEM_REQUIRED`; the display may still turn off).
//! Dropping the last guard (of the process, of the thread) hands each
//! decision back to Windows.

use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::{Mutex, PoisonError};

/// Guards alive in this process.
static ACTIVE: Mutex<usize> = Mutex::new(0);

thread_local! {
    /// Guards alive on this thread: the execution state is per thread, so
    /// only the thread's first guard sets it and its last one clears it.
    static ON_THREAD: Cell<usize> = const { Cell::new(0) };
}

/// Keeps the machine awake and the process unthrottled while it lives.
///
/// The execution state belongs to the thread that set it, so the guard
/// stays on the thread that created it (it is not `Send`; it is `Sync`, so
/// a job can lend it to its worker threads).
#[derive(Debug)]
#[must_use = "the job runs unthrottled only while the guard lives"]
pub struct JobPower {
    _thread: PhantomData<std::sync::MutexGuard<'static, ()>>,
}

impl JobPower {
    /// Opts the process out of throttling (if no other job did) and keeps
    /// the system awake while the current thread runs the job. Failures
    /// are logged and ignored: the job runs either way, only perhaps
    /// slower.
    pub fn begin() -> Self {
        let mut active = ACTIVE.lock().unwrap_or_else(PoisonError::into_inner);
        if *active == 0 {
            set_throttling(false);
        }
        *active += 1;
        if ON_THREAD.replace(ON_THREAD.get() + 1) == 0 {
            keep_awake(true);
        }
        Self {
            _thread: PhantomData,
        }
    }
}

impl Drop for JobPower {
    fn drop(&mut self) {
        let on_thread = ON_THREAD.get().saturating_sub(1);
        ON_THREAD.set(on_thread);
        if on_thread == 0 {
            keep_awake(false);
        }
        let mut active = ACTIVE.lock().unwrap_or_else(PoisonError::into_inner);
        *active = active.saturating_sub(1);
        if *active == 0 {
            set_throttling(true);
        }
    }
}

/// Lets Windows throttle the process's execution speed as it sees fit
/// (`allowed`), or opts out of it.
#[cfg(windows)]
fn set_throttling(allowed: bool) {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
        ProcessPowerThrottling, SetProcessInformation,
    };

    // ControlMask 0 hands the decision back to the system; the bit in
    // ControlMask with StateMask 0 turns throttling off.
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: if allowed {
            0
        } else {
            PROCESS_POWER_THROTTLING_EXECUTION_SPEED
        },
        StateMask: 0,
    };
    // SAFETY: the pseudo handle of the current process needs no closing;
    // `state` is a valid PROCESS_POWER_THROTTLING_STATE that outlives the
    // call, and its size is passed with it.
    let result = unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            std::ptr::from_ref(&state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    };
    if let Err(error) = result {
        tracing::debug!(%error, allowed, "could not change power throttling");
    }
}

/// No power throttling outside Windows.
#[cfg(not(windows))]
fn set_throttling(_allowed: bool) {}

/// Tells Windows that the system is required (`awake`) or no longer
/// required by the current thread.
#[cfg(windows)]
fn keep_awake(awake: bool) {
    use windows::Win32::System::Power::{
        ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

    let flags = if awake {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    };
    // SAFETY: SetThreadExecutionState only records flags for the calling
    // thread.
    let previous = unsafe { SetThreadExecutionState(flags) };
    if previous.0 == 0 {
        tracing::debug!(awake, "could not change the thread's execution state");
    }
}

/// No execution state outside Windows.
#[cfg(not(windows))]
fn keep_awake(_awake: bool) {}

#[cfg(test)]
mod tests {
    use super::{ACTIVE, JobPower, ON_THREAD};

    fn active() -> usize {
        *ACTIVE.lock().unwrap()
    }

    #[test]
    fn guards_nest_and_hand_back_on_the_last_drop() {
        let before = active();
        let outer = JobPower::begin();
        let inner = JobPower::begin();
        assert_eq!(active(), before + 2);
        assert_eq!(ON_THREAD.get(), 2);
        drop(inner);
        // The outer guard still keeps the thread's execution state.
        assert_eq!(active(), before + 1);
        assert_eq!(ON_THREAD.get(), 1);
        // A guard on another thread counts for the process only.
        std::thread::spawn(|| {
            let other = JobPower::begin();
            assert_eq!(ON_THREAD.get(), 1);
            drop(other);
            assert_eq!(ON_THREAD.get(), 0);
        })
        .join()
        .unwrap();
        assert_eq!(ON_THREAD.get(), 1);
        drop(outer);
        assert_eq!(active(), before);
        assert_eq!(ON_THREAD.get(), 0);
    }
}
