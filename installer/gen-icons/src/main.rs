//! Renders the Windows `.ico` files used by the MSI shortcuts (issue #230) from
//! the project logo `imgs/scope-logo.png`.
//!
//! The logo is a wide banner: a square oscilloscope glyph on the left followed
//! by the "scope" wordmark. We auto-detect the glyph's bounding box (the dark
//! pixels in the left half), crop it to a square, and emit one base icon plus
//! four per-command variants. Each variant carries a small coloured badge in
//! the bottom-right corner with a white Font Awesome glyph, so they read
//! differently at a glance:
//!
//!   scope.ico                 — plain glyph (the executable's own icon)
//!   scope-serial.ico          — green  badge, ethernet glyph  (`scope serial`)
//!   scope-serial-headless.ico — blue   badge, terminal glyph  (`scope --headless serial`)
//!   scope-rtt.ico             — purple badge, microchip glyph (`scope rtt`)
//!   scope-rtt-headless.ico    — orange badge, terminal glyph  (`scope --headless rtt`)
//!
//! The glyphs are Font Awesome Free 6 (CC BY 4.0) — see installer/icons/README.md.
//! Run from the repo root:  cargo run --manifest-path installer/gen-icons/Cargo.toml

use image::{imageops, Rgba, RgbaImage};
use resvg::{tiny_skia, usvg};
use std::error::Error;
use std::path::PathBuf;

const ETHERNET_SVG: &str = include_str!("../glyphs/ethernet.svg");
const TERMINAL_SVG: &str = include_str!("../glyphs/terminal.svg");
const MICROCHIP_SVG: &str = include_str!("../glyphs/microchip.svg");

/// One command's badge: fill colour + the glyph drawn on it.
struct Badge {
    color: [u8; 3],
    glyph: &'static str,
}

/// Icon sizes packed into each `.ico` (Windows picks the best fit per context).
const SIZES: [u32; 6] = [16, 32, 48, 64, 128, 256];

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.join("..").join("..");
    let logo_path = repo_root.join("imgs").join("scope-logo.png");
    let out_dir = manifest_dir.join("..").join("icons");
    std::fs::create_dir_all(&out_dir)?;

    let logo = image::open(&logo_path)?.to_rgba8();
    let base = crop_square_glyph(&logo);

    // (filename, optional badge)
    let variants: [(&str, Option<Badge>); 5] = [
        ("scope.ico", None),
        (
            "scope-serial.ico",
            Some(Badge {
                color: [46, 204, 113],
                glyph: ETHERNET_SVG,
            }), // green
        ),
        (
            "scope-serial-headless.ico",
            Some(Badge {
                color: [52, 152, 219],
                glyph: TERMINAL_SVG,
            }), // blue
        ),
        (
            "scope-rtt.ico",
            Some(Badge {
                color: [155, 89, 182],
                glyph: MICROCHIP_SVG,
            }), // purple
        ),
        (
            "scope-rtt-headless.ico",
            Some(Badge {
                color: [230, 126, 34],
                glyph: TERMINAL_SVG,
            }), // orange
        ),
    ];

    for (name, badge) in variants {
        write_ico(&base, badge.as_ref(), &out_dir.join(name))?;
        println!("wrote {}", out_dir.join(name).display());
    }

    Ok(())
}

/// Crop the square oscilloscope glyph out of the wide logo. The glyph is the
/// leftmost block of dark artwork; the "scope" wordmark follows after a gap of
/// background. We isolate that first block (stop at the first vertical gap of
/// empty columns), take its bounding box, pad it, and square it off — so the
/// wordmark never bleeds into the icon regardless of exact pixel positions.
fn crop_square_glyph(logo: &RgbaImage) -> RgbaImage {
    let (w, h) = logo.dimensions();
    let is_dark = |x: u32, y: u32| {
        let p = logo.get_pixel(x, y);
        (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) < 80.0
    };
    let col_has_dark: Vec<bool> = (0..w).map(|x| (0..h).any(|y| is_dark(x, y))).collect();

    // Left edge: first column containing the glyph.
    let left = col_has_dark.iter().position(|&d| d).unwrap_or(0) as u32;
    // Right edge: walk right until a run of empty columns marks the gap before
    // the wordmark (~2.5% of the banner width).
    let gap_needed = (w / 40).max(3);
    let (mut right, mut gap) = (left, 0u32);
    for x in left..w {
        if col_has_dark[x as usize] {
            right = x;
            gap = 0;
        } else {
            gap += 1;
            if gap >= gap_needed {
                break;
            }
        }
    }

    // Vertical extent within the glyph's columns.
    let (mut top, mut bot) = (h, 0u32);
    for x in left..=right {
        for y in 0..h {
            if is_dark(x, y) {
                top = top.min(y);
                bot = bot.max(y);
            }
        }
    }

    // Pad ~8% of the glyph size so it isn't flush against the icon edge.
    let bbox_w = right - left;
    let bbox_h = bot.saturating_sub(top);
    let pad = (bbox_w.max(bbox_h) / 12).max(4);
    let side = bbox_w.max(bbox_h) + 2 * pad;

    // Centre the square on the bounding box, clamped inside the image.
    let cx = (left + right) / 2;
    let cy = (top + bot) / 2;
    let x0 = cx.saturating_sub(side / 2).min(w.saturating_sub(side));
    let y0 = cy.saturating_sub(side / 2).min(h.saturating_sub(side));
    let side = side.min(w - x0).min(h - y0);

    imageops::crop_imm(logo, x0, y0, side, side).to_image()
}

