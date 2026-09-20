//! The loader every generated TigerSetup `Setup.exe` begins with.
//!
//! The product of this package is `tigersetup-loader.exe`, a C Win32
//! executable that `build.rs` compiles and links into the profile
//! directory; this library holds nothing. A dependent's build script finds
//! the executable through `DEP_TIGERSETUP_LOADER_LOADER`, and the package's
//! own tests through `TIGERSETUP_LOADER_EXE`.
