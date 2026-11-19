#[path = "native/cry.rs"]
mod cry;
#[path = "native/mp2k.rs"]
mod mp2k;

use std::{env, fs, path::Path, process::Command};

fn main() {
    let core = Path::new("vendor/pokeemerald");
    assert!(
        core.join("tools/native/build.mk").is_file(),
        "initialize the pinned game sources with git submodule update --init --recursive"
    );
    assert_eq!(
        env::var_os("HOST"),
        env::var_os("TARGET"),
        "the native game build currently requires a host-target build"
    );
    assert!(
        env::var("CARGO_CFG_TARGET_POINTER_WIDTH").as_deref() == Ok("64")
            && env::var("CARGO_CFG_TARGET_ENDIAN").as_deref() == Ok("little")
            && env::var_os("CARGO_CFG_UNIX").is_some(),
        "Emerald's generated game modules require a little-endian 64-bit Unix host"
    );

    for input in [
        "tools/build-core.mk",
        "tools/rust-units.txt",
        "native/m4a.patch",
        "native/cry.patch",
        "native/reset.patch",
        "native/reset-keys.patch",
        "vendor/pokeemerald/Makefile",
        "vendor/pokeemerald/src",
        "vendor/pokeemerald/include",
        "vendor/pokeemerald/tools/native",
        "vendor/pokeemerald/graphics",
        "vendor/pokeemerald/sound",
        "vendor/pokeemerald/data",
        "vendor/pokeemerald/constants",
        "vendor/pokeemerald/charmap.txt",
    ] {
        println!("cargo:rerun-if-changed={input}");
    }

    let output = core.join("build/native");
    fs::create_dir_all(&output).expect("create the native build directory");
    // Profiles share the prepared C and converted assets. Only one make process
    // may update those outputs at a time, even with separate Cargo target dirs.
    let lock = fs::File::create(output.join("rustic-build.lock")).expect("open native build lock");
    lock.lock().expect("lock the native build directory");

    let jobs = env::var("NUM_JOBS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);
    let mut make = Command::new("make");
    let macos = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if macos {
        println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
        let version = Command::new(env::var_os("RUSTC").expect("Cargo sets RUSTC"))
            .args(["--print", "deployment-target", "--target"])
            .arg(env::var_os("TARGET").expect("Cargo sets TARGET"))
            .output()
            .expect("ask rustc for the macOS deployment target");
        assert!(
            version.status.success(),
            "rustc could not report its macOS deployment target"
        );
        let config = std::str::from_utf8(&version.stdout).expect("UTF-8 deployment target");
        let (_, version) = config
            .trim()
            .split_once('=')
            .expect("MACOSX_DEPLOYMENT_TARGET=value");
        make.env("MACOSX_DEPLOYMENT_TARGET", version);
    }
    let status = make
        .current_dir(core)
        .args([
            "-f",
            "Makefile",
            "-f",
            "../../tools/build-core.mk",
            "NODEP=1",
            "NATIVE_CFLAGS=-O2 -DNDEBUG",
        ])
        .arg(format!("-j{}", jobs.clamp(1, 8)))
        .arg("native-rustic")
        .status()
        .expect("run GNU make to build the native Emerald engine");
    assert!(status.success(), "native Emerald build failed: {status}");
    let output = output.canonicalize().expect("resolve native library path");
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=rustic_test_support");
    println!("cargo:rustc-link-lib=static:+whole-archive=rustic_core");
    println!("cargo:rustc-link-lib=m");
    if macos {
        println!("cargo:rustc-link-arg=-Wl,-no_fixup_chains");
    }
    mp2k::build();
    cry::build();
}
