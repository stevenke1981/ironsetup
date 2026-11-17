//! Strict .ico parsing shared by the compiler (validation) and the CLI
//! (resource embedding). Malformed icons are errors, never silently ignored.
use anyhow::{bail, ensure, Result};

pub const MAX_ICO_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMAGES: usize = 256;

/// One image of an icon file, in the form needed for RT_ICON / RT_GROUP_ICON.
#[derive(Clone, Debug)]
pub struct IcoImage {
    pub width: u8,
    pub height: u8,
    pub color_count: u8,
    pub planes: u16,
    pub bit_count: u16,
    pub data: Vec<u8>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub fn parse_ico(bytes: &[u8]) -> Result<Vec<IcoImage>> {
    ensure!(
        bytes.len() <= MAX_ICO_BYTES,
        "Icon file exceeds {} MiB",
        MAX_ICO_BYTES / 1024 / 1024
    );
    ensure!(bytes.len() >= 6, "Not an .ico file: header is truncated");
    ensure!(
        u16_at(bytes, 0) == 0 && u16_at(bytes, 2) == 1,
        "Not an .ico file: bad ICONDIR header (cursors and other formats are rejected)"
    );
    let count = u16_at(bytes, 4) as usize;
    ensure!(count > 0, "Icon file contains no images");
    ensure!(count <= MAX_IMAGES, "Icon file has too many images");
    ensure!(bytes.len() >= 6 + count * 16, "Icon directory is truncated");
    let mut images = Vec::with_capacity(count);
    for i in 0..count {
        let e = 6 + i * 16;
        let size = u32_at(bytes, e + 8) as usize;
        let offset = u32_at(bytes, e + 12) as usize;
        ensure!(size > 0, "Icon image {i} is empty");
        let end = offset.checked_add(size);
        ensure!(
            offset >= 6 + count * 16 && end.is_some_and(|end| end <= bytes.len()),
            "Icon image {i} lies outside the file"
        );
        let data = bytes[offset..offset + size].to_vec();
        let png = data.starts_with(b"\x89PNG\r\n\x1a\n");
        let dib = data.len() >= 40 && u32_at(&data, 0) == 40;
        if !png && !dib {
            bail!("Icon image {i} is neither a PNG nor a BMP (DIB) image");
        }
        images.push(IcoImage {
            width: bytes[e],
            height: bytes[e + 1],
            color_count: bytes[e + 2],
            planes: u16_at(bytes, e + 4),
            bit_count: u16_at(bytes, e + 6),
            data,
        });
    }
    Ok(images)
}

/// Builds the RT_GROUP_ICON (GRPICONDIR) resource; RT_ICON ids are 1..=n.
pub fn group_icon_resource(images: &[IcoImage]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + images.len() * 14);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (i, im) in images.iter().enumerate() {
        out.extend_from_slice(&[im.width, im.height, im.color_count, 0]);
        out.extend_from_slice(&im.planes.to_le_bytes());
        out.extend_from_slice(&im.bit_count.to_le_bytes());
        out.extend_from_slice(&(im.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&((i + 1) as u16).to_le_bytes());
    }
    out
}

/// Parses an UninstallDisplayIcon value: {app}\relative\path.ext with an
/// optional ,index suffix. Returns the normalized relative path (with '/')
/// and the icon index.
pub fn parse_uninstall_icon(value: &str) -> Result<(String, Option<i32>)> {
    let (path, index) = match value.rsplit_once(',') {
        Some((p, i)) if i.trim().parse::<i32>().is_ok() => (p, Some(i.trim().parse::<i32>()?)),
        _ => (value, None),
    };
    let tail = path
        .get(..5)
        .filter(|p| p.eq_ignore_ascii_case("{app}"))
        .and_then(|_| path[5..].strip_prefix(['\\', '/']));
    let Some(tail) = tail else {
        bail!("UninstallDisplayIcon must be a path under {{app}}, such as {{app}}\\app.exe");
    };
    crate::validate_relative_path(tail)?;
    Ok((tail.replace('\\', "/"), index))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Vec<u8> {
        let mut dib = vec![0u8; 48];
        dib[0] = 40;
        let mut b = vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
        b.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        b.extend_from_slice(&22u32.to_le_bytes());
        b.extend_from_slice(&dib);
        b
    }
    #[test]
    fn accepts_valid_and_rejects_malformed() {
        let ok = sample();
        let images = parse_ico(&ok).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(group_icon_resource(&images).len(), 20);
        assert!(parse_ico(&ok[..20]).is_err());
        assert!(parse_ico(b"MZ").is_err());
        let mut cursor = ok.clone();
        cursor[2] = 2;
        assert!(parse_ico(&cursor).is_err());
        let mut junk = ok.clone();
        junk[22] = 1;
        assert!(parse_ico(&junk).is_err());
    }
    #[test]
    fn uninstall_icon_is_app_relative() {
        assert_eq!(
            parse_uninstall_icon("{app}\\notedit.exe").unwrap(),
            ("notedit.exe".into(), None)
        );
        assert_eq!(
            parse_uninstall_icon("{APP}/bin/a.exe,2").unwrap(),
            ("bin/a.exe".into(), Some(2))
        );
        for bad in [
            "C:\\x.exe",
            "{app}",
            "{app}\\..\\x.exe",
            "{pf}\\x.exe",
            "{app}x.exe",
            "",
        ] {
            assert!(parse_uninstall_icon(bad).is_err(), "{bad}");
        }
    }
}
