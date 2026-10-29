//! Front covers from the Cover Art Archive (<https://coverartarchive.org>),
//! which hosts the artwork of MusicBrainz releases.

use super::Release;
use crate::{Result, http};

const API: &str = "https://coverartarchive.org";
/// Thumbnail size in pixels: sharp on any screen, and only a few hundred kB.
const SIZE: u32 = 1200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverArt {
    pub data: Vec<u8>,
    pub mime_type: &'static str,
    pub width: u32,
    pub height: u32,
}

impl CoverArt {
    /// Recognizes JPEG and PNG images; `None` for anything else.
    pub fn from_bytes(data: Vec<u8>) -> Option<Self> {
        let (mime_type, (width, height)) = if data.starts_with(&[0xFF, 0xD8]) {
            ("image/jpeg", jpeg_size(&data)?)
        } else if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.len() >= 24 {
            let be = |i: usize| u32::from_be_bytes(data[i..i + 4].try_into().unwrap());
            ("image/png", (be(16), be(20)))
        } else {
            return None;
        };
        Some(Self { data, mime_type, width, height })
    }

    pub fn extension(&self) -> &'static str {
        if self.mime_type == "image/png" { "png" } else { "jpg" }
    }
}

/// The front cover of the release, or else of its release group (the
/// artwork is often uploaded for only one edition of an album).
pub fn fetch_front(release: &Release) -> Result<Option<CoverArt>> {
    if release.id.is_empty() {
        return Ok(None);
    }
    let agent = http::agent();
    let mut urls = vec![format!("{API}/release/{}/front-{SIZE}", release.id)];
    if let Some(group) = &release.release_group_id {
        urls.push(format!("{API}/release-group/{group}/front-{SIZE}"));
    }
    for url in urls {
        if let Some(data) = http::get_bytes(&agent, &url)? {
            return Ok(CoverArt::from_bytes(data));
        }
    }
    Ok(None)
}

/// Reads the dimensions in the first SOF (start of frame) segment.
fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    let mut pos = 2;
    while pos + 9 <= data.len() {
        if data[pos] != 0xFF {
            return None;
        }
        let marker = data[pos + 1];
        let len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        // SOF0 to SOF15, except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let height = u16::from_be_bytes([data[pos + 5], data[pos + 6]]);
            let width = u16::from_be_bytes([data[pos + 7], data[pos + 8]]);
            return Some((width as u32, height as u32));
        }
        pos += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_dimensions() {
        let mut data = vec![0xFF, 0xD8];
        data.extend([0xFF, 0xE0, 0x00, 0x04, 0, 0]); // APP0, skipped
        data.extend([0xFF, 0xC0, 0x00, 0x11, 8, 0x01, 0xF4, 0x02, 0x58]); // 600×500
        data.extend([0; 12]);
        let cover = CoverArt::from_bytes(data).unwrap();
        assert_eq!((cover.mime_type, cover.width, cover.height), ("image/jpeg", 600, 500));
    }

    #[test]
    fn rejects_unknown_formats() {
        assert!(CoverArt::from_bytes(b"GIF89a".to_vec()).is_none());
    }
}
