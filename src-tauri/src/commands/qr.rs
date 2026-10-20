use base64::Engine;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
static CAPTURING: AtomicBool = AtomicBool::new(false);

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

/// Local raster only; never interpret a URL or executable SVG.
#[tauri::command(async)]
pub fn read_qr_image(path: String) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|_| "无法读取所选图片")?;
    if !file.metadata().map_err(|_| "无法读取图片信息")?.is_file() {
        return Err("请选择本地图片文件".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "图片读取失败")?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err("图片不能超过 16 MB".into());
    }
    let mime = image_mime(&bytes).ok_or("请选择 PNG、JPEG、WebP、BMP 或 GIF 图片")?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

struct CaptureLease;
impl Drop for CaptureLease {
    fn drop(&mut self) {
        CAPTURING.store(false, Ordering::Release);
    }
}

#[cfg(windows)]
struct RestoreWindow(tauri::WebviewWindow);
#[cfg(windows)]
impl Drop for RestoreWindow {
    fn drop(&mut self) {
        let _ = self.0.show();
        let _ = self.0.set_focus();
    }
}

/// Explicit action only. Capture stays in memory, with no image/payload logs.
#[tauri::command]
pub async fn capture_qr_screen(window: tauri::WebviewWindow) -> Result<String, String> {
    CAPTURING
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .map_err(|_| "正在扫描屏幕，请稍候")?;
    let lease = CaptureLease;
    #[cfg(windows)]
    {
        let restore = RestoreWindow(window.clone());
        window.hide().map_err(|_| "无法暂时隐藏窗口以扫描屏幕")?;
        let captured = tauri::async_runtime::spawn_blocking(move || {
            let _lease = lease;
            std::thread::sleep(std::time::Duration::from_millis(200));
            capture_windows_screen()
        })
        .await
        .map_err(|_| "屏幕扫描任务异常")?;
        drop(restore);
        captured
    }
    #[cfg(not(windows))]
    {
        let _ = (window, lease);
        Err("当前平台请使用扫描图片中的二维码".into())
    }
}

#[cfg(windows)]
fn capture_windows_screen() -> Result<String, String> {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    struct CaptureDc {
        desktop: HDC,
        memory: HDC,
        bitmap: HBITMAP,
    }
    impl Drop for CaptureDc {
        fn drop(&mut self) {
            unsafe {
                if !self.memory.is_invalid() {
                    let _ = DeleteDC(self.memory);
                }
                if !self.bitmap.is_invalid() {
                    let _ = DeleteObject(self.bitmap.into());
                }
                if !self.desktop.is_invalid() {
                    let _ = ReleaseDC(None, self.desktop);
                }
            }
        }
    }
    unsafe {
        let (x, y) = (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        );
        let (width, height) = (
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        );
        if width <= 0 || height <= 0 || (width as u64) * (height as u64) > 32_000_000 {
            return Err("屏幕尺寸超出扫描限制，请改用二维码图片".into());
        }
        let mut dc = CaptureDc {
            desktop: GetDC(None),
            memory: HDC::default(),
            bitmap: HBITMAP::default(),
        };
        if dc.desktop.is_invalid() {
            return Err("无法读取桌面图像".into());
        }
        dc.memory = CreateCompatibleDC(Some(dc.desktop));
        if dc.memory.is_invalid() {
            return Err("无法创建屏幕扫描缓冲区".into());
        }
        dc.bitmap = CreateCompatibleBitmap(dc.desktop, width, height);
        if dc.bitmap.is_invalid() {
            return Err("无法分配屏幕扫描图像".into());
        }
        let previous = SelectObject(dc.memory, dc.bitmap.into());
        if previous.is_invalid() {
            return Err("无法准备屏幕扫描图像".into());
        }
        let copied = BitBlt(
            dc.memory,
            0,
            0,
            width,
            height,
            Some(dc.desktop),
            x,
            y,
            SRCCOPY | CAPTUREBLT,
        );
        let _ = SelectObject(dc.memory, previous);
        copied.map_err(|_| "屏幕截取失败，请检查屏幕捕获权限")?;
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        if GetDIBits(
            dc.desktop,
            dc.bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        ) != height
        {
            return Err("无法读取屏幕像素".into());
        }
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|_| "无法编码屏幕图像")?;
            writer
                .write_image_data(&pixels)
                .map_err(|_| "屏幕图像编码失败")?;
        }
        if png.len() as u64 > MAX_IMAGE_BYTES {
            return Err("屏幕图像过大，请改用二维码图片".into());
        }
        Ok(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_only_raster_signatures() {
        assert_eq!(image_mime(b"\x89PNG\r\n\x1a\n"), Some("image/png"));
        assert_eq!(image_mime(b"\xff\xd8\xff"), Some("image/jpeg"));
        assert_eq!(image_mime(b"RIFF0000WEBP"), Some("image/webp"));
        for bytes in [
            b"<svg/>".as_slice(),
            b"https://example.test/image.png",
            b"",
            b"RIFF0000WAVE",
        ] {
            assert!(image_mime(bytes).is_none());
        }
    }
    #[test]
    fn rejects_non_images_and_oversized_files() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "satelite-qr-{}",
            crate::config::generate_api_secret()
        ));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"<svg/>").unwrap();
        assert!(read_qr_image(path.to_string_lossy().into()).is_err());
        file.set_len(MAX_IMAGE_BYTES + 1).unwrap();
        assert!(read_qr_image(path.to_string_lossy().into())
            .unwrap_err()
            .contains("16 MB"));
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "CI only: captures the temporary runner desktop, never the user screen"]
    fn live_capture_returns_png_and_releases_gdi() {
        assert_eq!(std::env::var("CI").as_deref(), Ok("true"));
        for _ in 0..3 {
            let data = capture_windows_screen().expect("capture CI desktop");
            let encoded = data.strip_prefix("data:image/png;base64,").unwrap();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap();
            assert_eq!(image_mime(&bytes), Some("image/png"));
            let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            let reader = decoder.read_info().unwrap();
            assert!(reader.info().width > 0 && reader.info().height > 0);
            assert!(u64::from(reader.info().width) * u64::from(reader.info().height) <= 32_000_000);
        }
    }
}
