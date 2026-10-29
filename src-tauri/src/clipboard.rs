//! Copying a picture to the Windows clipboard. The webview's own clipboard
//! API can't write pictures in the app's window, so the engine does it: the
//! PNG itself (which chat apps and browsers paste with transparency) and a
//! plain bitmap (which every other program understands).

/// A bitmap for the clipboard (`CF_DIB`): a 40-byte header, then the rows
/// bottom-up in blue, green, red, alpha order.
pub fn png_to_dib(png_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "The image couldn't be read.".to_string();
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|_| bad())?;
    let size = reader.output_buffer_size().ok_or_else(bad)?;
    if size > 64 << 20 {
        return Err(bad());
    }
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(|_| bad())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return Err(bad()),
    };
    let line = info.line_size;
    if w == 0 || h == 0 || line < w * channels || buf.len() < line * h {
        return Err(bad());
    }

    let mut out = Vec::with_capacity(40 + w * h * 4);
    out.extend_from_slice(&40u32.to_le_bytes()); // header size
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes()); // positive: bottom-up
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..h).rev() {
        let row = &buf[y * line..y * line + w * channels];
        for px in row.chunks_exact(channels) {
            let (r, g, b, a) = match channels {
                4 => (px[0], px[1], px[2], px[3]),
                3 => (px[0], px[1], px[2], 255),
                2 => (px[0], px[0], px[0], px[1]),
                _ => (px[0], px[0], px[0], 255),
            };
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    Ok(out)
}

/// Puts a PNG on the clipboard, as itself and as a bitmap.
#[cfg(windows)]
pub fn copy_png(png_bytes: &[u8]) -> Result<(), String> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    const CF_DIB: u32 = 8;
    let failed = || "The banner couldn't be copied. Save it instead.".to_string();

    let dib = png_to_dib(png_bytes)?;

    /// Copies bytes into memory the clipboard can own.
    unsafe fn global(bytes: &[u8]) -> Option<*mut core::ffi::c_void> {
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if mem.is_null() {
            return None;
        }
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            GlobalFree(mem);
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        GlobalUnlock(mem);
        Some(mem)
    }

    // Another program may be holding the clipboard for a moment.
    let mut opened = false;
    for _ in 0..10 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    if !opened {
        return Err(failed());
    }
    let result = unsafe {
        let name: Vec<u16> = "PNG\0".encode_utf16().collect();
        let png_format = RegisterClipboardFormatW(name.as_ptr());
        let mut ok = EmptyClipboard() != 0;
        // The clipboard owns the memory once it's set; it's freed here only
        // when setting fails.
        for (format, bytes) in [(CF_DIB, dib.as_slice()), (png_format, png_bytes)] {
            if !ok || format == 0 {
                continue;
            }
            match global(bytes) {
                Some(mem) => {
                    if SetClipboardData(format, mem).is_null() {
                        GlobalFree(mem);
                        ok = false;
                    }
                }
                None => ok = false,
            }
        }
        ok
    };
    unsafe { CloseClipboard() };
    if result {
        Ok(())
    } else {
        Err(failed())
    }
}

#[cfg(not(windows))]
pub fn copy_png(_png_bytes: &[u8]) -> Result<(), String> {
    Err("Copying pictures only works on Windows.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(color);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(data).unwrap();
        }
        out
    }

    #[test]
    fn rgba_becomes_a_bottom_up_bitmap() {
        // Two pixels wide, two high: red, green on top; blue, white below.
        let px = [
            255, 0, 0, 255, 0, 255, 0, 128, //
            0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let dib = png_to_dib(&encode(2, 2, png::ColorType::Rgba, &px)).unwrap();
        assert_eq!(dib.len(), 40 + 16);
        assert_eq!(&dib[0..4], &40u32.to_le_bytes());
        assert_eq!(&dib[4..8], &2i32.to_le_bytes());
        assert_eq!(&dib[8..12], &2i32.to_le_bytes());
        assert_eq!(&dib[14..16], &32u16.to_le_bytes());
        // The bottom row comes first: blue, then white.
        assert_eq!(&dib[40..48], &[255, 0, 0, 255, 255, 255, 255, 255]);
        // Then the top row: red, then half-transparent green.
        assert_eq!(&dib[48..56], &[0, 0, 255, 255, 0, 255, 0, 128]);
    }

    #[test]
    fn rgb_is_opaque() {
        let dib = png_to_dib(&encode(1, 1, png::ColorType::Rgb, &[10, 20, 30])).unwrap();
        assert_eq!(&dib[40..44], &[30, 20, 10, 255]);
    }

    #[test]
    fn junk_is_refused() {
        assert!(png_to_dib(b"not a png").is_err());
        assert!(png_to_dib(&[]).is_err());
    }
}
