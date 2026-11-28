//! Animation frame encoders for Monet. Byte-in/byte-out codecs only: no document, no renderer,
//! no engine dependency. Frames are straight RGBA8 with an explicit size; the caller owns
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
        let mut enc = GifWriter::new(&mut out, *delays_ms.first().unwrap_or(&10), repeating)?;
        for (f, ms) in frames.iter().zip(delays_ms.iter()) {
            enc.set_delay(*ms);
            enc.push(f.clone())?;
        }
    }
    Ok(out)
}

/// Streaming GIF encoder: push frames as they are drawn, in order. Avoids buffering every frame
/// (and its RGBA copy) just to start encoding — the old slice API copied the whole animation.
pub struct GifWriter<'a> {
    enc: GifEncoder<&'a mut Vec<u8>>,
    delay_ms: u32,
    any: bool,
}

impl<'a> GifWriter<'a> {
    /// `delay_ms` is the delay of the frames until [`set_delay`](Self::set_delay) changes it.
    pub fn new(out: &'a mut Vec<u8>, delay_ms: u32, repeating: bool) -> Result<Self, String> {
        let mut enc = GifEncoder::new(out);
        if repeating {
            enc.set_repeat(Repeat::Infinite).map_err(|e| format!("gif repeat: {e}"))?;
        }
        Ok(Self { enc, delay_ms: delay_ms.max(10), any: false })
    }

    pub fn set_delay(&mut self, delay_ms: u32) {
        self.delay_ms = delay_ms.max(10);
    }

    pub fn push(&mut self, f: FrameRgba) -> Result<(), String> {
        let img = RgbaImage::from_raw(f.width, f.height, f.rgba).ok_or("gif frame size mismatch")?;
        let delay = Delay::from_numer_denom_ms(self.delay_ms, 1);
        self.enc.encode_frame(Frame::from_parts(img, 0, 0, delay)).map_err(|e| format!("gif encode: {e}"))?;
        self.any = true;
        Ok(())
    }

    /// Drops the encoder (writing the GIF trailer). Fails when nothing was pushed.
    pub fn finish(self) -> Result<(), String> {
        if !self.any {
            return Err("no frames to encode".into());
        }
        Ok(())
    }
}

/// Pack frames into a zip of numbered PNG files (`base_0001.png`, ...).
pub fn write_png_zip(frames: &[FrameRgba], base: &str) -> Result<Vec<u8>, String> {
    let mut zw = PngZipWriter::new(base, frames.len())?;
    for f in frames {
        zw.push(f.clone())?;
    }
    zw.finish()
}

/// Streaming PNG-zip writer: each frame is PNG-encoded and appended as it arrives, so a long
/// animation never has to be held in memory as raw RGBA.
pub struct PngZipWriter {
    zw: Option<zip::ZipWriter<Cursor<Vec<u8>>>>,
    base: String,
    digits: usize,
    opts: zip::write::SimpleFileOptions,
    index: usize,
}

impl PngZipWriter {
    pub fn new(base: &str, count: usize) -> Result<Self, String> {
        Ok(Self {
            zw: Some(zip::ZipWriter::new(Cursor::new(Vec::new()))),
            base: base.to_string(),
            digits: count.to_string().len().max(4),
            opts: zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            index: 0,
        })
    }

    pub fn push(&mut self, f: FrameRgba) -> Result<(), String> {
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&f.rgba, f.width, f.height, image::ExtendedColorType::Rgba8)
            .map_err(|e| format!("png encode: {e}"))?;
        let zw = self.zw.as_mut().ok_or("png zip already finished")?;
        self.index += 1;
        let name = format!("{}_{:0digits$}.png", self.base, self.index, digits = self.digits);
        zw.start_file(name, self.opts).map_err(|e| format!("zip entry: {e}"))?;
        zw.write_all(&png).map_err(|e| format!("zip write: {e}"))
    }

    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        let zw = self.zw.take().ok_or("png zip already finished")?;
        zw.finish().map(|c| c.into_inner()).map_err(|e| format!("zip finish: {e}"))
    }
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
    /// Encoder threads (0 = one per core, capped at 32; an explicit count is used as-is).
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

