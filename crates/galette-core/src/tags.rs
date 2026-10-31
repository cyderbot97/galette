//! Tags of a ripped track, named as MusicBrainz Picard writes them in Vorbis
//! comments (<https://picard-docs.musicbrainz.org/en/appendices/tag_mapping.html>),
//! so that players and library managers recognize them. MP3 and AIFF files
//! get the same tags as ID3v2.4 frames and .m4a files as iTunes atoms, again
//! as Picard names them.

use id3::frame::{Content, ExtendedText, Picture, PictureType, UniqueFileIdentifier};
use id3::{Frame, TagLike};
use std::path::Path;

use crate::metadata::Release;
use crate::metadata::coverart::CoverArt;
use crate::{Error, Result};

/// ID3 text frames holding a Vorbis comment as is.
const ID3_TEXT: [(&str, &str); 9] = [
    ("TITLE", "TIT2"),
    ("ARTIST", "TPE1"),
    ("ALBUM", "TALB"),
    ("ALBUMARTIST", "TPE2"),
    ("DATE", "TDRC"),
    ("ORIGINALDATE", "TDOR"),
    ("LABEL", "TPUB"),
    ("MEDIA", "TMED"),
    ("ISRC", "TSRC"),
];

/// Vorbis comments that go in user-defined (TXXX) ID3 frames, with their description.
const ID3_USER: [(&str, &str); 9] = [
    ("CATALOGNUMBER", "CATALOGNUMBER"),
    ("BARCODE", "BARCODE"),
    ("RELEASECOUNTRY", "MusicBrainz Album Release Country"),
    ("MUSICBRAINZ_ALBUMID", "MusicBrainz Album Id"),
    ("MUSICBRAINZ_RELEASEGROUPID", "MusicBrainz Release Group Id"),
    ("MUSICBRAINZ_RELEASETRACKID", "MusicBrainz Release Track Id"),
    ("MUSICBRAINZ_ARTISTID", "MusicBrainz Artist Id"),
    ("MUSICBRAINZ_ALBUMARTISTID", "MusicBrainz Album Artist Id"),
    ("MUSICBRAINZ_DISCID", "MusicBrainz Disc Id"),
];

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

    // Metadata entered by hand has empty fields and identifiers.
    let mut add = |key, value: Option<&String>| {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
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
        add("MUSICBRAINZ_ALBUMARTISTID", Some(id));
    }
    tags
}

/// Vorbis comments stored in the iTunes atoms of the same meaning.
const MP4_STANDARD: [(&str, mp4ameta::Fourcc); 5] = [
    ("TITLE", mp4ameta::ident::TITLE),
    ("ARTIST", mp4ameta::ident::ARTIST),
    ("ALBUM", mp4ameta::ident::ALBUM),
    ("ALBUMARTIST", mp4ameta::ident::ALBUM_ARTIST),
    ("DATE", mp4ameta::ident::YEAR),
];

/// Vorbis comments stored in freeform (`----:com.apple.iTunes:…`) atoms, in
/// addition to those of [`ID3_USER`], which have the same names.
const MP4_FREEFORM: [(&str, &str); 5] = [
    ("ORIGINALDATE", "ORIGINALDATE"),
    ("LABEL", "LABEL"),
    ("MEDIA", "MEDIA"),
    ("ISRC", "ISRC"),
    ("MUSICBRAINZ_TRACKID", "MusicBrainz Track Id"),
];

/// The Vorbis comments `tags` as an ID3v2.4 tag, with the cover if given.
pub fn id3(tags: &[(&str, String)], cover: Option<&CoverArt>) -> id3::Tag {
    // ID3v2.4 separates the values of a frame with a null character.
    let joined = |key: &str| {
        let values: Vec<&str> = tags.iter().filter(|(k, _)| *k == key).map(|(_, v)| v.as_str()).collect();
        (!values.is_empty()).then(|| values.join("\0"))
    };
    let mut tag = id3::Tag::with_version(id3::Version::Id3v24);
    for (key, frame) in ID3_TEXT {
        if let Some(value) = joined(key) {
            tag.add_frame(Frame::text(frame, value));
        }
    }
    // "3/12": number and total in one frame.
    for (number, total, frame) in [("TRACKNUMBER", "TRACKTOTAL", "TRCK"), ("DISCNUMBER", "DISCTOTAL", "TPOS")] {
        if let Some(number) = joined(number) {
            let value = joined(total).map_or(number.clone(), |total| format!("{number}/{total}"));
            tag.add_frame(Frame::text(frame, value));
        }
    }
    for (key, description) in ID3_USER {
        if let Some(value) = joined(key) {
            tag.add_frame(ExtendedText { description: description.into(), value });
        }
    }
    if let Some(recording) = joined("MUSICBRAINZ_TRACKID") {
        let id = UniqueFileIdentifier { owner_identifier: "http://musicbrainz.org".into(), identifier: recording.into_bytes() };
        tag.add_frame(Frame::with_content("UFID", Content::UniqueFileIdentifier(id)));
    }
    if let Some(cover) = cover {
        tag.add_frame(Picture {
            mime_type: cover.mime_type.into(),
            picture_type: PictureType::CoverFront,
            description: String::new(),
            data: cover.data.clone(),
        });
    }
    tag
}

