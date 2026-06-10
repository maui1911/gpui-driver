//! Best-effort window capture via Win32 `PrintWindow`.
//!
//! Used when the gpui platform backend lacks `render_to_image` — i.e. the app was
//! built against stock `gpui_windows` without the vendored patch. Unlike the renderer
//! readback this asks the window to paint itself into a memory DC; it needs no gpui
//! modifications, but fidelity while occluded, minimized or session-locked is not
//! guaranteed (the image may be stale or black). Callers surface this distinction via
//! the `method` field of the screenshot result.

use anyhow::{Context as _, Result, anyhow, bail};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, HDC, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PW_CLIENTONLY, PrintWindow};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

/// Undocumented but long-stable (Win 8.1+) flag: include DWM-composed content such as
/// DirectComposition surfaces — required for GPU-rendered windows like GPUI's.
const PW_RENDERFULLCONTENT: u32 = 0x0000_0002;

/// Captures the client area of `hwnd` by sending it a print request.
///
/// Must run on the window's own thread (PrintWindow dispatches `WM_PRINTCLIENT`
/// synchronously); the driver's handler loop runs on the GPUI main thread, which is
/// exactly that.
pub(crate) fn capture_hwnd(hwnd: isize) -> Result<image::RgbaImage> {
    unsafe {
        let hwnd = HWND(hwnd as *mut core::ffi::c_void);
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect).context("GetClientRect failed")?;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            bail!("window has an empty client area");
        }

        let hdc = CreateCompatibleDC(None);
        if hdc.is_invalid() {
            bail!("CreateCompatibleDC failed");
        }
        let result = capture_with_dc(hdc, hwnd, width, height);
        let _ = DeleteDC(hdc);
        result
    }
}

unsafe fn capture_with_dc(
    hdc: HDC,
    hwnd: HWND,
    width: i32,
    height: i32,
) -> Result<image::RgbaImage> {
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // negative = top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let hbitmap = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
            .context("CreateDIBSection failed")?;

        let old = SelectObject(hdc, hbitmap.into());
        let ok = PrintWindow(
            hwnd,
            hdc,
            PRINT_WINDOW_FLAGS(PW_CLIENTONLY.0 | PW_RENDERFULLCONTENT),
        );
        let _ = GdiFlush();
        // 32bpp DIB rows have no padding: stride is exactly width * 4.
        let pixels = ok.as_bool().then(|| {
            let len = width as usize * height as usize * 4;
            std::slice::from_raw_parts(bits as *const u8, len).to_vec()
        });
        SelectObject(hdc, old);
        let _ = DeleteObject(hbitmap.into());

        let mut bgra = pixels.ok_or_else(|| anyhow!("PrintWindow returned FALSE"))?;
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2); // BGRA -> RGBA
            px[3] = 0xff; // PrintWindow leaves alpha undefined
        }
        image::RgbaImage::from_raw(width as u32, height as u32, bgra)
            .ok_or_else(|| anyhow!("failed to construct image buffer"))
    }
}