/// Encode `base` (optionally badged) into a multi-size `.ico`.
fn write_ico(
    base: &RgbaImage,
    badge: Option<&Badge>,
    path: &std::path::Path,
) -> Result<(), Box<dyn Error>> {
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in SIZES {
        let mut img = imageops::resize(base, size, size, imageops::FilterType::Lanczos3);
        if let Some(badge) = badge {
            draw_badge(&mut img, badge);
        }
        let icon = ico::IconImage::from_rgba_data(size, size, img.into_raw());
        dir.add_entry(ico::IconDirEntry::encode(&icon)?);
    }
    dir.write(std::fs::File::create(path)?)?;
    Ok(())
}

/// Draw a small coloured disc (white ring for contrast) in the bottom-right
/// corner and stamp the badge's white glyph onto it.
fn draw_badge(img: &mut RgbaImage, badge: &Badge) {
    let (w, h) = img.dimensions();
    // Compact badge: ~18% radius (≈36% diameter), tucked into the corner.
    let radius = (w as f32 * 0.18).round() as i32;
    let ring = ((w as f32 * 0.025).round() as i32).max(1);
    let margin = (w as f32 * 0.05).round() as i32;
    let cx = w as i32 - radius - margin;
    let cy = h as i32 - radius - margin;

    let fill = Rgba([badge.color[0], badge.color[1], badge.color[2], 255]);
    let white = Rgba([255, 255, 255, 255]);
    let inner = radius.pow(2);
    let outer = (radius + ring).pow(2);

    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let d2 = (x - cx).pow(2) + (y - cy).pow(2);
            if d2 <= inner {
                img.put_pixel(x as u32, y as u32, fill);
            } else if d2 <= outer {
                img.put_pixel(x as u32, y as u32, white);
            }
        }
    }

    // Stamp the white glyph, sized to sit comfortably inside the disc. Skip it
    // at tiny sizes where it would just be mud — the colour carries there.
    let glyph_box = (radius as f32 * 1.15) as u32;
    if glyph_box >= 10 {
        let glyph = rasterize_glyph(badge.glyph, glyph_box);
        let gx = cx - glyph_box as i32 / 2;
        let gy = cy - glyph_box as i32 / 2;
        for gy_off in 0..glyph_box as i32 {
            for gx_off in 0..glyph_box as i32 {
                let a = glyph.get_pixel(gx_off as u32, gy_off as u32)[3];
                if a == 0 {
                    continue;
                }
                let (px, py) = (gx + gx_off, gy + gy_off);
                if px < 0 || py < 0 || px as u32 >= w || py as u32 >= h {
                    continue;
                }
                // Blend white over the disc with the glyph's coverage.
                let dst = img.get_pixel_mut(px as u32, py as u32);
                for c in 0..3 {
                    dst[c] = (255 * a as u32 / 255 + dst[c] as u32 * (255 - a as u32) / 255) as u8;
                }
            }
        }
    }
}

/// Rasterize a Font Awesome SVG (recoloured white) into a `size`×`size` RGBA
/// buffer, scaled to fit and centred.
fn rasterize_glyph(svg: &str, size: u32) -> RgbaImage {
    // Font Awesome paths inherit `fill`, so setting it white on the root recolours the glyph.
    let svg = svg.replacen("<svg ", "<svg fill=\"#ffffff\" ", 1);
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default())
        .expect("Font Awesome glyph SVG should parse");

    let ts = tree.size();
    let scale = (size as f32 / ts.width()).min(size as f32 / ts.height());
    let tx = (size as f32 - ts.width() * scale) / 2.0;
    let ty = (size as f32 - ts.height() * scale) / 2.0;
    let transform = tiny_skia::Transform::from_translate(tx, ty).pre_scale(scale, scale);

    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("non-zero pixmap");
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    RgbaImage::from_raw(size, size, pixmap.data().to_vec()).expect("pixmap matches dimensions")
}
