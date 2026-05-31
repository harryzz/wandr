//! Runtime fit-test for `photon-rs` on `wasm32-wasip2` — a `wasi:cli/command`
//! that runs the realistic pipeline with NO host wiring or input file:
//!   synthesize RGBA → PNG encode (`image`) → PNG decode (`image`)
//!   → photon effects (grayscale/contrast/filter/sepia/resize) → PNG re-encode.
//! Proves photon's processing + the `image` codec actually execute under wasmtime
//! on our component-model target. Architecture note: use the pure-Rust `image`
//! crate for decode/encode and `photon-rs` (default-features off, no wasm-bindgen)
//! for the effects — photon's own `native::image_to_bytes` returns raw pixels here.
//! (Rust command: Kotlin/Wasm's command adapter throws at init; Rust is clean.)

use std::io::Cursor;

use image::{DynamicImage, RgbaImage};
use photon_rs::PhotonImage;

fn encode_png(rgba: Vec<u8>, w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_raw(w, h, rgba).expect("rgba buffer");
    let mut buf = Vec::new();
    DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .expect("encode png");
    buf
}

fn main() {
    // 1. Synthesize a 96×96 RGBA gradient, encode to PNG (the codec path).
    let (w, h) = (96u32, 96u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 128, 255]);
        }
    }
    let png_in = encode_png(rgba, w, h);
    let png_ok = png_in.len() > 8 && &png_in[1..4] == b"PNG";
    eprintln!("[photon] encoded input PNG = {} bytes (PNG sig: {})", png_in.len(), png_ok);

    // 2. Decode the PNG back to pixels (image crate) → into a PhotonImage.
    let decoded = image::load_from_memory(&png_in).expect("decode png").to_rgba8();
    let (dw, dh) = decoded.dimensions();
    let mut img = PhotonImage::new(decoded.into_raw(), dw, dh);
    eprintln!("[photon] decoded {}x{} → PhotonImage", dw, dh);

    // 3. Photon effects pipeline.
    photon_rs::monochrome::grayscale(&mut img);
    photon_rs::effects::adjust_contrast(&mut img, 25.0);
    photon_rs::filters::filter(&mut img, "twenties");
    photon_rs::monochrome::sepia(&mut img);
    let small = photon_rs::transform::resize(&img, w / 2, h / 2, photon_rs::transform::SamplingFilter::Lanczos3);
    eprintln!(
        "[photon] effects ok → resized {}x{} raw={} bytes",
        small.get_width(),
        small.get_height(),
        small.get_raw_pixels().len()
    );

    // 4. Re-encode the processed image to PNG (full round-trip).
    let png_out = encode_png(small.get_raw_pixels(), small.get_width(), small.get_height());
    let out_ok = png_out.len() > 8 && &png_out[1..4] == b"PNG";
    eprintln!("[photon] re-encoded output PNG = {} bytes (PNG sig: {})", png_out.len(), out_ok);

    eprintln!(
        "[photon] RESULT: photon effects + image codec on wasm32-wasip2 = {}",
        if png_ok && out_ok { "PASS \u{2713}" } else { "FAIL" }
    );
}
