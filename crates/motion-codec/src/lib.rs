//! Animation frame encoders for Monet. Byte-in/byte-out codecs only: no document, no renderer,
//! no engine dependency. Frames are premultiplied RGBA8 with an explicit size; the caller owns
//! timeline evaluation and drawing.
//!
//! Ported from the engine's `motion_export.rs` / `motion_webm.rs` so the export formats live in
//! their own crate, the way the upstream Craft apps keep their format codecs in craft-libs.

use std::io::{Cursor, Write};

use image::codecs::gif::{GifEncoder, Repeat};
use image::codecs::png::PngEncoder;
use image::{Delay, Frame, ImageEncoder, RgbaImage};
use rav1e::prelude::*;

/// An animation frame: straight RGBA8 bytes, row-major, `w*h*4` long.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRgba {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl FrameRgba {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, &'static str> {
        if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
            return Err("frame size does not match its RGBA bytes");
        }
        Ok(Self { width, height, rgba })
    }
}

/// Encode straight RGBA frames as an animated GIF with the given per-frame delays (ms).
pub fn encode_gif(frames: &[FrameRgba], delays_ms: &[u32], repeating: bool) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        if repeating {
            enc.set_repeat(Repeat::Infinite).map_err(|e| format!("gif repeat: {e}"))?;
        }
        let mut imgs = Vec::with_capacity(frames.len());
        for (f, ms) in frames.iter().zip(delays_ms.iter()) {
            let img = RgbaImage::from_raw(f.width, f.height, f.rgba.clone()).ok_or("gif frame size mismatch")?;
            imgs.push(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms((*ms).max(10), 1)));
        }
        enc.encode_frames(imgs).map_err(|e| format!("gif encode: {e}"))?;
    }
    Ok(out)
}

/// Pack frames into a zip of numbered PNG files (`base_0001.png`, ...).
pub fn write_png_zip(frames: &[FrameRgba], base: &str) -> Result<Vec<u8>, String> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let digits = frames.len().to_string().len().max(4);
        for (i, f) in frames.iter().enumerate() {
            let name = format!("{base}_{:0digits$}.png", i + 1);
            let mut png = Vec::new();
            PngEncoder::new(&mut png)
                .write_image(&f.rgba, f.width, f.height, image::ExtendedColorType::Rgba8)
                .map_err(|e| format!("png encode: {e}"))?;
            zw.start_file(name, opts).map_err(|e| format!("zip entry: {e}"))?;
            zw.write_all(&png).map_err(|e| format!("zip write: {e}"))?;
        }
        zw.finish().map_err(|e| format!("zip finish: {e}"))?;
    }
    Ok(buf.into_inner())
}

/// WebM/AV1 encoder options.
#[derive(Clone, Debug, PartialEq)]
pub struct WebmOptions {
    /// Frames per second, 1..=120.
    pub fps: u32,
    /// Picture quality 1..=100 (quantizer `255 - 2*quality`; default 85 → 85).
    pub quality: u8,
    /// rav1e speed preset 0 (slowest) ..= 10 (fastest). Default 9 ≈ real time at 720p.
    pub speed: u8,
    /// Colour transparent parts are flattened onto (AV1-in-WebM here has no alpha).
    pub background: [u8; 3],
    /// Encoder threads (0 = one per core, at most 8).
    pub threads: usize,
    /// Tiles to split the picture into for parallel encoding (0 = auto).
    pub tiles: usize,
}

impl Default for WebmOptions {
    fn default() -> Self {
        Self { fps: 60, quality: 85, speed: 9, background: [255, 255, 255], threads: 0, tiles: 0 }
    }
}

impl WebmOptions {
    pub fn quantizer(&self) -> usize {
        255 - 2 * self.quality.clamp(1, 100) as usize
    }
}

// ---------------------------------------------------------------------------------- EBML

fn size_vint(n: u64) -> Vec<u8> {
    let mut len = 1;
    while len < 8 && n >= (1u64 << (7 * len)) - 1 {
        len += 1;
    }
    let mut out = vec![0u8; len];
    for (i, b) in out.iter_mut().enumerate() {
        *b = (n >> (8 * (len - 1 - i))) as u8;
    }
    out[0] |= 1 << (8 - len);
    out
}

