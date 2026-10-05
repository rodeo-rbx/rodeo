//! Studio's on-disk cookie store, where it keeps its sign-in tokens. On macOS
//! it is `~/Library/HTTPStorages/com.Roblox.RobloxStudio.binarycookies`, in
//! Apple's binarycookies format. Only record names and creation dates are
//! read, never values.
//!
//! Verified on Studio 0.741 (macOS): each CookieKeyValueStorage key is one
//! record on `www.roblox.com` named after the key's path (e.g.
//! `/RobloxStudioAuth/oauth2RefreshToken<user id>`), with its creation date in
//! whole seconds. Studio writes the store to disk lazily, seconds after it
//! saves a key.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Seconds from the Unix epoch to Apple's reference date, 2001-01-01T00:00:00Z.
const APPLE_EPOCH: f64 = 978_307_200.0;

/// Studio's cookie store, on platforms where it can be read (macOS).
pub fn store_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|h| h.join("Library/HTTPStorages/com.Roblox.RobloxStudio.binarycookies"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// When the newest record named `name` in a binarycookies file was created.
/// `None` if there is none or the file isn't binarycookies.
///
/// Layout: `cook`, a big-endian page count and page sizes, then pages. A page
/// is a header, a little-endian record count and record offsets. A record has
/// eight little-endian u32s (size, ?, flags, ?, then offsets of domain, name,
/// path and value from the record start), an 8-byte marker, then the expiry
/// and creation dates as little-endian f64 seconds since 2001-01-01.
pub fn created_at(file: &[u8], name: &str) -> Option<SystemTime> {
    let be = |at: usize| Some(u32::from_be_bytes(file.get(at..at + 4)?.try_into().ok()?) as usize);
    let le = |bytes: &[u8], at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize);
    if file.get(..4)? != b"cook" {
        return None;
    }
    let pages = be(4)?;
    let mut page_at = 8 + 4 * pages;
    let mut newest = None;
    for page_index in 0..pages {
        let size = be(8 + 4 * page_index)?;
        let page = file.get(page_at..page_at + size)?;
        page_at += size;
        for record_index in 0..le(page, 4)? {
            let record = page.get(le(page, 8 + 4 * record_index)?..)?;
            let record_name = record.get(le(record, 20)?..)?.split(|&b| b == 0).next()?;
            if record_name != name.as_bytes() {
                continue;
            }
            let created = f64::from_le_bytes(record.get(48..56)?.try_into().ok()?);
            let created = SystemTime::UNIX_EPOCH + Duration::try_from_secs_f64(APPLE_EPOCH + created).ok()?;
            newest = newest.max(Some(created));
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page binarycookies file holding `(name, created)` records, with
    /// Apple-epoch creation seconds and a placeholder value.
    fn store(records: &[(&str, f64)]) -> Vec<u8> {
        let bodies: Vec<Vec<u8>> = records
            .iter()
            .map(|(name, created)| {
                let name = format!("{name}\0");
                let strings = [b"www.roblox.com\0".as_slice(), name.as_bytes(), b"/RobloxStudioAuth\0", b"value\0"];
                let mut offsets = Vec::new();
                let mut at = 56;
                for s in &strings {
                    offsets.push(at as u32);
                    at += s.len();
                }
                let mut body = Vec::new();
                for word in [at as u32, 1, 0, 0, offsets[0], offsets[1], offsets[2], offsets[3]] {
                    body.extend_from_slice(&word.to_le_bytes());
                }
                body.extend_from_slice(&[0; 8]);
                body.extend_from_slice(&0f64.to_le_bytes());
                body.extend_from_slice(&created.to_le_bytes());
                for s in strings {
                    body.extend_from_slice(s);
                }
                body
            })
            .collect();
        let mut page = vec![0, 0, 1, 0];
        page.extend_from_slice(&(bodies.len() as u32).to_le_bytes());
        let mut at = 8 + 4 * bodies.len() + 4;
        for body in &bodies {
            page.extend_from_slice(&(at as u32).to_le_bytes());
            at += body.len();
        }
        page.extend_from_slice(&[0; 4]);
        for body in &bodies {
            page.extend_from_slice(body);
        }
        let mut file = b"cook".to_vec();
        file.extend_from_slice(&1u32.to_be_bytes());
        file.extend_from_slice(&(page.len() as u32).to_be_bytes());
        file.extend_from_slice(&page);
        file
    }

    #[test]
    fn reads_a_record_creation_date_by_name() {
        // 2026-10-05T21:32:34Z = Unix 1791235954 = Apple 812928754.
        let file = store(&[
            ("/RobloxStudioAuth/userid", 812_928_754.0),
            ("/RobloxStudioAuth/oauth2RefreshToken902015375", 812_928_754.0),
        ]);
        assert_eq!(
            created_at(&file, "/RobloxStudioAuth/oauth2RefreshToken902015375"),
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_791_235_954)),
        );
        assert_eq!(created_at(&file, "/RobloxStudioAuth/accessToken902015375"), None);
        assert_eq!(created_at(b"not a cookie store", "/RobloxStudioAuth/userid"), None);
    }
}
