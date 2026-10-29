# Third-party notices

IronSetup's original source is MIT licensed. It is an independent implementation based on publicly documented behavior; no Inno Setup compiler, source code, artwork or runtime is bundled. Inno Setup remains a separate project by its respective authors.

The portable binaries use Rust's standard library and Cargo dependencies pinned in `Cargo.lock`. The release bundle contains `third-party/dependencies.json` and copied upstream license/notice files under `third-party/licenses/`. License metadata is extracted from the installed package manifests; preserve those notices when redistributing.

Primary direct dependencies include serde / serde_json (MIT OR Apache-2.0), sha2 (MIT OR Apache-2.0), flate2 (MIT OR Apache-2.0), tempfile (MIT OR Apache-2.0), anyhow (MIT OR Apache-2.0), and windows / windows-sys (MIT OR Apache-2.0). Actual resolved package metadata is authoritative for the distributed versions. Do not infer an unused package is linked just because it appears in Cargo.lock.

Windows GNU executables are cross-compiled using Rust and MinGW-w64/GCC. The toolchain itself is not included. MinGW-w64 runtime notices and the GCC Runtime Library Exception are included in `third-party/toolchain/`, as applicable to the generated binaries. Linux executables dynamically depend on the host glibc and ordinary operating-system libraries; those system libraries are not bundled.

Source references: [Rust licensing](https://github.com/rust-lang/rust/tree/master/LICENSES), [MinGW-w64](https://www.mingw-w64.org/), [GCC Runtime Library Exception](https://www.gnu.org/licenses/gcc-exception-3.1.html). See copied license texts for the complete terms.