fn elem(id: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(id.len() + 9 + payload.len());
    out.extend_from_slice(id);
    out.extend_from_slice(&size_vint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

fn uint_bytes(v: u64, min_len: usize) -> Vec<u8> {
    let mut len = min_len.max(1);
    while len < 8 && (v >> (8 * len)) != 0 {
        len += 1;
    }
    (0..len).rev().map(|i| (v >> (8 * i)) as u8).collect()
}

fn uint(id: &[u8], v: u64) -> Vec<u8> {
    elem(id, &uint_bytes(v, 1))
}

fn string(id: &[u8], s: &str) -> Vec<u8> {
    elem(id, s.as_bytes())
}

fn float64(id: &[u8], v: f64) -> Vec<u8> {
    elem(id, &v.to_be_bytes())
}

struct Block {
    ts_ms: u64,
    key: bool,
    data: Vec<u8>,
}

fn ebml_header() -> Vec<u8> {
    let mut p = Vec::new();
    p.extend(uint(&[0x42, 0x86], 1)); // EBMLVersion
    p.extend(uint(&[0x42, 0xF7], 1)); // EBMLReadVersion
    p.extend(uint(&[0x42, 0xF2], 4)); // EBMLMaxIDLength
    p.extend(uint(&[0x42, 0xF3], 8)); // EBMLMaxSizeLength
    p.extend(string(&[0x42, 0x82], "webm")); // DocType
    p.extend(uint(&[0x42, 0x87], 4)); // DocTypeVersion
    p.extend(uint(&[0x42, 0x85], 2)); // DocTypeReadVersion
    elem(&[0x1A, 0x45, 0xDF, 0xA3], &p)
}

fn seek(id: &[u8], position: u64) -> Vec<u8> {
    let mut p = elem(&[0x53, 0xAB], id);
    p.extend(elem(&[0x53, 0xAC], &uint_bytes(position, 4)));
    elem(&[0x4D, 0xBB], &p)
}

/// Builds the whole file. `blocks` are in presentation order; the first is a key frame.
fn mux(w: u32, h: u32, fps: u32, av1c: &[u8], blocks: &[Block]) -> Vec<u8> {
    let frame_ms = 1000.0 / fps as f64;
    let duration = blocks.len() as f64 * frame_ms;

    let mut info = uint(&[0x2A, 0xD7, 0xB1], 1_000_000); // TimestampScale: 1 ms
    info.extend(float64(&[0x44, 0x89], duration));
    info.extend(string(&[0x4D, 0x80], "figcraft"));
    info.extend(string(&[0x57, 0x41], "figcraft"));
    let info = elem(&[0x15, 0x49, 0xA9, 0x66], &info);

    let mut colour = uint(&[0x55, 0xB1], 1); // MatrixCoefficients: BT.709
    colour.extend(uint(&[0x55, 0xB9], 1)); // Range: broadcast (limited)
    colour.extend(uint(&[0x55, 0xBA], 1)); // TransferCharacteristics: BT.709
    colour.extend(uint(&[0x55, 0xBB], 1)); // Primaries: BT.709
    let mut video = uint(&[0xB0], w as u64);
    video.extend(uint(&[0xBA], h as u64));
    video.extend(elem(&[0x55, 0xB0], &colour));
    let mut track = uint(&[0xD7], 1); // TrackNumber
    track.extend(uint(&[0x73, 0xC5], 1)); // TrackUID
    track.extend(uint(&[0x83], 1)); // TrackType: video
    track.extend(uint(&[0x9C], 0)); // FlagLacing
    track.extend(string(&[0x86], "V_AV1"));
    track.extend(elem(&[0x63, 0xA2], av1c));
    track.extend(uint(&[0x23, 0xE3, 0x83], (frame_ms * 1e6).round() as u64)); // DefaultDuration
    track.extend(elem(&[0xE0], &video));
    let tracks = elem(&[0x16, 0x54, 0xAE, 0x6B], &elem(&[0xAE], &track));

    // clusters: one per key frame (and at most ~20 s long so block offsets fit in 16 bits)
    let mut clusters: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        let start = blocks[i].ts_ms;
        let mut body = uint(&[0xE7], start);
        let mut j = i;
        while j < blocks.len() && (j == i || (!blocks[j].key && blocks[j].ts_ms - start < 20_000)) {
            let b = &blocks[j];
            let mut sb = vec![0x81]; // track 1
            sb.extend_from_slice(&((b.ts_ms - start) as i16).to_be_bytes());
            sb.push(if b.key { 0x80 } else { 0x00 });
            sb.extend_from_slice(&b.data);
            body.extend(elem(&[0xA3], &sb));
            j += 1;
        }
        clusters.push((start, elem(&[0x1F, 0x43, 0xB6, 0x75], &body)));
        i = j;
    }

    // layout: SeekHead, Info, Tracks, Clusters, Cues (positions relative to the segment payload)
    let seekhead_len = {
        let mut p = seek(&[0x15, 0x49, 0xA9, 0x66], 0);
        p.extend(seek(&[0x16, 0x54, 0xAE, 0x6B], 0));
        p.extend(seek(&[0x1C, 0x53, 0xBB, 0x6B], 0));
        elem(&[0x11, 0x4D, 0x9B, 0x74], &p).len()
    };
    let info_pos = seekhead_len as u64;
    let tracks_pos = info_pos + info.len() as u64;
    let mut pos = tracks_pos + tracks.len() as u64;
    let mut cues = Vec::new();
    for (ts, c) in &clusters {
        let mut tp = uint(&[0xF7], 1);
        tp.extend(uint(&[0xF1], pos));
        let mut cp = uint(&[0xB3], *ts);
        cp.extend(elem(&[0xB7], &tp));
        cues.extend(elem(&[0xBB], &cp));
        pos += c.len() as u64;
    }
    let cues = elem(&[0x1C, 0x53, 0xBB, 0x6B], &cues);
    let mut sh = seek(&[0x15, 0x49, 0xA9, 0x66], info_pos);
    sh.extend(seek(&[0x16, 0x54, 0xAE, 0x6B], tracks_pos));
    sh.extend(seek(&[0x1C, 0x53, 0xBB, 0x6B], pos));
    let seekhead = elem(&[0x11, 0x4D, 0x9B, 0x74], &sh);
    debug_assert_eq!(seekhead.len(), seekhead_len);

    let mut seg = seekhead;
    seg.extend(info);
    seg.extend(tracks);
    for (_, c) in clusters {
        seg.extend(c);
    }
    seg.extend(cues);

    let mut out = ebml_header();
    out.extend(elem(&[0x18, 0x53, 0x80, 0x67], &seg));
    out
}

// ------------------------------------------------------------------------------ colour

/// BT.709 limited-range planes (4:2:0) of opaque `rgb`.
fn to_yuv420(rgb: &[u8], w: usize, h: usize) -> ([Vec<u8>; 3], usize) {
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);
    let mut y = vec![0u8; w * h];
    let mut u = vec![128u8; cw * ch];
    let mut v = vec![128u8; cw * ch];
    for (i, px) in rgb.chunks_exact(3).enumerate() {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        y[i] = (16.0 + (0.2126 * r + 0.7152 * g + 0.0722 * b) * (219.0 / 255.0)).round().clamp(16.0, 235.0) as u8;
    }
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut r, mut g, mut b, mut n) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for dy in 0..2 {
                for dx in 0..2 {
                    let (x, yy) = ((cx * 2 + dx).min(w - 1), (cy * 2 + dy).min(h - 1));
                    let p = (yy * w + x) * 3;
                    r += rgb[p] as f32;
                    g += rgb[p + 1] as f32;
                    b += rgb[p + 2] as f32;
                    n += 1.0;
                }
            }
            let (r, g, b) = (r / n, g / n, b / n);
            let k = 224.0 / 255.0;
            u[cy * cw + cx] = (128.0 + (-0.1146 * r - 0.3854 * g + 0.5 * b) * k).round().clamp(16.0, 240.0) as u8;
            v[cy * cw + cx] = (128.0 + (0.5 * r - 0.4542 * g - 0.0458 * b) * k).round().clamp(16.0, 240.0) as u8;
        }
    }
    ([y, u, v], cw)
}

