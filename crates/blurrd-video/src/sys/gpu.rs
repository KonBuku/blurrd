//! GPUs as reported by DXGI.
//!
//! Adapters are identified by their LUID, never by index: the order of
//! adapters changes with drivers, docking and power settings. The list is
//! ordered by `DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE`, so the first entry is
//! the GPU Windows would use for demanding work. Software adapters (WARP)
//! are skipped.

use blurrd_core::{GpuInfo, Luid};

/// PCI vendor id of NVIDIA.
pub const VENDOR_NVIDIA: u32 = 0x10de;
/// PCI vendor id of AMD.
pub const VENDOR_AMD: u32 = 0x1002;
/// PCI vendor id of Intel.
pub const VENDOR_INTEL: u32 = 0x8086;

/// A hardware GPU adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// LUID, name, dedicated memory and driver version.
    pub info: GpuInfo,
    /// PCI vendor id ([`VENDOR_NVIDIA`], [`VENDOR_AMD`], [`VENDOR_INTEL`]).
    pub vendor_id: u32,
}

impl Adapter {
    /// Whether NVDEC and NVENC can exist on this adapter.
    pub fn is_nvidia(&self) -> bool {
        self.vendor_id == VENDOR_NVIDIA
    }
}

/// Formats the user-mode driver version DXGI reports as `a.b.c.d`.
fn driver_version(umd: i64) -> String {
    let v = umd.cast_unsigned();
    format!(
        "{}.{}.{}.{}",
        v >> 48,
        (v >> 32) & 0xffff,
        (v >> 16) & 0xffff,
        v & 0xffff
    )
}

/// Combines the two halves of a Windows LUID.
fn luid(low: u32, high: i32) -> Luid {
    Luid((u64::from(high.cast_unsigned()) << 32) | u64::from(low))
}

/// The hardware GPUs of this machine, high-performance adapter first.
/// Returns an empty list when DXGI is unavailable (for example in a
/// session without graphics).
#[cfg(windows)]
pub fn adapters() -> Vec<Adapter> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
        IDXGIAdapter1, IDXGIDevice, IDXGIFactory6,
    };
    use windows::core::Interface;

    // SAFETY: creating a DXGI factory has no preconditions; the returned
    // COM interface is reference counted by the `windows` crate.
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory6>() }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for index in 0.. {
        // SAFETY: enumerating with an index; DXGI returns an error after the
        // last adapter.
        let Ok(adapter) = (unsafe {
            factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(
                index,
                DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
            )
        }) else {
            break;
        };
        // SAFETY: `adapter` is a valid interface; `GetDesc1` fills a plain
        // struct.
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        // SAFETY: as above; the GUID pointer refers to a static constant.
        let driver = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }
            .map(driver_version)
            .unwrap_or_default();
        let name_len = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        out.push(Adapter {
            info: GpuInfo {
                luid: luid(desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart),
                name: String::from_utf16_lossy(&desc.Description[..name_len]),
                vram: desc.DedicatedVideoMemory as u64,
                driver,
            },
            vendor_id: desc.VendorId,
        });
    }
    out
}

/// No DXGI outside Windows.
#[cfg(not(windows))]
pub fn adapters() -> Vec<Adapter> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::{driver_version, luid};

    #[test]
    fn formats_driver_versions_and_luids() {
        // 32.0.15.7242 is how Windows shows NVIDIA driver 572.42.
        let umd = (32i64 << 48) + (15 << 16) + 7242;
        assert_eq!(driver_version(umd), "32.0.15.7242");
        assert_eq!(luid(0x00a1_b2c3, 1).to_string(), "0000000100a1b2c3");
        assert_eq!(luid(u32::MAX, -1).0, u64::MAX);
    }

    #[test]
    fn lists_hardware_adapters() {
        for adapter in super::adapters() {
            assert_ne!(adapter.info.name, "");
            assert_ne!(adapter.info.luid.0, 0);
        }
    }
}
