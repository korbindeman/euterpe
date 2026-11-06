//! Memory layout of QEMU's mps2-an500: 4 MB of code RAM at 0, 4 MB of data RAM
//! at `0x2000_0000`.

use std::{env, fs, path::PathBuf};

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(
        out.join("memory.x"),
        "MEMORY\n{\n  FLASH : ORIGIN = 0x00000000, LENGTH = 4M\n  RAM : ORIGIN = 0x20000000, LENGTH = 4M\n}\n",
    )
    .expect("write memory.x");
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=build.rs");
}