/// Flatten premultiplied RGBA bytes onto `bg` and return straight RGB bytes (AV1 has no alpha).
fn flatten_rgb(rgba: &[u8], bg: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.chunks_exact(4) {
        let inv = 255 - px[3] as u32;
        for c in 0..3 {
            out.push((px[c] as u32 + (bg[c] as u32 * inv + 127) / 255).min(255) as u8);
        }
    }
    out
}

/// Strip a leading temporal delimiter OBU (Matroska stores temporal units without them).
fn strip_temporal_delimiter(data: &[u8]) -> &[u8] {
    if data.len() >= 2 && data[0] == 0x12 && data[1] == 0x00 { &data[2..] } else { data }
}


/// Encode the animation as an AV1 WebM. Returns the file bytes.
pub fn encode_webm(frames: &[FrameRgba], o: &WebmOptions) -> Result<Vec<u8>, String> {
    let fps = o.fps.clamp(1, 120);
    let total = frames.len() as u32;
    let (w, h) = if let Some(f) = frames.first() { (f.width, f.height) } else { return Err("no frames to encode".into()) };
    let (wu, hu) = (w as usize, h as usize);
    if wu == 0 || hu == 0 || wu % 2 == 1 || hu % 2 == 1 {
        return Err("WebM needs an even, non-zero picture size; lower the scale".into());
    }

    let mut enc = EncoderConfig::with_speed_preset(o.speed.min(10));
    enc.width = wu;
    enc.height = hu;
    enc.bit_depth = 8;
    enc.chroma_sampling = ChromaSampling::Cs420;
    enc.time_base = Rational::new(1, fps as u64);
    enc.quantizer = o.quantizer();
    enc.min_quantizer = 0;
    enc.low_latency = true;
    enc.min_key_frame_interval = 0;
    enc.max_key_frame_interval = (fps as u64 * 2).max(1);
    enc.pixel_range = PixelRange::Limited;
    enc.color_description = Some(ColorDescription {
        color_primaries: ColorPrimaries::BT709,
        transfer_characteristics: TransferCharacteristics::BT709,
        matrix_coefficients: MatrixCoefficients::BT709,
    });
    enc.speed_settings.scene_detection_mode = SceneDetectionSpeed::None;
    let threads = if o.threads == 0 { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).min(8) } else { o.threads };
    enc.tiles = if o.tiles > 0 { o.tiles } else { (wu * hu / (640 * 360)).clamp(1, threads.next_power_of_two()) };
    let cfg = Config::new().with_encoder_config(enc).with_threads(threads);
    let mut ctx: Context<u8> = cfg.new_context().map_err(|e| format!("AV1 encoder: {e}"))?;
    let av1c = ctx.container_sequence_header();

    let mut blocks: Vec<Block> = Vec::with_capacity(frames.len());
    let take = |ctx: &mut Context<u8>, blocks: &mut Vec<Block>| -> Result<(), String> {
        loop {
            match ctx.receive_packet() {
                Ok(p) => {
                    let ts_ms = (p.input_frameno as f64 * 1000.0 / fps as f64).round() as u64;
                    blocks.push(Block { ts_ms, key: p.frame_type == FrameType::KEY, data: strip_temporal_delimiter(&p.data).to_vec() });
                }
                Err(EncoderStatus::Encoded) => continue,
                Err(EncoderStatus::NeedMoreData) | Err(EncoderStatus::LimitReached) => return Ok(()),
                Err(e) => return Err(format!("AV1 encoder: {e}")),
            }
        }
    };
    let mut sent = 0u32;
    for f in frames {
        let rgb = flatten_rgb(&f.rgba, o.background);
        let ([py, pu, pv], cw) = to_yuv420(&rgb, wu, hu);
        let mut frame = ctx.new_frame();
        frame.planes[0].copy_from_raw_u8(&py, wu, 1);
        frame.planes[1].copy_from_raw_u8(&pu, cw, 1);
        frame.planes[2].copy_from_raw_u8(&pv, cw, 1);
        ctx.send_frame(frame).map_err(|e| format!("AV1 encoder: {e}"))?;
        sent += 1;
        take(&mut ctx, &mut blocks)?;
    }
    ctx.flush();
    take(&mut ctx, &mut blocks)?;
    if blocks.len() as u32 != sent || blocks.is_empty() {
        return Err(format!("AV1 encoder produced {} packets for {sent} frames", blocks.len()));
    }
    blocks.sort_by_key(|b| b.ts_ms);
    let _ = total;
    Ok(mux(w, h, fps, &av1c, &blocks))
}

