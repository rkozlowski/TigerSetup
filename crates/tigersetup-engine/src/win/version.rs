//! The Windows build number, read through `RtlGetVersion`.
//!
//! `GetVersionEx` and the `VerifyVersionInfo` helpers lie to an executable
//! without a compatibility manifest entry for the running release;
//! `RtlGetVersion` reports what the machine actually runs, which is what a
//! platform baseline has to be decided from.

use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;

/// The running Windows build number, or `None` when the call fails.
pub fn build_number() -> Option<u32> {
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
    let status = unsafe { RtlGetVersion(&mut info) };
    (status >= 0).then_some(info.dwBuildNumber)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_number_is_at_least_the_engine_baseline() {
        let build = build_number().expect("RtlGetVersion answers on any supported Windows");
        assert!(
            build >= tigersetup_format::metadata::ENGINE_MINIMUM_BUILD,
            "this machine reports build {build}"
        );
    }
}
