//! Album metadata, independent of the service it comes from.

pub mod musicbrainz;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    /// Identifier at the source (MusicBrainz release MBID).
    pub id: String,
    pub title: String,
    pub artist: String,
    pub date: Option<String>,
    pub country: Option<String>,
    pub label: Option<String>,
    pub catalog_number: Option<String>,
    pub barcode: Option<String>,
    /// Position of this disc in the release, starting at 1.
    pub disc_number: u32,
    pub disc_count: u32,
    pub tracks: Vec<TrackInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackInfo {
    pub number: u32,
    pub title: String,
    pub artist: String,
    pub length_ms: Option<u64>,
}