// ---------------------------------------------------------------------------------- MP4 / H.264

use craft_media::isobmff::{AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriterOptions, WriteSample};
use craft_video::h264::{Encoder as H264Encoder, EncoderConfig as H264Config, PacketFormat, RateControl, YuvFrame};

/// H.264-in-MP4 encoder options.
#[derive(Clone, Debug, PartialEq)]
pub struct Mp4Options {
    /// Frames per second, 1..=120.
    pub fps: u32,
    /// CRF quality: lower is better quality and larger (default 23, roughly 18..30).
    pub crf: f32,
}

impl Default for Mp4Options {
    fn default() -> Self {
        Self { fps: 30, crf: 23.0 }
    }
}

/// Encode frames as an H.264/AVC video in an MP4 container. Frames are straight RGBA8; the
/// encoder flattens alpha onto `background` (MP4/H.264 here carries no alpha) and downsamples to
/// 4:2:0, so both dimensions must be even and non-zero.
pub fn encode_mp4(frames: &[FrameRgba], o: &Mp4Options, background: [u8; 3]) -> Result<Vec<u8>, String> {
    let fps = o.fps.clamp(1, 120);
    let (w, h) = if let Some(f) = frames.first() { (f.width, f.height) } else { return Err("no frames to encode".into()) };
    if w == 0 || h == 0 || w % 2 == 1 || h % 2 == 1 {
        return Err("MP4 needs an even, non-zero picture size; lower the scale".into());
    }
    if frames.iter().any(|f| f.width != w || f.height != h) {
        return Err("all frames must have the same size".into());
    }

    let mut cfg = H264Config::new(w, h, fps, 1);
    cfg.format = PacketFormat::LengthPrefixed; // MP4 samples
    cfg.rate = RateControl::Crf(o.crf);
    let mut enc = H264Encoder::new(cfg).map_err(|e| format!("H.264 encoder: {e}"))?;
    let avcc = enc.avcc();
    let avc = AvcConfig::parse(&avcc).map_err(|e| format!("H.264 avcC: {e}"))?;

    let timescale = 90_000u32;
    let step = (timescale / fps).max(1);
    let mut writer = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).map_err(|e| format!("MP4 writer: {e}"))?;
    let track = writer
        .add_track(TrackConfig::new(SampleEntry::avc(avc, w as u16, h as u16), timescale))
        .map_err(|e| format!("MP4 track: {e}"))?;

    // Samples are written in decode order; composition_offset restores presentation order.
    let mut write = |data: &[u8], key: bool, cto: i64| -> Result<(), String> {
        writer
            .write_sample(track, WriteSample { data, duration: step, composition_offset: cto as i32, is_sync: key })
            .map_err(|e| format!("MP4 sample: {e}"))
    };
    let mut sent = 0usize;
    let mut last_pts = 0i64;
    for f in frames {
        let rgb = flatten_rgb(&f.rgba, background);
        let ([y, u, v], cw) = to_yuv420(&rgb, w as usize, h as usize);
        let pic = YuvFrame { y: &y, u: &u, v: &v, y_stride: w as usize, uv_stride: cw };
        // pts is the presentation timestamp and must advance by a constant step per picture
        // (a packet may carry several NALs; they all belong to this one picture).
        for p in enc.encode(&pic, last_pts).map_err(|e| format!("H.264 encode: {e}"))? {
            write(&p.data, p.keyframe, p.pts - p.dts)?;
        }
        last_pts += 1;
        sent += 1;
    }
    for p in enc.flush() {
        write(&p.data, p.keyframe, p.pts - p.dts)?;
    }
    if sent == 0 {
        return Err("H.264 encoder produced no frames".into());
    }
    let out = writer.finish_faststart().map_err(|e| format!("MP4 finish: {e}"))?.into_inner();
    Ok(out)
}
