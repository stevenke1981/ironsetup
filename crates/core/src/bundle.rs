//! Versioned package and single-executable trailer. Hashes detect corruption;
//! they are not a publisher signature.
use crate::{Manifest, Package, FORMAT_VERSION, MAX_FILES, MAX_PACKAGE_BYTES};
use anyhow::{bail, ensure, Context, Result};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom, Write},
    path::Path,
};
const MAGIC: &[u8; 8] = b"IRNPKG01";
const SFX_MAGIC: &[u8; 16] = b"IRONSETUP-SFX-01";
const FOOTER_LEN: u64 = 56;
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;
const MAX_ENCODED: u64 = MAX_PACKAGE_BYTES + 32 * 1024 * 1024;
const MAX_STUB: u64 = 64 * 1024 * 1024;

pub fn validate_package(package: &Package) -> Result<()> {
    let m = &package.manifest;
    ensure!(
        m.format_version == FORMAT_VERSION,
        "Unsupported package format {}",
        m.format_version
    );
    ensure!(
        !m.app.id.trim().is_empty()
            && !m.app.name.trim().is_empty()
            && !m.app.version.trim().is_empty(),
        "AppId, AppName and AppVersion must not be empty"
    );
    ensure!(
        matches!(m.app.privileges.as_str(), "lowest" | "admin"),
        "Unsupported privilege mode"
    );
    ensure!(
        matches!(m.app.compression.as_str(), "zip" | "none"),
        "Unsupported compression: {}",
        m.app.compression
    );
    ensure!(m.files.len() <= MAX_FILES, "Package has too many files");
    ensure!(
        m.files.len() == package.payloads.len(),
        "Manifest/payload count mismatch"
    );
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for (entry, data) in m.files.iter().zip(&package.payloads) {
        crate::validate_relative_path(&entry.path)?;
        let key = entry.path.replace('\\', "/").to_lowercase();
        ensure!(
            names.insert(key),
            "Duplicate Windows destination: {}",
            entry.path
        );
        ensure!(
            entry.size == data.len() as u64,
            "Size mismatch: {}",
            entry.path
        );
        total = total
            .checked_add(entry.size)
            .context("Package size overflow")?;
        ensure!(
            total <= MAX_PACKAGE_BYTES,
            "Uncompressed package exceeds the 512 MiB v0.1 limit"
        );
        ensure!(
            entry.sha256 == crate::sha256(data),
            "SHA-256 mismatch: {}",
            entry.path
        );
        for f in &entry.flags {
            ensure!(
                matches!(
                    f.as_str(),
                    "ignoreversion"
                        | "onlyifdoesntexist"
                        | "uninsneveruninstall"
                        | "recursesubdirs"
                        | "createallsubdirs"
                        | "skipifsourcedoesntexist"
                ),
                "Unsupported file flag in package: {f}"
            );
        }
        validate_conditions(&entry.conditions)?;
    }
    for path in &names {
        let mut parent = path.as_str();
        while let Some((head, _)) = parent.rsplit_once('/') {
            ensure!(
                !names.contains(head),
                "File is also a parent directory: {head}"
            );
            parent = head;
        }
    }
    for d in &m.dirs {
        crate::validate_relative_path(&d.path)?;
        let directory_key = d.path.replace('\\', "/").to_lowercase();
        let mut parent = directory_key.as_str();
        while let Some((head, _)) = parent.rsplit_once('/') {
            ensure!(
                !names.contains(head),
                "File is also a parent directory: {head}"
            );
            parent = head;
        }
        ensure!(
            !names.contains(&d.path.replace('\\', "/").to_lowercase()),
            "Directory conflicts with a file: {}",
            d.path
        );
        validate_conditions(&d.conditions)?;
    }
    for choices in [&m.components, &m.tasks] {
        let mut ids = BTreeSet::new();
        for c in choices {
            ensure!(
                !c.name.is_empty() && ids.insert(c.name.to_lowercase()),
                "Duplicate or empty choice name: {}",
                c.name
            );
            validate_conditions(&c.conditions)?;
        }
    }
    for x in &m.icons {
        validate_conditions(&x.conditions)?;
    }
    for x in &m.registry {
        validate_conditions(&x.conditions)?;
    }
    for x in &m.run {
        validate_conditions(&x.conditions)?;
    }
    Ok(())
}
fn validate_conditions(c: &crate::Conditions) -> Result<()> {
    crate::evaluate_condition(&c.components, &BTreeSet::new())?;
    crate::evaluate_condition(&c.tasks, &BTreeSet::new())?;
    Ok(())
}
pub fn encode_package(package: &Package) -> Result<Vec<u8>> {
    validate_package(package)?;
    let manifest = serde_json::to_vec(&package.manifest)?;
    ensure!(
        manifest.len() as u64 <= MAX_MANIFEST,
        "Manifest is too large"
    );
    let mut output = Vec::new();
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
    output.extend_from_slice(&manifest);
    for bytes in &package.payloads {
        let (method, data) = if package.manifest.app.compression == "zip" {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
            encoder.write_all(bytes)?;
            (1u8, encoder.finish()?)
        } else {
            (0u8, bytes.clone())
        };
        output.push(method);
        output.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        output.extend_from_slice(&(data.len() as u64).to_le_bytes());
        output.extend_from_slice(&data);
        ensure!(
            output.len() as u64 <= MAX_ENCODED - 32,
            "Encoded package is too large"
        );
    }
    let digest = Sha256::digest(&output);
    output.extend_from_slice(&digest);
    Ok(output)
}
fn read_u64(cursor: &mut Cursor<&[u8]>) -> Result<u64> {
    let mut b = [0u8; 8];
    cursor
        .read_exact(&mut b)
        .context("Truncated package header")?;
    Ok(u64::from_le_bytes(b))
}
pub fn decode_package(bytes: &[u8]) -> Result<Package> {
    ensure!(
        bytes.len() >= 48 && bytes.len() as u64 <= MAX_ENCODED,
        "Invalid package size"
    );
    let split = bytes.len() - 32;
    ensure!(
        Sha256::digest(&bytes[..split]).as_slice() == &bytes[split..],
        "Package checksum mismatch"
    );
    let mut cursor = Cursor::new(&bytes[..split]);
    let mut magic = [0u8; 8];
    cursor.read_exact(&mut magic)?;
    ensure!(&magic == MAGIC, "Not an IronSetup package");
    let manifest_len = read_u64(&mut cursor)?;
    ensure!(
        manifest_len <= MAX_MANIFEST && manifest_len <= (split - 16) as u64,
        "Invalid manifest length"
    );
    let start = cursor.position() as usize;
    let end = start + manifest_len as usize;
    let manifest: Manifest =
        serde_json::from_slice(&bytes[start..end]).context("Invalid package manifest")?;
    ensure!(
        manifest.format_version == FORMAT_VERSION,
        "Unsupported package format"
    );
    ensure!(manifest.files.len() <= MAX_FILES, "Too many package files");
    cursor.set_position(end as u64);
    let mut payloads = Vec::with_capacity(manifest.files.len());
    let mut total = 0u64;
    for entry in &manifest.files {
        let mut method = [0u8];
        cursor.read_exact(&mut method).context("Missing payload")?;
        let raw_size = read_u64(&mut cursor)?;
        let stored_size = read_u64(&mut cursor)?;
        total = total
            .checked_add(raw_size)
            .context("Package size overflow")?;
        ensure!(
            total <= MAX_PACKAGE_BYTES && raw_size == entry.size,
            "Invalid/unbounded payload size: {}",
            entry.path
        );
        ensure!(
            stored_size <= (split as u64).saturating_sub(cursor.position()),
            "Truncated payload: {}",
            entry.path
        );
        let begin = cursor.position() as usize;
        let finish = begin + stored_size as usize;
        let data = &bytes[begin..finish];
        let raw = match method[0] {
            0 => {
                ensure!(stored_size == raw_size, "Stored payload size mismatch");
                data.to_vec()
            }
            1 => {
                let mut decoder = ZlibDecoder::new(data);
                let mut result = Vec::with_capacity((raw_size as usize).min(8 * 1024 * 1024));
                decoder
                    .by_ref()
                    .take(raw_size + 1)
                    .read_to_end(&mut result)
                    .context("Invalid compressed payload")?;
                ensure!(
                    result.len() as u64 == raw_size,
                    "Decompressed size mismatch: {}",
                    entry.path
                );
                ensure!(
                    decoder.total_in() == stored_size,
                    "Trailing compressed payload data"
                );
                result
            }
            _ => bail!("Unsupported payload compression method"),
        };
        payloads.push(raw);
        cursor.set_position(finish as u64);
    }
    ensure!(
        cursor.position() == split as u64,
        "Unexpected trailing package data"
    );
    let package = Package { manifest, payloads };
    validate_package(&package)?;
    Ok(package)
}
fn atomic_write(path: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    crate::reject_symlink_ancestors(path)?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Cannot create {}", parent.display()))?;
    crate::reject_symlink_ancestors(path)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(if executable {
                0o755
            } else {
                0o644
            }))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    temp.persist(path)
        .with_context(|| format!("Cannot replace output {}", path.display()))?;
    Ok(())
}
pub fn read_package_file(path: &Path) -> Result<Package> {
    ensure!(
        std::fs::metadata(path)?.len() <= MAX_ENCODED,
        "Package is too large"
    );
    decode_package(&std::fs::read(path)?)
}
pub fn write_package_file(path: &Path, package: &Package) -> Result<()> {
    atomic_write(path, &encode_package(package)?, false)
}
fn embedded_layout(file: &mut File) -> Result<Option<(u64, u64, [u8; 32])>> {
    let length = file.metadata()?.len();
    if length < FOOTER_LEN {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-(FOOTER_LEN as i64)))?;
    let mut footer = [0u8; FOOTER_LEN as usize];
    file.read_exact(&mut footer)?;
    if &footer[40..] != SFX_MAGIC {
        return Ok(None);
    }
    let size = u64::from_le_bytes(footer[..8].try_into()?);
    ensure!(
        size <= MAX_ENCODED && size <= length - FOOTER_LEN,
        "Invalid embedded package length"
    );
    let offset = length - FOOTER_LEN - size;
    ensure!(
        offset > 0 && offset <= MAX_STUB,
        "Invalid installer stub length"
    );
    Ok(Some((offset, size, footer[8..40].try_into()?)))
}
pub fn read_embedded_package(path: &Path) -> Result<Option<Package>> {
    let mut file = File::open(path)?;
    let Some((offset, size, digest)) = embedded_layout(&mut file)? else {
        return Ok(None);
    };
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0u8; size as usize];
    file.read_exact(&mut bytes)?;
    ensure!(
        Sha256::digest(&bytes).as_slice() == digest,
        "Embedded package checksum mismatch"
    );
    Ok(Some(decode_package(&bytes)?))
}
pub fn stub_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let size = match embedded_layout(&mut file)? {
        Some((offset, _, _)) => offset,
        None => file.metadata()?.len(),
    };
    ensure!(size > 0 && size <= MAX_STUB, "Stub must be 1 byte–64 MiB");
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = vec![0u8; size as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
pub fn append_package(stub: &Path, package: &Package, output: &Path) -> Result<()> {
    let mut bytes = stub_bytes(stub)?;
    let archive = encode_package(package)?;
    let digest = Sha256::digest(&archive);
    bytes.extend_from_slice(&archive);
    bytes.extend_from_slice(&(archive.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&digest);
    bytes.extend_from_slice(SFX_MAGIC);
    atomic_write(output, &bytes, true)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppInfo, Conditions, ManifestFile};
    fn sample() -> Package {
        let data = b"hello world, hello world, hello world".to_vec();
        Package {
            manifest: Manifest {
                format_version: 1,
                app: AppInfo {
                    id: "demo".into(),
                    name: "Demo".into(),
                    version: "1.0".into(),
                    publisher: "".into(),
                    default_dir: "{userpf}\\Demo".into(),
                    default_group: "Demo".into(),
                    privileges: "lowest".into(),
                    uninstallable: true,
                    compression: "zip".into(),
                    architectures_allowed: String::new(),
                },
                files: vec![ManifestFile {
                    path: "bin/demo.txt".into(),
                    size: data.len() as u64,
                    sha256: crate::sha256(&data),
                    executable: false,
                    flags: vec!["ignoreversion".into()],
                    conditions: Conditions::default(),
                }],
                dirs: vec![],
                components: vec![],
                tasks: vec![],
                types: vec![],
                icons: vec![],
                registry: vec![],
                run: vec![],
                license_text: None,
                info_before: None,
                info_after: None,
            },
            payloads: vec![data],
        }
    }
    #[test]
    fn deterministic_compressed_roundtrip() {
        let p = sample();
        let b = encode_package(&p).unwrap();
        assert_eq!(b, encode_package(&p).unwrap());
        let q = decode_package(&b).unwrap();
        assert_eq!(q.payloads, p.payloads);
        assert_eq!(q.manifest.app.id, "demo");
    }
    #[test]
    fn arbitrary_mutations_never_panic() {
        // Re-sign every mutated buffer so corruption reaches the parser rather
        // than stopping at the outer checksum.
        let base = encode_package(&sample()).unwrap();
        let body = base.len() - 32;
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for i in 0..4000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let mut b = base[..body].to_vec();
            match i % 3 {
                0 => b[(seed as usize) % body] ^= (seed >> 32) as u8 | 1,
                1 => b.truncate((seed as usize) % body),
                _ => {
                    let at = 8 + (seed as usize) % 8;
                    b[at] = (seed >> 24) as u8;
                }
            }
            let digest = Sha256::digest(&b);
            b.extend_from_slice(&digest);
            let r = std::panic::catch_unwind(|| decode_package(&b));
            assert!(r.is_ok(), "decoder panicked on mutation {i}");
        }
    }
    #[test]
    fn rejects_corruption_and_truncation() {
        let mut b = encode_package(&sample()).unwrap();
        b[25] ^= 1;
        assert!(decode_package(&b).is_err());
        for n in [0, 8, 16, 31] {
            assert!(decode_package(&b[..n]).is_err());
        }
    }
    #[test]
    fn rejects_case_insensitive_collision() {
        let mut p = sample();
        let mut f = p.manifest.files[0].clone();
        f.path = "BIN/DEMO.TXT".into();
        p.manifest.files.push(f);
        p.payloads.push(p.payloads[0].clone());
        assert!(encode_package(&p)
            .unwrap_err()
            .to_string()
            .contains("Duplicate"));
    }
    #[test]
    fn rejects_bad_payload_and_traversal() {
        let mut p = sample();
        p.payloads[0].push(0);
        assert!(encode_package(&p).is_err());
        let mut p = sample();
        p.manifest.files[0].path = "../escape".into();
        assert!(encode_package(&p).is_err());
    }
    #[test]
    fn embedded_roundtrip_and_stub_strip() {
        let d = tempfile::tempdir().unwrap();
        let s = d.path().join("stub");
        let e = d.path().join("setup");
        std::fs::write(&s, b"MZ-test-stub").unwrap();
        append_package(&s, &sample(), &e).unwrap();
        assert_eq!(
            read_embedded_package(&e).unwrap().unwrap().payloads,
            sample().payloads
        );
        assert_eq!(stub_bytes(&e).unwrap(), b"MZ-test-stub");
        assert!(read_embedded_package(&s).unwrap().is_none());
        let mut b = std::fs::read(&e).unwrap();
        let ix = b.len() - 40;
        b[ix] ^= 1;
        std::fs::write(&e, b).unwrap();
        assert!(read_embedded_package(&e).is_err());
    }
    #[test]
    fn stored_roundtrip() {
        let mut p = sample();
        p.manifest.app.compression = "none".into();
        assert_eq!(
            decode_package(&encode_package(&p).unwrap())
                .unwrap()
                .payloads,
            p.payloads
        );
    }
}
