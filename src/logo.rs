// SPDX-License-Identifier: AGPL-3.0-or-later
//! The app logo, drawn by KitKat, embedded as RGBA.

const BYTES: &[u8] = include_bytes!("../assets/logo.png");

pub struct Image {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn decode() -> Option<Image> {
    let decoder = png::Decoder::new(std::io::Cursor::new(BYTES));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    buffer.truncate(info.buffer_size());

    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        _ => return None,
    };

    Some(Image {
        rgba,
        width: info.width,
        height: info.height,
    })
}
