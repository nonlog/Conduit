//! Offline pairing QR rendered natively in Rune; never sent to a web service.
//! Payload matches Android PairingCode.fromQrPayload exactly.
use qrcodegen::{QrCode, QrCodeEcc};

pub fn pairing_url(code: &str) -> Result<String, String> {
    if code.len() != 6 || !code.bytes().all(|c| c.is_ascii_digit()) {
        return Err("Pairing code must contain six digits".into());
    }
    Ok(format!("conduit://pair?code={code}"))
}

pub fn png_for_code(code: &str) -> Result<Vec<u8>, String> {
    let url = pairing_url(code)?;
    let qr =
        QrCode::encode_text(&url, QrCodeEcc::Medium).map_err(|_| "QR payload cannot be encoded")?;
    let modules = qr.size() as usize;
    const QUIET: usize = 4;
    const SCALE: usize = 6;
    let width = (modules + QUIET * 2) * SCALE;
    let mut pixels = vec![255u8; width * width * 3];
    for y in 0..modules {
        for x in 0..modules {
            if !qr.get_module(x as i32, y as i32) {
                continue;
            }
            for py in 0..SCALE {
                for px in 0..SCALE {
                    let offset =
                        (((y + QUIET) * SCALE + py) * width + (x + QUIET) * SCALE + px) * 3;
                    pixels[offset..offset + 3].fill(0);
                }
            }
        }
    }
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width as u32, width as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&pixels)
            .map_err(|e| e.to_string())?;
    }
    Ok(png_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_encodes_the_exact_android_pairing_uri() {
        assert_eq!(pairing_url("000012").unwrap(), "conduit://pair?code=000012");
        assert!(pairing_url("12345").is_err());
        assert!(pairing_url("1234567").is_err());
        assert!(pairing_url("12345x").is_err());
    }

    #[test]
    fn png_is_offline_valid_and_contains_a_quiet_zone() {
        let bytes = png_for_code("123456").unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes.as_slice()));
        let mut reader = decoder.read_info().unwrap();
        let mut buffer = vec![0u8; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        assert_eq!(info.width, info.height);
        assert!(info.width >= 220);
        assert!(buffer[0..3].iter().all(|byte| *byte == 255));
        assert!(buffer[..info.buffer_size()].iter().any(|byte| *byte == 0));
    }
}
