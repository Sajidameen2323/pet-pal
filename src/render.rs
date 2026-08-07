//! Per-pixel-alpha presentation via `UpdateLayeredWindow`.
//!
//! The whole renderer is a single DIB section that we scribble into on the CPU
//! and hand to the compositor. There is no GPU context, no swapchain and no
//! per-frame allocation: a 32x32 sprite at 3x is a 96x96 buffer (36 KB) and a
//! frame costs one nearest-neighbour upscale plus one kernel call.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, POINT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC,
    HGDIOBJ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, HICON, ICONINFO, ULW_ALPHA,
};

use crate::sprites::Frame;

pub struct Canvas {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    pub w: i32,
    pub h: i32,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Canvas {
        let w = w.max(1);
        let h = h.max(1);
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative height selects a top-down DIB, matching our row order.
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut core::ffi::c_void = null_mut();
            let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            let old = SelectObject(dc, bmp as HGDIOBJ);
            Canvas {
                dc,
                bmp,
                old,
                bits: bits as *mut u32,
                w,
                h,
            }
        }
    }

    #[inline]
    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: `bits` points at a w*h u32 DIB section owned by this Canvas
        // and kept alive until Drop.
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
    }

    #[inline]
    fn pixels_ref(&self) -> &[u32] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.w * self.h) as usize) }
    }

    /// Draw one sprite frame scaled to fill the entire canvas.
    ///
    /// Every destination pixel is written, so there is no separate clear pass.
    /// `mirror` flips horizontally for a left-facing creature, which is why
    /// sprite sheets only ever need right-facing art.
    pub fn draw_frame(&mut self, f: &Frame, src_w: u32, src_h: u32, mirror: bool) {
        let (sw, sh) = (src_w as i32, src_h as i32);
        if sw <= 0 || sh <= 0 || f.px.len() < (sw * sh) as usize {
            self.pixels().fill(0);
            return;
        }
        let (dw, dh) = (self.w, self.h);
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, (dw * dh) as usize) };

        // Fixed-point 16.16 stepping avoids a divide per pixel.
        let step_x = ((sw as u32) << 16) / dw as u32;
        let step_y = ((sh as u32) << 16) / dh as u32;

        let mut sy_fx = 0u32;
        for y in 0..dh {
            let sy = (sy_fx >> 16).min(sh as u32 - 1) as i32;
            sy_fx = sy_fx.wrapping_add(step_y);
            let src_row = &f.px[(sy * sw) as usize..(sy * sw + sw) as usize];
            let dst_row = &mut dst[(y * dw) as usize..(y * dw + dw) as usize];

            let mut sx_fx = 0u32;
            if mirror {
                for d in dst_row.iter_mut() {
                    let sx = (sx_fx >> 16).min(sw as u32 - 1) as usize;
                    sx_fx = sx_fx.wrapping_add(step_x);
                    *d = src_row[sw as usize - 1 - sx];
                }
            } else {
                for d in dst_row.iter_mut() {
                    let sx = (sx_fx >> 16).min(sw as u32 - 1) as usize;
                    sx_fx = sx_fx.wrapping_add(step_x);
                    *d = src_row[sx];
                }
            }
        }
    }

    /// Alpha of a canvas pixel — used by `WM_NCHITTEST` so clicks pass through
    /// the transparent parts of the window.
    pub fn alpha_at(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0;
        }
        (self.pixels_ref()[(y * self.w + x) as usize] >> 24) as u8
    }

    /// Push the canvas to the screen, moving the window in the same call.
    pub fn present(&self, hwnd: HWND, x: i32, y: i32, opacity: u8) {
        let pt_dst = POINT { x, y };
        let size = SIZE {
            cx: self.w,
            cy: self.h,
        };
        let pt_src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: opacity,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            // A NULL destination DC means "the screen", saving a GetDC/ReleaseDC
            // pair on every frame.
            windows_sys::Win32::UI::WindowsAndMessaging::UpdateLayeredWindow(
                hwnd,
                null_mut(),
                &pt_dst,
                &size,
                self.dc,
                &pt_src,
                0,
                &blend,
                ULW_ALPHA,
            );
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp as HGDIOBJ);
            DeleteDC(self.dc);
        }
    }
}

/// Build an `HICON` from a sprite frame, for the notification area.
///
/// Icons want *straight* alpha (Windows premultiplies internally when drawing),
/// so we undo the premultiplication our canvas format uses.
pub fn icon_from_frame(f: &Frame, src_w: u32, src_h: u32, size: i32) -> HICON {
    let size = size.max(8);
    let mut color = Canvas::new(size, size);
    color.draw_frame(f, src_w, src_h, false);
    for p in color.pixels() {
        let a = (*p >> 24) & 0xff;
        if a != 0 && a != 255 {
            let un = |sh: u32| ((((*p >> sh) & 0xff) * 255) / a).min(255);
            *p = (a << 24) | (un(16) << 16) | (un(8) << 8) | un(0);
        }
    }

    // A 32bpp colour bitmap supplies the shape via its alpha channel, but
    // CreateIconIndirect still requires a mask bitmap to be present.
    let mask = Canvas::new(size, size);

    unsafe {
        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask.bmp,
            hbmColor: color.bmp,
        };
        CreateIconIndirect(&info)
    }
}

pub fn destroy_icon(icon: HICON) {
    if !icon.is_null() {
        unsafe { DestroyIcon(icon) };
    }
}