/// The Vorbis comments `tags` as iTunes atoms for .m4a files, with the cover if given.
pub fn mp4(tags: &[(&str, String)], cover: Option<&CoverArt>) -> mp4ameta::Tag {
    use mp4ameta::{Data, FreeformIdent, Img};
    let values = |key: &str| -> Vec<String> { tags.iter().filter(|(k, _)| *k == key).map(|(_, v)| v.clone()).collect() };
    let number = |key: &str| values(key).first().and_then(|v| v.parse::<u16>().ok());

    let mut tag = mp4ameta::Tag::default();
    for (key, ident) in MP4_STANDARD {
        for value in values(key) {
            tag.add_data(ident, Data::Utf8(value));
        }
    }
    for (key, name) in ID3_USER.into_iter().chain(MP4_FREEFORM) {
        for value in values(key) {
            tag.add_data(FreeformIdent::new_static("com.apple.iTunes", name), Data::Utf8(value));
        }
    }
    if let Some(track) = number("TRACKNUMBER") {
        tag.set_track(track, number("TRACKTOTAL").unwrap_or(0));
    }
    if let Some(disc) = number("DISCNUMBER") {
        tag.set_disc(disc, number("DISCTOTAL").unwrap_or(0));
    }
    if let Some(cover) = cover {
        let data = cover.data.clone();
        tag.add_artwork(if cover.mime_type == "image/png" { Img::png(data) } else { Img::jpeg(data) });
    }
    tag
}

/// Tags added to a file already written, such as ReplayGain's, which needs
/// the whole album: `tags` replace any of the same names. They are stored
/// as plain Vorbis comments, user-defined (TXXX) ID3 frames or freeform MP4
/// atoms named as the tags, as Picard does for ReplayGain.
pub fn update_flac(path: &Path, tags: &[(&str, String)]) -> Result<()> {
    let failed = |e: metaflac::Error| tag_error(path, e);
    let mut tag = metaflac::Tag::read_from_path(path).map_err(failed)?;
    for (key, value) in tags {
        tag.set_vorbis(*key, vec![value.clone()]);
    }
    tag.write_to_path(path).map_err(failed)
}

/// See [`update_flac`]; for MP3 and AIFF files.
pub fn update_id3(path: &Path, tags: &[(&str, String)]) -> Result<()> {
    let failed = |e: id3::Error| tag_error(path, e);
    let mut tag = id3::Tag::read_from_path(path).map_err(failed)?;
    for (key, value) in tags {
        tag.remove_extended_text(Some(key), None);
        tag.add_frame(ExtendedText { description: (*key).into(), value: value.clone() });
    }
    tag.write_to_path(path, id3::Version::Id3v24).map_err(failed)
}

