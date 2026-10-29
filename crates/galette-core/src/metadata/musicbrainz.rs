//! Lookup of a disc on MusicBrainz (<https://musicbrainz.org/doc/MusicBrainz_API>).
//!
//! MusicBrainz asks clients for a meaningful User-Agent and at most one
//! request per second.

use serde::Deserialize;

use super::{Release, TrackInfo};
use crate::toc::{PREGAP, Toc};
use crate::{Error, Result, http, ids};

const API: &str = "https://musicbrainz.org/ws/2";

pub struct Client {
    agent: ureq::Agent,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        Self { agent: http::agent() }
    }

    /// Releases containing this disc. Matches the exact Disc ID first, then
    /// falls back to releases with a similar TOC (MusicBrainz's fuzzy search).
    pub fn lookup(&self, toc: &Toc) -> Result<Vec<Release>> {
        let disc_id = ids::musicbrainz(toc);
        let url = format!(
            "{API}/discid/{disc_id}?toc={}&cdstubs=no&inc=artist-credits+labels+recordings&fmt=json",
            toc_param(toc)
        );
        let Some(bytes) = http::get_bytes(&self.agent, &url)? else {
            return Ok(Vec::new());
        };
        let body: DiscResponse = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Network(format!("unexpected MusicBrainz response: {e}")))?;
        Ok(body.releases.into_iter().filter_map(|r| r.into_release(&disc_id, toc)).collect())
    }
}

/// TOC in the `toc` query parameter format: `first+last+lead-out+offsets...`.
fn toc_param(toc: &Toc) -> String {
    let audio: Vec<_> = toc.audio_tracks().collect();
    let mut parts = vec![
        audio[0].number as u32,
        audio.last().unwrap().number as u32,
        toc.audio_lead_out() + PREGAP,
    ];
    parts.extend(audio.iter().map(|t| t.start + PREGAP));
    parts.iter().map(u32::to_string).collect::<Vec<_>>().join("+")
}

#[derive(Deserialize)]
struct DiscResponse {
    #[serde(default)]
    releases: Vec<MbRelease>,
}

#[derive(Deserialize)]
struct MbRelease {
    id: String,
    title: String,
    date: Option<String>,
    country: Option<String>,
    barcode: Option<String>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<MbCredit>,
    #[serde(rename = "label-info", default)]
    label_info: Vec<MbLabelInfo>,
    #[serde(default)]
    media: Vec<MbMedium>,
}

#[derive(Deserialize)]
struct MbCredit {
    name: String,
    #[serde(default)]
    joinphrase: String,
}

#[derive(Deserialize)]
struct MbLabelInfo {
    #[serde(rename = "catalog-number")]
    catalog_number: Option<String>,
    label: Option<MbLabel>,
}

#[derive(Deserialize)]
struct MbLabel {
    name: String,
}

#[derive(Deserialize)]
struct MbMedium {
    position: u32,
    #[serde(default)]
    discs: Vec<MbDisc>,
    #[serde(default)]
    tracks: Vec<MbTrack>,
}

#[derive(Deserialize)]
struct MbDisc {
    id: String,
}

#[derive(Deserialize)]
struct MbTrack {
    position: u32,
    title: String,
    length: Option<u64>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<MbCredit>,
}

fn credit_name(credits: &[MbCredit]) -> String {
    credits.iter().map(|c| format!("{}{}", c.name, c.joinphrase)).collect()
}

impl MbRelease {
    /// Keeps the medium matching our disc: by Disc ID, else by track count
    /// (for fuzzy TOC matches).
    fn into_release(self, disc_id: &str, toc: &Toc) -> Option<Release> {
        let track_count = toc.audio_tracks().count();
        let disc_count = self.media.len() as u32;
        let medium = self
            .media
            .iter()
            .find(|m| m.discs.iter().any(|d| d.id == disc_id))
            .or_else(|| self.media.iter().find(|m| m.tracks.len() == track_count))?;

        let artist = credit_name(&self.artist_credit);
        let tracks = medium
            .tracks
            .iter()
            .map(|t| TrackInfo {
                number: t.position,
                title: t.title.clone(),
                artist: if t.artist_credit.is_empty() {
                    artist.clone()
                } else {
                    credit_name(&t.artist_credit)
                },
                length_ms: t.length,
            })
            .collect();
        let label = self.label_info.first();

        Some(Release {
            disc_number: medium.position,
            disc_count,
            tracks,
            id: self.id,
            title: self.title,
            artist,
            date: self.date.filter(|d| !d.is_empty()),
            country: self.country,
            label: label.and_then(|l| l.label.as_ref()).map(|l| l.name.clone()),
            catalog_number: label.and_then(|l| l.catalog_number.clone()),
            barcode: self.barcode.filter(|b| !b.is_empty()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_query_parameter() {
        let toc = Toc::parse_musicbrainz("1 2 30150 150 15150").unwrap();
        assert_eq!(toc_param(&toc), "1+2+30150+150+15150");
    }
}