/// Composite **straight** RGBA8 pixels onto `bg` and convert to BT.709 limited-range 4:2:0
/// planes, filling the caller's reusable buffers.
///
/// Integer fixed-point maths instead of the previous per-pixel `f32` loop: a 1080p frame is ~2M
/// pixels and video exports do this for every frame. The buffers are reused across frames so a
/// long export allocates them once, not three times per frame.
#[allow(clippy::too_many_arguments)]
pub fn rgba_to_yuv420_into(rgba: &[u8], w: usize, h: usize, bg: [u8; 3], y: &mut [u8], u: &mut [u8], v: &mut [u8]) {
    #[inline]
    fn over(c: u8, a: u8, bg: u8) -> u32 {
        // Straight alpha: multiply the source by its coverage. Treating `c` as already
        // premultiplied lightens semi-transparent colours (255 red at 50% became 382).
        (c as u32 * a as u32 + bg as u32 * (255 - a as u32) + 127) / 255
    }
    for (i, px) in rgba.chunks_exact(4).enumerate() {
        let a = px[3];
        let (r, g, b) = (over(px[0], a, bg[0]), over(px[1], a, bg[1]), over(px[2], a, bg[2]));
        // BT.709 limited range: Y = 16 + (47r + 157g + 16b) / 256
        y[i] = (16 + ((47 * r + 157 * g + 16 * b + 128) >> 8)).clamp(16, 235) as u8;
    }
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
            for dy in 0..2 {
                let yy = (cy * 2 + dy).min(h - 1);
                for dx in 0..2 {
                    let x = (cx * 2 + dx).min(w - 1);
                    let p = (yy * w + x) * 4;
                    sr += over(rgba[p], rgba[p + 3], bg[0]);
                    sg += over(rgba[p + 1], rgba[p + 3], bg[1]);
                    sb += over(rgba[p + 2], rgba[p + 3], bg[2]);
                    n += 1;
                }
            }
            // BT.709 U/V on the 2x2 average. Each triplet sums to zero, so neutral grey
            // (including white) lands exactly on 128 rather than one code below it.
            let ar = (sr + n / 2) / n;
            let ag = (sg + n / 2) / n;
            let ab = (sb + n / 2) / n;
            let i = cy * cw + cx;
            u[i] = (((-26 * ar as i32 - 87 * ag as i32 + 113 * ab as i32 + 128) >> 8) + 128).clamp(16, 240) as u8;
            v[i] = (((112 * ar as i32 - 102 * ag as i32 - 10 * ab as i32 + 128) >> 8) + 128).clamp(16, 240) as u8;
        }
    }
}

/// Size of the three reusable plane buffers of a `w`×`h` picture.
pub fn yuv_sizes(w: usize, h: usize) -> (usize, usize) {
    (w * h, w.div_ceil(2) * h.div_ceil(2))
}

/// Strip a leading temporal delimiter OBU (Matroska stores temporal units without them).
fn strip_temporal_delimiter(data: &[u8]) -> &[u8] {
    if data.len() >= 2 && data[0] == 0x12 && data[1] == 0x00 { &data[2..] } else { data }
}


/// Encode the animation as an AV1 WebM. Returns the file bytes.
pub fn encode_webm(frames: &[FrameRgba], o: &WebmOptions) -> Result<Vec<u8>, String> {
    let (w, h) = if let Some(f) = frames.first() { (f.width, f.height) } else { return Err("no frames to encode".into()) };
    let mut enc = WebmEncoder::new(w, h, o)?;
    for f in frames {
        enc.push(f.clone())?;
    }
    enc.finish()
}

/// Streaming AV1-in-WebM encoder: frames are converted, encoded and packetised as they arrive, so
/// the renderer can draw the next frame while this one is being encoded and no full frame list is
/// buffered.
pub struct WebmEncoder {
    ctx: Context<u8>,
    w: u32,
    h: u32,
    fps: u32,
    av1c: Vec<u8>,
    blocks: Vec<Block>,
    sent: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    background: [u8; 3],
}

