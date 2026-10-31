//! Names of the album folder and track files, from templates such as
//! `{albumartist} - {album}` and `{track} - {title}`.
//!
//! Album fields: `{albumartist}`, `{album}`, `{year}`, `{disc}`, `{disccount}`.
//! Track fields (file template only): `{track}` (two digits), `{title}`, `{artist}`.
//! The folder template may contain `/` to create subfolders.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ids;
use crate::metadata::Release;
use crate::toc::Toc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Naming {
    pub folder: String,
    pub file: String,
}

impl Default for Naming {
    fn default() -> Self {
        Self { folder: "{albumartist} - {album}".into(), file: "{track} - {title}".into() }
    }
}

impl Naming {
    /// Album folder, relative to the output folder.
    pub fn folder(&self, release: Option<&Release>, toc: &Toc) -> PathBuf {
        let fields = Fields { release, toc, track: None };
        let path: PathBuf = self
            .folder
            .split(['/', '\\'])
            .map(|part| render(part, &fields))
            .filter(|part| !part.is_empty())
            .collect();
        if path.as_os_str().is_empty() { PathBuf::from(fields.album()) } else { path }
    }

    /// File name of a track, without extension.
    pub fn file(&self, release: Option<&Release>, toc: &Toc, track: u8) -> String {
        let name = render(&self.file, &Fields { release, toc, track: Some(track) });
        if name.is_empty() { format!("{track:02}") } else { name }
    }
}

struct Fields<'a> {
    release: Option<&'a Release>,
    toc: &'a Toc,
    track: Option<u8>,
}

impl Fields<'_> {
    fn album(&self) -> String {
        match self.release {
            Some(r) if !r.title.is_empty() => r.title.clone(),
            _ => format!("Disc {}", ids::musicbrainz(self.toc)),
        }
    }

    fn get(&self, name: &str) -> Option<String> {
        let release = self.release;
        let info = self.track.and_then(|n| release?.tracks.iter().find(|t| t.number == n as u32));
        Some(match name {
            "albumartist" => release.map(|r| r.artist.clone()).filter(|a| !a.is_empty()).unwrap_or("Unknown Artist".into()),
            "album" => self.album(),
            "year" => release.and_then(|r| r.date.as_ref()).map(|d| d.chars().take(4).collect()).unwrap_or_default(),
            "disc" => release.map_or(1, |r| r.disc_number).to_string(),
            "disccount" => release.map_or(1, |r| r.disc_count).to_string(),
            "track" => format!("{:02}", self.track?),
            "title" => info.map(|t| t.title.clone()).unwrap_or_else(|| match self.track.unwrap_or(0) {
                0 => "Hidden track".to_string(),
                n => format!("Track {n}"),
            }),
            "artist" => info.map(|t| t.artist.clone()).or_else(|| self.get("albumartist"))?,
            _ => return None,
        })
    }
}

/// Replaces `{field}`s; unknown fields are kept as written.
fn render(template: &str, fields: &Fields) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match fields.get(&name.to_lowercase()) {
                    Some(value) => out.push_str(&crate::session::sanitize(&value)),
                    None => out.push_str(&rest[start..start + end + 2]),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    crate::session::sanitize(out.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::TrackInfo;

    fn release() -> Release {
        Release {
            id: "r".into(),
            release_group_id: None,
            title: "Hail to the King".into(),
            artist: "Avenged Sevenfold".into(),
            artist_ids: vec![],
            date: Some("2013-08-26".into()),
            original_date: None,
            country: None,
            label: None,
            catalog_number: None,
            barcode: None,
            disc_number: 1,
            disc_count: 2,
            tracks: vec![TrackInfo {
                id: "t".into(),
                recording_id: None,
                number: 3,
                title: "Doing Time / Live?".into(),
                artist: "A7X".into(),
                artist_ids: vec![],
                length_ms: None,
            }],
        }
    }

    fn toc() -> Toc {
        Toc::parse_musicbrainz("1 3 30150 150 10150 20150").unwrap()
    }

    #[test]
    fn default_names() {
        let naming = Naming::default();
        let r = release();
        assert_eq!(naming.folder(Some(&r), &toc()), PathBuf::from("Avenged Sevenfold - Hail to the King"));
        assert_eq!(naming.file(Some(&r), &toc(), 3), "03 - Doing Time _ Live_");
    }

    #[test]
    fn subfolders_and_fields() {
        let naming = Naming {
            folder: "{albumartist}/{year} - {album}/CD {disc}".into(),
            file: "{disc}-{track} {artist} - {title} {unknown}".into(),
        };
        let r = release();
        assert_eq!(
            naming.folder(Some(&r), &toc()),
            ["Avenged Sevenfold", "2013 - Hail to the King", "CD 1"].iter().collect::<PathBuf>()
        );
        assert_eq!(naming.file(Some(&r), &toc(), 3), "1-03 A7X - Doing Time _ Live_ {unknown}");
    }

    #[test]
    fn without_metadata() {
        let naming = Naming::default();
        let folder = naming.folder(None, &toc());
        assert_eq!(folder, PathBuf::from(format!("Unknown Artist - Disc {}", ids::musicbrainz(&toc()))));
        assert_eq!(naming.file(None, &toc(), 2), "02 - Track 2");
        let empty = Naming { folder: "/".into(), file: String::new() };
        assert!(!empty.folder(None, &toc()).as_os_str().is_empty());
        assert_eq!(empty.file(None, &toc(), 2), "02");
    }
}
