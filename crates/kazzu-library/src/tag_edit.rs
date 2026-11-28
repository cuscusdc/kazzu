//! Edits a local file's primary tag while keeping the original on write failure.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, Tag, TagExt};

pub fn update_tags(
    path: &Path,
    title: &str,
    artist: &str,
    album: &str,
    genre: &str,
) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| io::Error::other("Arquivo sem extensão reconhecível"))?;
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let temporary =
        path.with_file_name(format!(".kazzu-tag-{}-{sequence}.part", std::process::id()));
    let mut source = fs::File::open(path)?;
    let mut target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        io::copy(&mut source, &mut target)?;
        target.set_permissions(source.metadata()?.permissions())?;
        drop(target);

        let kind = lofty::file::FileType::from_ext(extension)
            .ok_or_else(|| io::Error::other("Formato sem suporte para edição de tags"))?;
        let tagged = lofty::probe::Probe::open(&temporary)
            .map_err(io::Error::other)?
            .set_file_type(kind)
            .read()
            .map_err(io::Error::other)?;
        let mut tag = tagged
            .primary_tag()
            .cloned()
            .unwrap_or_else(|| Tag::new(kind.primary_tag_type()));
        tag.set_title(title.to_owned());
        if !artist.is_empty() {
            tag.set_artist(artist.to_owned());
        }
        if !album.is_empty() {
            tag.set_album(album.to_owned());
        }
        if !genre.is_empty() {
            tag.set_genre(genre.to_owned());
        }
        let mut file = OpenOptions::new().read(true).write(true).open(&temporary)?;
        tag.save_to(&mut file, WriteOptions::default())
            .map_err(io::Error::other)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
