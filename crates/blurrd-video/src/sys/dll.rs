//! Where Windows looks for DLLs that blurrd loads at run time.
//!
//! The FFmpeg DLLs are linked at load time and Windows takes them from the
//! folder of the executable before any other place. ONNX Runtime, the
//! DirectML DLL it loads, and the drivers and system components that
//! FFmpeg and the GPU code load later are loaded at run time, by name or
//! by path. By default such a search also visits the current directory
//! and every folder on `PATH`, where another program's DLL of the same
//! name (an FFmpeg build, an older ONNX Runtime from a CUDA toolkit) could
//! be picked up. [`restrict_dll_search`] limits it to the folder of the
//! executable and the Windows system folder.

/// Restricts the DLL search of every later run-time load in this process
/// to the folder of the executable and the Windows system folder (no
/// current directory, no `PATH`). Call it first thing in `main`. Failure
/// is logged and ignored: the default search order still finds the
/// bundled DLLs first.
#[cfg(windows)]
pub fn restrict_dll_search() {
    use windows::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, SetDefaultDllDirectories,
    };

    // LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: the application folder, System32
    // and folders added with AddDllDirectory (blurrd adds none).
    // SAFETY: SetDefaultDllDirectories only changes the process-wide
    // search flags; it takes no pointers.
    if let Err(error) = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) } {
        tracing::warn!(%error, "could not restrict the DLL search path");
    }
}

/// Nothing to restrict outside Windows.
#[cfg(not(windows))]
pub fn restrict_dll_search() {}
