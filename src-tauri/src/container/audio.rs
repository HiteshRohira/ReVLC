//! Pure-Rust audio metadata, duration, and embedded cover art extraction via `lofty`.

use std::path::Path;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey};

use crate::engine::MediaMeta;

use super::{parse_number_tag, write_embedded_art};

/// Reads tags, duration, and embedded cover art from an audio file without
/// spawning libVLC or any decoder.
pub fn parse_audio(path: &Path, art_dir: &Path) -> Result<MediaMeta, String> {
    let tagged = Probe::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .guess_file_type()
        .map_err(|e| format!("{}: {e}", path.display()))?
        .read()
        .map_err(|e| format!("{}: {e}", path.display()))?;

    let duration = tagged.properties().duration().as_millis() as i64;
    let mut meta = MediaMeta { duration, ..Default::default() };

    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        let non_empty = |s: Option<std::borrow::Cow<'_, str>>| {
            s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
        };
        let item_str = |key: ItemKey| {
            tag.get_string(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
        };

        meta.title = non_empty(tag.title());
        meta.artist = non_empty(tag.artist());
        meta.album = non_empty(tag.album());
        meta.album_artist = item_str(ItemKey::AlbumArtist);
        meta.genre = non_empty(tag.genre());
        meta.track = tag.track().filter(|&n| n > 0).or_else(|| item_str(ItemKey::TrackNumber).as_deref().and_then(parse_number_tag));
        meta.disc = tag.disk().filter(|&n| n > 0).or_else(|| item_str(ItemKey::DiscNumber).as_deref().and_then(parse_number_tag));
        meta.date = item_str(ItemKey::RecordingDate)
            .or_else(|| item_str(ItemKey::OriginalReleaseDate))
            .or_else(|| item_str(ItemKey::ReleaseDate))
            .or_else(|| tag.date().map(|d| d.year.to_string()));

        // Prefer front cover, else fall back to the first embedded picture.
        let pics = tag.pictures();
        let pic = pics
            .iter()
            .find(|p| p.pic_type() == PictureType::CoverFront)
            .or_else(|| pics.first());
        if let Some(p) = pic {
            meta.artwork = write_embedded_art(art_dir, p.data());
        }
    }

    Ok(meta)
}
