use std::io::Cursor;

pub(crate) fn encode(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Selecione uma imagem de até 8 MiB.".into());
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Não foi possível identificar a imagem.")?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
    ) {
        return Err("Selecione uma imagem PNG, JPEG ou WebP.".into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(80 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "A imagem está ilegível ou excede 4096 pixels por lado.")?;
    let mut output = Cursor::new(Vec::new());
    image
        .thumbnail(256, 256)
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|_| "Não foi possível preparar a capa.")?;
    Ok(output.into_inner())
}
