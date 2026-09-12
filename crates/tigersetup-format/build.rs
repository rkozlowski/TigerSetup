//! Compiles `proto/tigersetup.proto` into Rust types with `protox` (a pure-Rust
//! Protocol Buffers compiler) and `prost-build`, so no machine needs `protoc`.

use std::path::PathBuf;

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let proto_dir = manifest_dir.join("..").join("..").join("proto");
    let proto = proto_dir.join("tigersetup.proto");
    println!("cargo:rerun-if-changed={}", proto.display());

    let descriptors = protox::compile([&proto], [&proto_dir]).expect("compile tigersetup.proto");
    prost_build::Config::new()
        .compile_fds(descriptors)
        .expect("generate Rust types from tigersetup.proto");
}
