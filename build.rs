//! Builds transcribe.cpp (vendored as a submodule) as static libraries with
//! `cmake`, then compiles the two C shims and links everything.
//!
//! Environment:
//! - `TRANSCRIBE_DIR`: use this transcribe.cpp checkout instead of the submodule.
//! - `WALKIE_GGML_NATIVE=0`: build ggml for any x86-64 with AVX2/FMA/F16C
//!   (Haswell or newer) instead of `-march=native`. Use it when the binary
//!   will run on a different CPU than the one that builds it.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let tcpp = env::var_os("TRANSCRIBE_DIR").map_or_else(|| manifest.join("vendor/transcribe.cpp"), PathBuf::from);
    assert!(
        tcpp.join("CMakeLists.txt").exists(),
        "transcribe.cpp not found at {}; run `git submodule update --init`",
        tcpp.display()
    );
    println!("cargo:rerun-if-env-changed=TRANSCRIBE_DIR");
    println!("cargo:rerun-if-env-changed=WALKIE_GGML_NATIVE");
    println!("cargo:rerun-if-changed=csrc");
    // Rebuild transcribe.cpp when the submodule moves or its sources change.
    for sub in ["CMakeLists.txt", "include", "src", "ggml"] {
        println!("cargo:rerun-if-changed={}", tcpp.join(sub).display());
    }
    let gitlink = manifest.join(".git/modules/vendor/transcribe.cpp/HEAD");
    if gitlink.exists() {
        println!("cargo:rerun-if-changed={}", gitlink.display());
    }

    let mut cfg = cmake::Config::new(&tcpp);
    cfg.profile("Release")
        .define("TRANSCRIBE_BUILD_TESTS", "OFF")
        .define("TRANSCRIBE_BUILD_EXAMPLES", "OFF")
        .define("TRANSCRIBE_BUILD_SHARED", "OFF")
        .define("BUILD_SHARED_LIBS", "OFF")
        .build_target("transcribe");
    if env::var("WALKIE_GGML_NATIVE").is_ok_and(|v| v == "0") {
        for (k, v) in
            [("GGML_NATIVE", "OFF"), ("GGML_AVX", "ON"), ("GGML_AVX2", "ON"), ("GGML_FMA", "ON"), ("GGML_F16C", "ON")]
        {
            cfg.define(k, v);
        }
    }
    let out = cfg.build().join("build");
    for dir in ["src", "ggml/src"] {
        println!("cargo:rustc-link-search=native={}", out.join(dir).display());
    }
    for lib in ["transcribe", "ggml", "ggml-cpu", "ggml-base"] {
        println!("cargo:rustc-link-lib=static={lib}");
    }

    cc::Build::new()
        .file("csrc/engine.c")
        .include(tcpp.join("include"))
        .define("TRANSCRIBE_STATIC", None)
        .std("c11")
        .warnings(true)
        .compile("walkie_engine");

    let pw = pkg_config::Config::new()
        .atleast_version("0.3.40")
        .probe("libpipewire-0.3")
        .expect("libpipewire-0.3 development files (Fedora: pipewire-devel)");
    let mut pwc = cc::Build::new();
    pwc.file("csrc/pw.c").std("gnu11").warnings(true);
    for inc in &pw.include_paths {
        pwc.include(inc);
    }
    pwc.compile("walkie_pw");

    // ggml is C++; link its runtime after the static archives.
    println!("cargo:rustc-link-lib=dylib=stdc++");
}
