//! The compositor's built-in pointer cursor.
//!
//! A compositor has to draw the pointer itself: there is no hardware cursor
//! unless one is programmed, and a client only supplies an image once it has
//! called `wl_pointer.set_cursor` — which it cannot do until the pointer is
//! already over its surface. Without a built-in image the pointer is invisible
//! over the compositor's own chrome and on an empty output, which on a DRM boot
//! means an invisible mouse.
//!
//! Rather than depend on an XCursor theme being installed and parsed, the arrow
//! is spelled out here as a mask. It is the classic left-pointing arrow: `#` is
//! the outline, `.` the fill, and a space is transparent. Keeping it legible in
//! source means the shape can be checked by reading it.

/// The arrow, top-left aligned; the hotspot is the tip at (0, 0).
const ARROW: &[&str] = &[
    "#                 ",
    "##                ",
    "#.#               ",
    "#..#              ",
    "#...#             ",
    "#....#            ",
    "#.....#           ",
    "#......#          ",
    "#.......#         ",
    "#........#        ",
    "#.........#       ",
    "#..........#      ",
    "#...........#     ",
    "#............#    ",
    "#.....########    ",
    "#..#..#           ",
    "#.# #..#          ",
    "##  #..#          ",
    "#    #..#         ",
    "     #..#         ",
    "      ##          ",
];

/// Bytes per pixel in the cursor image.
const BPP: usize = 4;

/// A cursor image ready to upload: premultiplied RGBA, plus the hotspot the
/// image should be drawn relative to.
pub struct CursorBitmap {
    pub width: i32,
    pub height: i32,
    /// Offset from the image's top-left to the point that tracks the pointer.
    pub hotspot: (i32, i32),
    pixels: Vec<u8>,
}

impl CursorBitmap {
    /// Build the built-in arrow: white fill, black outline, transparent
    /// elsewhere. Premultiplied, since that is what the renderer composites.
    pub fn arrow() -> Self {
        let height = ARROW.len() as i32;
        let width = ARROW.iter().map(|row| row.len()).max().unwrap_or(0) as i32;
        let mut pixels = vec![0u8; (width * height) as usize * BPP];

        for (y, row) in ARROW.iter().enumerate() {
            for (x, cell) in row.bytes().enumerate() {
                let rgba: [u8; 4] = match cell {
                    b'#' => [0, 0, 0, 255],
                    b'.' => [255, 255, 255, 255],
                    _ => continue,
                };
                let offset = (y * width as usize + x) * BPP;
                pixels[offset..offset + BPP].copy_from_slice(&rgba);
            }
        }

        CursorBitmap {
            width,
            height,
            hotspot: (0, 0),
            pixels,
        }
    }

    /// The image: RGBA8, premultiplied, row-major.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_is_rectangular_and_sized_from_the_mask() {
        let cursor = CursorBitmap::arrow();
        assert_eq!(cursor.height, ARROW.len() as i32);
        assert_eq!(
            cursor.pixels().len(),
            (cursor.width * cursor.height) as usize * BPP
        );
    }

    #[test]
    fn the_hotspot_pixel_is_opaque() {
        // The hotspot is the arrow's tip and must be on the drawn part of the
        // image — a hotspot over a transparent pixel would leave the pointer
        // visually offset from where clicks land.
        let cursor = CursorBitmap::arrow();
        let (hx, hy) = cursor.hotspot;
        let offset = ((hy * cursor.width + hx) as usize) * BPP;
        assert_eq!(cursor.pixels()[offset + 3], 255, "hotspot pixel is opaque");
    }

    #[test]
    fn arrow_has_both_outline_and_fill() {
        let cursor = CursorBitmap::arrow();
        let px = cursor.pixels();
        let opaque = |i: usize| px[i * BPP + 3] == 255;
        let black = |i: usize| opaque(i) && px[i * BPP] == 0;
        let white = |i: usize| opaque(i) && px[i * BPP] == 255;
        let count = (cursor.width * cursor.height) as usize;
        assert!((0..count).any(black), "arrow has an outline");
        assert!((0..count).any(white), "arrow has a fill");
        assert!(
            (0..count).any(|i| px[i * BPP + 3] == 0),
            "arrow has transparent margin"
        );
    }
}