impl WebmEncoder {
    pub fn new(w: u32, h: u32, o: &WebmOptions) -> Result<Self, String> {
        let fps = o.fps.clamp(1, 120);
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
        // Export is a batch job: use every core (up to 32), and split the picture into enough
        // tiles for them to work in parallel. The old cap of 8 threads / 8 tiles left most of a
        // 24-core machine idle and was the main reason AV1 export crawled.
        let threads = if o.threads == 0 { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).min(32) } else { o.threads };
        enc.tiles = if o.tiles > 0 { o.tiles } else { (wu * hu / (320 * 180)).clamp(1, threads.next_power_of_two()).min(32) };
        let cfg = Config::new().with_encoder_config(enc).with_threads(threads);
        let ctx: Context<u8> = cfg.new_context().map_err(|e| format!("AV1 encoder: {e}"))?;
        let av1c = ctx.container_sequence_header();
        let (ys, cs) = yuv_sizes(wu, hu);
        Ok(Self {
            ctx,
            w,
            h,
            fps,
            av1c,
            blocks: Vec::new(),
            sent: 0,
            y: vec![0; ys],
            u: vec![128; cs],
            v: vec![128; cs],
            background: o.background,
        })
    }

    fn drain_packets(&mut self) -> Result<(), String> {
        let fps = self.fps as f64;
        loop {
            match self.ctx.receive_packet() {
                Ok(p) => {
                    let ts_ms = (p.input_frameno as f64 * 1000.0 / fps).round() as u64;
                    self.blocks.push(Block { ts_ms, key: p.frame_type == FrameType::KEY, data: strip_temporal_delimiter(&p.data).to_vec() });
                }
                Err(EncoderStatus::Encoded) => continue,
                Err(EncoderStatus::NeedMoreData) | Err(EncoderStatus::LimitReached) => return Ok(()),
                Err(e) => return Err(format!("AV1 encoder: {e}")),
            }
        }
    }

    pub fn push(&mut self, f: FrameRgba) -> Result<(), String> {
        if (f.width, f.height) != (self.w, self.h) {
            return Err("all frames must have the same size".into());
        }
        let (wu, hu) = (self.w as usize, self.h as usize);
        let cw = wu.div_ceil(2);
        rgba_to_yuv420_into(&f.rgba, wu, hu, self.background, &mut self.y, &mut self.u, &mut self.v);
        let mut frame = self.ctx.new_frame();
        frame.planes[0].copy_from_raw_u8(&self.y, wu, 1);
        frame.planes[1].copy_from_raw_u8(&self.u, cw, 1);
        frame.planes[2].copy_from_raw_u8(&self.v, cw, 1);
        self.ctx.send_frame(frame).map_err(|e| format!("AV1 encoder: {e}"))?;
        self.sent += 1;
        self.drain_packets()
    }

    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        self.ctx.flush();
        self.drain_packets()?;
        if self.blocks.len() as u32 != self.sent || self.blocks.is_empty() {
            return Err(format!("AV1 encoder produced {} packets for {} frames", self.blocks.len(), self.sent));
        }
        self.blocks.sort_by_key(|b| b.ts_ms);
        Ok(mux(self.w, self.h, self.fps, &self.av1c, &self.blocks))
    }
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
    let (w, h) = if let Some(f) = frames.first() { (f.width, f.height) } else { return Err("no frames to encode".into()) };
    let mut enc = Mp4Encoder::new(w, h, o, background)?;
    for f in frames {
        enc.push(f.clone())?;
    }
    enc.finish()
}

/// Streaming H.264-in-MP4 encoder: each frame is converted to YUV and encoded as it arrives, so
/// the renderer's frames never have to be collected into one giant `Vec` before encoding.
pub struct Mp4Encoder {
    enc: H264Encoder,
    writer: Mp4Writer<std::io::Cursor<Vec<u8>>>,
    track: usize,
    step: u32,
    last_pts: i64,
    sent: usize,
    w: u32,
    h: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    background: [u8; 3],
}

