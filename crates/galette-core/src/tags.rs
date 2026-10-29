//! Tags of a ripped track, named as MusicBrainz Picard writes them in Vorbis
//! comments (<https://picard-docs.musicbrainz.org/en/appendices/tag_mapping.html>),
//! so that players and library managers recognize them.

use crate::metadata::Release;

/// Tags for track `number` of a disc with `track_count` audio tracks.
/// Without a release, only the numbering and the Disc ID are known.
pub fn vorbis_comments(
    release: Option<&Release>,
    number: u32,
    track_count: u32,
    disc_id: &str,
) -> Vec<(&'static str, String)> {
    let mut tags = vec![
        ("TRACKNUMBER", number.to_string()),
        ("TRACKTOTAL", track_count.to_string()),
        ("MUSICBRAINZ_DISCID", disc_id.to_string()),
        ("MEDIA", "CD".to_string()),
    ];
    let Some(release) = release else {
        return tags;
    };

    let mut add = |key, value: Option<&String>| {
        if let Some(value) = value {
            tags.push((key, value.clone()));
        }
    };
    add("ALBUM", Some(&release.title));
    add("ALBUMARTIST", Some(&release.artist));
    add("DATE", release.date.as_ref());
    add("ORIGINALDATE", release.original_date.as_ref());
    add("LABEL", release.label.as_ref());
    add("CATALOGNUMBER", release.catalog_number.as_ref());
    add("BARCODE", release.barcode.as_ref());
    add("RELEASECOUNTRY", release.country.as_ref());
    add("DISCNUMBER", Some(&release.disc_number.to_string()));
    add("DISCTOTAL", Some(&release.disc_count.to_string()));
    add("MUSICBRAINZ_ALBUMID", Some(&release.id));
    add("MUSICBRAINZ_RELEASEGROUPID", release.release_group_id.as_ref());

    if let Some(track) = release.tracks.iter().find(|t| t.number == number) {
        add("TITLE", Some(&track.title));
        add("ARTIST", Some(&track.artist));
        add("MUSICBRAINZ_RELEASETRACKID", Some(&track.id));
        add("MUSICBRAINZ_TRACKID", track.recording_id.as_ref());
        for id in &track.artist_ids {
            add("MUSICBRAINZ_ARTISTID", Some(id));
        }
    }
    for id in &release.artist_ids {
        tags.push(("MUSICBRAINZ_ALBUMARTISTID", id.clone()));
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::TrackInfo;

    #[test]
    fn without_release() {
        let tags = vorbis_comments(None, 3, 10, "abc");
        assert!(tags.contains(&("TRACKNUMBER", "3".into())));
        assert!(!tags.iter().any(|(k, _)| *k == "TITLE"));
    }

    #[test]
    fn with_release() {
        let release = Release {
            id: "rel".into(),
            release_group_id: None,
            title: "Album".into(),
            artist: "A & B".into(),
            artist_ids: vec!["a".into(), "b".into()],
            date: Some("2013".into()),
            original_date: None,
            country: None,
            label: None,
            catalog_number: None,
            barcode: None,
            disc_number: 1,
            disc_count: 1,
            tracks: vec![TrackInfo {
                id: "trk".into(),
                recording_id: Some("rec".into()),
                number: 1,
                title: "Song".into(),
                artist: "A".into(),
                artist_ids: vec!["a".into()],
                length_ms: None,
            }],
        };
        let tags = vorbis_comments(Some(&release), 1, 1, "abc");
        for expected in [("TITLE", "Song"), ("ALBUMARTIST", "A & B"), ("MUSICBRAINZ_TRACKID", "rec")] {
            assert!(tags.contains(&(expected.0, expected.1.into())), "{expected:?}");
        }
        assert_eq!(tags.iter().filter(|(k, _)| *k == "MUSICBRAINZ_ALBUMARTISTID").count(), 2);
        assert!(!tags.iter().any(|(k, _)| *k == "LABEL"));
    }
}
