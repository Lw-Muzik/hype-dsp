//! Now-playing metadata extraction.
//!
//! Pulls title/artist/album tags and the embedded front-cover image out of a
//! probed container (ID3, Vorbis comments, MP4 atoms, …) into a [`TrackMeta`].
//! The same extractor serves local files and HTTP streams (cloud / phone), so
//! every source surfaces real metadata + cover art the same way.

use base64::Engine as _;
use hm_core::TrackMeta;
use symphonia::core::formats::FormatReader;
use symphonia::core::meta::{StandardTag, StandardVisualKey};

/// Text tags read from a container during a library scan (no cover art, so it
/// stays cheap to run over a whole folder).
#[derive(Debug, Default, Clone)]
pub struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
}

/// Read title/artist/album/genre tags (no artwork) from a probed container.
pub fn extract_tags(format: &mut dyn FormatReader) -> TrackTags {
    let mut tags = TrackTags::default();
    let mut album_artist: Option<String> = None;

    let binding = format.metadata();
    let Some(rev) = binding.current() else {
        return tags;
    };
    for tag in &rev.media.tags {
        let Some(std) = &tag.std else { continue };
        match std {
            StandardTag::TrackTitle(v) if tags.title.is_none() => tags.title = Some(v.to_string()),
            StandardTag::Artist(v) if tags.artist.is_none() => tags.artist = Some(v.to_string()),
            StandardTag::AlbumArtist(v) if album_artist.is_none() => {
                album_artist = Some(v.to_string())
            }
            StandardTag::Album(v) if tags.album.is_none() => tags.album = Some(v.to_string()),
            StandardTag::Genre(v) if tags.genre.is_none() => tags.genre = Some(v.to_string()),
            _ => {}
        }
    }
    tags.artist = tags.artist.or(album_artist);
    tags
}

/// Read tags + the front-cover art from an already-probed container.
pub fn extract_metadata(format: &mut dyn FormatReader) -> TrackMeta {
    let mut meta = TrackMeta::default();
    let mut album_artist: Option<String> = None;

    let binding = format.metadata();
    let Some(rev) = binding.current() else {
        return meta;
    };

    for tag in &rev.media.tags {
        let Some(std) = &tag.std else { continue };
        match std {
            StandardTag::TrackTitle(v) if meta.title.is_none() => meta.title = Some(v.to_string()),
            StandardTag::Artist(v) if meta.artist.is_none() => meta.artist = Some(v.to_string()),
            StandardTag::AlbumArtist(v) if album_artist.is_none() => {
                album_artist = Some(v.to_string())
            }
            StandardTag::Album(v) if meta.album.is_none() => meta.album = Some(v.to_string()),
            _ => {}
        }
    }
    // Fall back to the album artist only when there's no track artist.
    meta.artist = meta.artist.or(album_artist);

    // Prefer the explicit front cover; otherwise take the first attached image.
    let cover = rev
        .media
        .visuals
        .iter()
        .find(|v| matches!(v.usage, Some(StandardVisualKey::FrontCover)))
        .or_else(|| rev.media.visuals.first());
    if let Some(v) = cover {
        if !v.data.is_empty() {
            let mime = v.media_type.clone().unwrap_or_else(|| "image/jpeg".into());
            let b64 = base64::engine::general_purpose::STANDARD.encode(&v.data);
            meta.cover = Some(format!("data:{mime};base64,{b64}"));
        }
    }
    meta
}
