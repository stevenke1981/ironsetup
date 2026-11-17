//! Embeds `ironsetup.manifest` (asInvoker, ComCtl32 v6, PerMonitorV2 DPI,
//! Windows 10/11, UTF-8 code page) into every binary of this crate, without
//! needing a resource compiler:
//! - MSVC: link.exe embeds it (`/MANIFEST:EMBED /MANIFESTINPUT`).
//! - GNU (MinGW, also cross builds): this script writes a COFF object with a
//!   `.rsrc` section holding RT_MANIFEST #1, which ld links as the resource
//!   directory, exactly like a windres-generated object.
//!   Other toolchains still get ComCtl32 v6 from the UI's runtime activation
//!   context.
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=ironsetup.manifest");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("ironsetup.manifest");
    match env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => {
            println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
            println!(
                "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
                manifest.display()
            );
            // The manifest carries its own trustInfo (asInvoker).
            println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");
        }
        Ok("gnu") => {
            let machine_and_relocation = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
                // IMAGE_FILE_MACHINE_*, IMAGE_REL_*_ADDR32NB
                Ok("x86_64") => Some((0x8664u16, 3u16)),
                Ok("x86") => Some((0x014c, 7)),
                Ok("aarch64") => Some((0xaa64, 2)),
                _ => None,
            };
            let Some((machine, relocation)) = machine_and_relocation else {
                return;
            };
            let xml = fs::read(&manifest).expect("read ironsetup.manifest");
            let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
            let object = out.join("ironsetup-manifest.o");
            fs::write(&object, resource_object(machine, relocation, &xml))
                .expect("write manifest resource object");
            println!("cargo:rustc-link-arg-bins={}", object.display());
            // MinGW GCC links its own default-manifest.o (a startfile found via
            // the -B search path). ld only drops a competing manifest whose
            // language is 0, and some toolchains (e.g. WinLibs) ship it as
            // en-US, which fails with "multiple non-default manifests".
            // Shadow it with an empty object, searched before the toolchain.
            let shadow = out.join("no-default-manifest");
            fs::create_dir_all(&shadow).expect("create manifest shadow directory");
            fs::write(shadow.join("default-manifest.o"), empty_object(machine))
                .expect("write empty default-manifest.o");
            // GCC prepends the -B prefix verbatim, so it needs a separator.
            println!(
                "cargo:rustc-link-arg-bins=-B{}{}",
                shadow.display(),
                std::path::MAIN_SEPARATOR
            );
        }
        _ => {}
    }
}

/// A COFF object with no sections and no symbols.
fn empty_object(machine: u16) -> Vec<u8> {
    let mut object = Vec::new();
    object.extend_from_slice(&machine.to_le_bytes());
    object.extend_from_slice(&0u16.to_le_bytes()); // sections
    object.extend_from_slice(&0u32.to_le_bytes()); // timestamp
    object.extend_from_slice(&20u32.to_le_bytes()); // symbol table offset
    object.extend_from_slice(&0u32.to_le_bytes()); // symbols
    object.extend_from_slice(&0u16.to_le_bytes()); // optional header size
    object.extend_from_slice(&0u16.to_le_bytes()); // characteristics
    object.extend_from_slice(&4u32.to_le_bytes()); // empty string table
    object
}

/// A COFF object whose `.rsrc` section is a resource directory with one
/// entry, RT_MANIFEST (24) / ID 1 / en-US, pointing at `data`.
fn resource_object(machine: u16, relocation: u16, data: &[u8]) -> Vec<u8> {
    const RT_MANIFEST: u32 = 24;
    const LANGUAGE: u32 = 0x0409;
    fn directory(section: &mut Vec<u8>, id: u32, target: u32) {
        // IMAGE_RESOURCE_DIRECTORY with one ID entry.
        section.extend_from_slice(&[0; 12]); // characteristics, time, version
        section.extend_from_slice(&0u16.to_le_bytes()); // named entries
        section.extend_from_slice(&1u16.to_le_bytes()); // ID entries
        section.extend_from_slice(&id.to_le_bytes());
        section.extend_from_slice(&target.to_le_bytes());
    }
    let mut section = Vec::new();
    const SUBDIRECTORY: u32 = 0x8000_0000;
    directory(&mut section, RT_MANIFEST, SUBDIRECTORY | 24); // -> name level
    directory(&mut section, 1, SUBDIRECTORY | 48); // -> language level
    directory(&mut section, LANGUAGE, 72); // -> data entry
    let data_rva_field = section.len() as u32; // 72
    let data_offset = 88u32;
    // IMAGE_RESOURCE_DATA_ENTRY; OffsetToData is an RVA, fixed up by the linker
    // (ADDR32NB against the section symbol, addend = offset in this section).
    section.extend_from_slice(&data_offset.to_le_bytes());
    section.extend_from_slice(&(data.len() as u32).to_le_bytes());
    section.extend_from_slice(&[0; 8]); // code page, reserved
    assert_eq!(section.len() as u32, data_offset);
    section.extend_from_slice(data);
    while section.len() % 8 != 0 {
        section.push(0);
    }

    const HEADERS: u32 = 20 + 40;
    let relocations = HEADERS + section.len() as u32;
    let symbols = relocations + 10;
    let mut object = Vec::new();
    // IMAGE_FILE_HEADER
    object.extend_from_slice(&machine.to_le_bytes());
    object.extend_from_slice(&1u16.to_le_bytes()); // sections
    object.extend_from_slice(&0u32.to_le_bytes()); // timestamp (reproducible)
    object.extend_from_slice(&symbols.to_le_bytes());
    object.extend_from_slice(&1u32.to_le_bytes()); // symbols
    object.extend_from_slice(&0u16.to_le_bytes()); // optional header size
    object.extend_from_slice(&0u16.to_le_bytes()); // characteristics
                                                   // IMAGE_SECTION_HEADER
    object.extend_from_slice(b".rsrc\0\0\0");
    object.extend_from_slice(&0u32.to_le_bytes()); // virtual size
    object.extend_from_slice(&0u32.to_le_bytes()); // virtual address
    object.extend_from_slice(&(section.len() as u32).to_le_bytes());
    object.extend_from_slice(&HEADERS.to_le_bytes()); // raw data
    object.extend_from_slice(&relocations.to_le_bytes());
    object.extend_from_slice(&0u32.to_le_bytes()); // line numbers
    object.extend_from_slice(&1u16.to_le_bytes()); // relocation count
    object.extend_from_slice(&0u16.to_le_bytes()); // line number count
                                                   // CNT_INITIALIZED_DATA | ALIGN_4BYTES | MEM_READ
    object.extend_from_slice(&0x4030_0040u32.to_le_bytes());
    object.extend_from_slice(&section);
    // IMAGE_RELOCATION
    object.extend_from_slice(&data_rva_field.to_le_bytes());
    object.extend_from_slice(&0u32.to_le_bytes()); // symbol index
    object.extend_from_slice(&relocation.to_le_bytes());
    // IMAGE_SYMBOL: static section symbol for .rsrc
    object.extend_from_slice(b".rsrc\0\0\0");
    object.extend_from_slice(&0u32.to_le_bytes()); // value
    object.extend_from_slice(&1i16.to_le_bytes()); // section number
    object.extend_from_slice(&0u16.to_le_bytes()); // type
    object.push(3); // IMAGE_SYM_CLASS_STATIC
    object.push(0); // aux symbols
    object.extend_from_slice(&4u32.to_le_bytes()); // empty string table
    object
}
