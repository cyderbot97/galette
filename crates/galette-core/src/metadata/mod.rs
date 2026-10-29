//! Album metadata, independent of the service it comes from.

pub mod coverart;
pub mod musicbrainz;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    /// Identifier at the source (MusicBrainz release MBID).
    pub id: String,
    pub release_group_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub artist_ids: Vec<String>,
    pub date: Option<String>,
    /// Date of the first release of this album, in any edition.
    pub original_date: Option<String>,
    pub country: Option<String>,
    pub label: Option<String>,
    pub catalog_number: Option<String>,
    pub barcode: Option<String>,
    /// Position of this disc in the release, starting at 1.
    pub disc_number: u32,
    pub disc_count: u32,
    pub tracks: Vec<TrackInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    /// Identifier of the track on this release.
    pub id: String,
    /// Identifier of the recording, shared by every release containing it.
    pub recording_id: Option<String>,
    pub number: u32,
    pub title: String,
    pub artist: String,
    pub artist_ids: Vec<String>,
    pub length_ms: Option<u64>,
}