/// See [`update_flac`]; for .m4a files.
pub fn update_mp4(path: &Path, tags: &[(&'static str, String)]) -> Result<()> {
    let failed = |e: mp4ameta::Error| tag_error(path, e);
    let mut tag = mp4ameta::Tag::read_from_path(path).map_err(failed)?;
    for (key, value) in tags {
        let ident = mp4ameta::FreeformIdent::new_static("com.apple.iTunes", key);
        tag.set_data(ident, mp4ameta::Data::Utf8(value.clone()));
    }
    tag.write_to_path(path).map_err(failed)
}

fn tag_error(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::Encode(format!("cannot add tags to {}: {e}", path.display()))
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

    #[test]
    fn id3_frames() {
        let tags = vec![
            ("TITLE", "Song".to_string()),
            ("TRACKNUMBER", "3".into()),
            ("TRACKTOTAL", "12".into()),
            ("MUSICBRAINZ_ARTISTID", "a".into()),
            ("MUSICBRAINZ_ARTISTID", "b".into()),
            ("MUSICBRAINZ_TRACKID", "rec".into()),
        ];
        let tag = id3(&tags, None);
        assert_eq!(tag.title(), Some("Song"));
        assert_eq!((tag.track(), tag.total_tracks()), (Some(3), Some(12)));
        let artist_ids = tag.extended_texts().find(|t| t.description == "MusicBrainz Artist Id").unwrap();
        assert_eq!(artist_ids.value, "a\0b");
        assert!(tag.unique_file_identifiers().any(|u| u.identifier == b"rec"));
    }

    /// Writes a short file of each kind, adds a tag twice, and reads it back.
    #[test]
    fn tags_added_afterwards() {
        let dir = std::env::temp_dir().join(format!("galette-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio: Vec<u8> = (0..44_100u32 * 4).map(|i| (i % 251) as u8).collect();
        let base = vec![("TITLE", "Song".to_string())];
        let gain = |v: &str| vec![("REPLAYGAIN_TRACK_GAIN", format!("{v} dB"))];

        let flac = dir.join("a.flac");
        std::fs::write(&flac, crate::flac::encode(&audio, &base, None).unwrap()).unwrap();
        update_flac(&flac, &gain("-1.00")).unwrap();
        update_flac(&flac, &gain("-2.00")).unwrap();
        let tag = metaflac::Tag::read_from_path(&flac).unwrap();
        let values: Vec<&str> = tag.get_vorbis("REPLAYGAIN_TRACK_GAIN").unwrap().collect();
        assert_eq!(values, ["-2.00 dB"]);
        assert!(tag.get_vorbis("TITLE").is_some());

        let mp3 = dir.join("a.mp3");
        std::fs::write(&mp3, crate::mp3::encode(&audio, crate::mp3::Quality::V2, &id3(&base, None)).unwrap()).unwrap();
        let aiff = dir.join("a.aiff");
        let mut file = Vec::new();
        let mut tag = Vec::new();
        id3(&base, None).write_to(&mut tag, id3::Version::Id3v24).unwrap();
        crate::aiff::write(&mut file, &audio, Some(&tag)).unwrap();
        std::fs::write(&aiff, file).unwrap();
        for path in [&mp3, &aiff] {
            update_id3(path, &gain("-1.00")).unwrap();
            update_id3(path, &gain("-2.00")).unwrap();
            let tag = id3::Tag::read_from_path(path).unwrap();
            let values: Vec<&str> =
                tag.extended_texts().filter(|t| t.description == "REPLAYGAIN_TRACK_GAIN").map(|t| t.value.as_str()).collect();
            assert_eq!(values, ["-2.00 dB"], "{}", path.display());
            assert_eq!(tag.title(), Some("Song"));
        }

        let m4a = dir.join("a.m4a");
        std::fs::write(&m4a, crate::alac::encode(&audio, &mp4(&base, None)).unwrap()).unwrap();
        let before = std::fs::metadata(&m4a).unwrap().len();
        update_mp4(&m4a, &gain("-1.00")).unwrap();
        update_mp4(&m4a, &gain("-2.00")).unwrap();
        let tag = mp4ameta::Tag::read_from_path(&m4a).unwrap();
        let ident = mp4ameta::FreeformIdent::new_static("com.apple.iTunes", "REPLAYGAIN_TRACK_GAIN");
        assert_eq!(tag.strings_of(&ident).collect::<Vec<_>>(), ["-2.00 dB"]);
        assert_eq!(tag.title(), Some("Song"));
        // The audio is still there.
        assert!(std::fs::metadata(&m4a).unwrap().len() > before);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mp4_atoms() {
        let tags = vec![
            ("TITLE", "Song".to_string()),
            ("TRACKNUMBER", "3".into()),
            ("TRACKTOTAL", "12".into()),
            ("MUSICBRAINZ_ARTISTID", "a".into()),
            ("MUSICBRAINZ_ARTISTID", "b".into()),
        ];
        let tag = mp4(&tags, None);
        assert_eq!(tag.title(), Some("Song"));
        assert_eq!(tag.track(), (Some(3), Some(12)));
        let ident = mp4ameta::FreeformIdent::new_static("com.apple.iTunes", "MusicBrainz Artist Id");
        assert_eq!(tag.strings_of(&ident).collect::<Vec<_>>(), ["a", "b"]);
    }
}