impl Mp4Encoder {
    pub fn new(w: u32, h: u32, o: &Mp4Options, background: [u8; 3]) -> Result<Self, String> {
        let fps = o.fps.clamp(1, 120);
        if w == 0 || h == 0 || w % 2 == 1 || h % 2 == 1 {
            return Err("MP4 needs an even, non-zero picture size; lower the scale".into());
        }
        let mut cfg = H264Config::new(w, h, fps, 1);
        cfg.format = PacketFormat::LengthPrefixed; // MP4 samples
        // Export is a batch job, not a live stream: the Speed preset (smaller motion search, no
        // sub-pixel refinement, no 8x8 partitions) encodes far faster; CRF still sets quality.
        cfg.preset = craft_video::h264::Preset::Speed;
        // No B-frames and no scene-cut search: both add latency for a timeline whose frames are
        // already in presentation order.
        cfg.bframes = 0;
        cfg.scenecut = false;
        cfg.rate = RateControl::Crf(o.crf);
        let enc = H264Encoder::new(cfg).map_err(|e| format!("H.264 encoder: {e}"))?;
        let avcc = enc.avcc();
        let avc = AvcConfig::parse(&avcc).map_err(|e| format!("H.264 avcC: {e}"))?;

        let timescale = 90_000u32;
        let step = (timescale / fps).max(1);
        let mut writer = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).map_err(|e| format!("MP4 writer: {e}"))?;
        let track = writer
            .add_track(TrackConfig::new(SampleEntry::avc(avc, w as u16, h as u16), timescale))
            .map_err(|e| format!("MP4 track: {e}"))?;
        let (ys, cs) = yuv_sizes(w as usize, h as usize);
        Ok(Self {
            enc,
            writer,
            track,
            step,
            last_pts: 0,
            sent: 0,
            w,
            h,
            y: vec![0; ys],
            u: vec![128; cs],
            v: vec![128; cs],
            background,
        })
    }

    fn write_packet(&mut self, p: &craft_video::h264::Packet) -> Result<(), String> {
        self.writer
            .write_sample(self.track, WriteSample { data: &p.data, duration: self.step, composition_offset: (p.pts - p.dts) as i32, is_sync: p.keyframe })
            .map_err(|e| format!("MP4 sample: {e}"))
    }

    pub fn push(&mut self, f: FrameRgba) -> Result<(), String> {
        if (f.width, f.height) != (self.w, self.h) {
            return Err("all frames must have the same size".into());
        }
        rgba_to_yuv420_into(&f.rgba, self.w as usize, self.h as usize, self.background, &mut self.y, &mut self.u, &mut self.v);
        let cw = (self.w as usize).div_ceil(2);
        // pts is the presentation timestamp and must advance by a constant step per picture
        // (a packet may carry several NALs; they all belong to this one picture).
        let packets = {
            let pic = YuvFrame { y: &self.y, u: &self.u, v: &self.v, y_stride: self.w as usize, uv_stride: cw };
            self.enc.encode(&pic, self.last_pts).map_err(|e| format!("H.264 encode: {e}"))?
        };
        for p in &packets {
            self.write_packet(p)?;
        }
        self.last_pts += 1;
        self.sent += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        let packets = self.enc.flush();
        for p in &packets {
            self.write_packet(p)?;
        }
        if self.sent == 0 {
            return Err("H.264 encoder produced no frames".into());
        }
        Ok(self.writer.finish_faststart().map_err(|e| format!("MP4 finish: {e}"))?.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_rgba_converts_to_neutral_chroma() {
        let mut y = vec![0; 16];
        let mut u = vec![0; 4];
        let mut v = vec![0; 4];
        rgba_to_yuv420_into(&[128, 128, 128, 255].repeat(16), 4, 4, [255, 255, 255], &mut y, &mut u, &mut v);
        assert!(y.iter().all(|&c| (i32::from(c) - 126).abs() <= 1), "grey mid-tone maps near luma 126: {y:?}");
        assert!(u.iter().all(|&c| c == 128) && v.iter().all(|&c| c == 128), "neutral grey must have neutral chroma: {u:?} {v:?}");
    }

    #[test]
    fn transparent_pixels_flatten_to_the_background() {
        let mut y = vec![0; 4];
        let mut u = vec![0; 1];
        let mut v = vec![0; 1];
        rgba_to_yuv420_into(&[0, 0, 0, 0].repeat(4), 2, 2, [255, 255, 255], &mut y, &mut u, &mut v);
        assert!(y.iter().all(|&c| c == 235), "transparent over white is white (luma 235): {y:?}");
        assert_eq!(u[0], 128, "white has exactly neutral U");
        assert_eq!(v[0], 128, "white has exactly neutral V");
    }

    #[test]
    fn semi_transparent_colours_composite_with_straight_alpha() {
        let mut y = vec![0; 4];
        let mut u = vec![0; 1];
        let mut v = vec![0; 1];
        // 50% red over white must become (255, 127, 127), not the "premultiplied" (382, 127, 127)
        // which clips to pure white in luma.
        rgba_to_yuv420_into(&[255, 0, 0, 128].repeat(4), 2, 2, [255, 255, 255], &mut y, &mut u, &mut v);
        assert!(y.iter().all(|&c| (i32::from(c) - 149).abs() <= 1), "50% red over white luma ≈149: {y:?}");
        assert!(u[0] < 120 && v[0] > 170, "red chroma must survive alpha compositing: u={} v={}", u[0], v[0]);
    }

    /// Local baseline for the hardware backend probes; ignored in normal CI because it encodes
    /// 150 full-HD frames.
    #[test]
    #[ignore = "local throughput probe for the MP4 encoder"]
    fn cpu_h264_1080p_probe() {
        use std::time::Instant;

        let o = Mp4Options { fps: 30, crf: 23.0 };
        let mut enc = Mp4Encoder::new(1920, 1080, &o, [255, 255, 255]).unwrap();
        let mut rgba = vec![0u8; 1920 * 1080 * 4];
        for y in 0..1080usize {
            for x in 0..1920usize {
                let p = (y * 1920 + x) * 4;
                rgba[p] = (x % 256) as u8;
                rgba[p + 1] = (y % 256) as u8;
                rgba[p + 2] = 128;
                rgba[p + 3] = 255;
            }
        }
        let frame = FrameRgba::new(1920, 1080, rgba).unwrap();
        let start = Instant::now();
        for _ in 0..150 {
            enc.push(frame.clone()).unwrap();
        }
        let bytes = enc.finish().unwrap();
        eprintln!("[motion] CPU H.264 1080p150: {} ms, {} bytes", start.elapsed().as_millis(), bytes.len());
    }
}
