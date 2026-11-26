//! Motion animation export: render a frame (or a whole page) of a Motion timeline frame by frame
//! and encode it as an animated **GIF** or a **PNG sequence in a zip**.
//!
//! Ported from figcraft/desigma `desigma-export::anim` (PR#1624 family), adapted to this tree:
//! frames are evaluated with [`figcraft_doc::anim::motion::evaluate`] and drawn with the same CPU
//! renderer as static exports ([`figcraft_render::render_node`]), so an exported frame is
//! pixel-identical to a PNG export of that moment. Encoding uses the workspace `image` (GIF) and
//! `zip` crates. **No UI dependency**: exposed through the `motion.export` engine command so the
//! CLI, control channel, MCP and tests all share one path (docs/architecture.md "one path").
//!
//! WebM/AV1 export is intentionally not included here — it needs the `rav1e` encoder and is a
//! separate, larger step.

use base64::Engine as _;
use figcraft_doc::Document;
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::{TimelineSettings, evaluate};
use figcraft_geom::Rect;
use figcraft_render::{Pixmap, View};
use serde_json::{Value, json};

#[allow(clippy::wildcard_imports)]
use crate::params::*;
use crate::{Result, Session, bad};

/// Largest side of an exported animation in pixels.
pub const MAX_SIDE: u32 = 8192;
/// Most frames one export may contain (10 minutes at 60 fps is 36000).
pub const MAX_FRAMES: u32 = 40_000;
/// Largest GIF dimension the encoder handles comfortably.
const GIF_MAX_FPS: u32 = 50;

/// Output formats of a motion export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimFormat {
    Gif,
    /// AV1-in-WebM (pure Rust `rav1e`).
    Webm,
    /// A zip of numbered PNG files.
    PngZip,
    /// H.264/AVC in an MP4 container.
    Mp4,
}

impl AnimFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gif => "GIF",
            Self::Webm => "WebM (AV1)",
            Self::PngZip => "PNG frames",
            Self::Mp4 => "MP4 (H.264)",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Webm => "webm",
            Self::PngZip => "zip",
            Self::Mp4 => "mp4",
        }
    }
    /// `gif`, `webm`, `png`, `zip` / `png-zip` (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "gif" => Some(Self::Gif),
            "webm" | "av1" => Some(Self::Webm),
            "png" | "zip" | "png-zip" | "pngzip" => Some(Self::PngZip),
            "mp4" | "h264" => Some(Self::Mp4),
            _ => None,
        }
    }
}

/// A node (frame, layer, or a whole page) and its timeline, ready to be drawn at any time.
pub struct Animation {
    base: Document,
    node: NodeId,
    page: NodeId,
    timeline: TimelineSettings,
    /// Fixed output view: union bounds over the whole timeline, so a size/position animation can
    /// never clip a frame or change its dimensions between frames.
    view: View,
    size: (u32, u32),
}

impl Animation {
    /// Prepares the export of `node` at `scale` (1.0 = 100%). The timeline is the one stored for
    /// the **frame/page itself** when present, else its page's — so two frames on one page can be
    /// exported with independent timelines (the "same container = one animation group" rule).
    pub fn new(doc: &Document, node: NodeId, scale: f64) -> Option<Self> {
        let n = doc.node(node)?;
        let is_page = n.is_page();
        let page = if is_page { node } else { doc.page_of(node)? };
        let scale = if scale.is_finite() { scale.clamp(0.05, 16.0) } else { 1.0 };
        // Per-frame timeline first: a frame's own entry wins over the page's.
        let timeline = doc.timelines.get(&node).or_else(|| doc.timelines.get(&page)).copied().unwrap_or_default();
        let mut bounds: Option<Rect> = None;
        let samples = 24u32;
        for i in 0..=samples {
            let t = timeline.duration_ms as f64 * i as f64 / samples as f64;
            let evaluated = evaluate(doc, page, t);
            if let Some(b) = figcraft_render::export_bounds(&evaluated, node) {
                bounds = Some(match bounds {
                    None => b,
                    Some(prev) => prev.union(b),
                });
            }
        }
        let b = bounds?;
        let w = (b.width() * scale).ceil().max(1.0);
        let h = (b.height() * scale).ceil().max(1.0);
        if w > MAX_SIDE as f64 || h > MAX_SIDE as f64 {
            return None;
        }
        let view = View { origin: (b.x0, b.y0), scale, width: w as u32, height: h as u32 };
        Some(Self { base: doc.clone(), node, page, timeline, view, size: (w as u32, h as u32) })
    }

    pub fn timeline(&self) -> TimelineSettings {
        self.timeline
    }
    /// Output size in pixels (constant over the animation).
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    /// H.264 (MP4) and AV1 (WebM) encode 4:2:0 and reject odd picture sizes. Add at most one
    /// transparent row/column instead of failing an otherwise valid animation whose union bounds
    /// landed on an odd pixel (a 110×116 layer can produce 111×116 after sub-pixel movement).
    fn pad_even(mut self) -> Self {
        let w = self.view.width + self.view.width % 2;
        let h = self.view.height + self.view.height % 2;
        self.view.width = w;
        self.view.height = h;
        self.size = (w, h);
        self
    }
    /// Whether anything on the page is animated.
    pub fn is_animated(&self) -> bool {
        figcraft_doc::anim::motion::animated_nodes(&self.base, self.page)
            .into_iter()
            .any(|id| self.base.node(id).is_some_and(|n| !n.motion.is_empty()))
    }

    /// Timeline times of the frames of an export at `fps`. A looping timeline gets the frames
    /// `0 .. duration` (the last leads back into the first); a non-looping one adds a final frame
    /// at the end so the animation ends on its last state.
    pub fn frame_times(&self, fps: u32) -> Vec<f64> {
        frame_times(&self.timeline, fps)
    }

    /// Draws the animation at `t_ms` on a transparent background as a premultiplied Pixmap of a
    /// constant size. Motion is baked into a scratch document first, then drawn through a fixed
    /// view (union bounds over the timeline), so position/size animation never clips or resizes a
    /// frame — the same guarantee desigma's `Animation::render` gives.
    pub fn render(&self, t_ms: f64) -> Option<Pixmap> {
        let evaluated = evaluate(&self.base, self.page, t_ms);
        let mut out = Pixmap::new(self.view.width, self.view.height)?;
        figcraft_render::render_nodes_into(&evaluated, &[self.node], &self.view, &mut out);
        Some(out)
    }
}

/// See [`Animation::frame_times`].
pub fn frame_times(timeline: &TimelineSettings, fps: u32) -> Vec<f64> {
    let fps = fps.clamp(1, TimelineSettings::MAX_FPS);
    let d = timeline.duration_ms.max(1) as f64;
    let step = 1000.0 / fps as f64;
    let n = ((d / step).round() as u32).clamp(1, MAX_FRAMES);
    let mut out: Vec<f64> = (0..n).map(|i| (i as f64 * step).min(d)).collect();
    if !timeline.looping && out.last().is_none_or(|l| *l < d - 0.5) {
        out.push(d);
    }
    out
}

/// The encoded bytes of one Motion export and the metadata a save dialog / test needs.
pub struct AnimationOutput {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    pub frames: usize,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub looping: bool,
}

/// The scale that gives a node's animation `size` (`2x` / `512w` / `512h`). The animation's
/// fixed union bounds are the reference size, exactly like the still export's own bounds.
pub fn scale_for(doc: &Document, node: NodeId, size: figcraft_doc::ExportSize) -> f64 {
    match Animation::new(doc, node, 1.0) {
        Some(a) => size.scale_for(a.size().0 as f64, a.size().1 as f64),
        None => 1.0,
    }
}

/// Encode one Motion export from a plain document. This is the single encoder used by
/// `motion.export` and by a normal Export row whose format is GIF / WebM / PNG frames / MP4.
pub fn encode_animation(doc: &Document, node: NodeId, format: AnimFormat, scale: f64, fps: u32, quality: u8, speed: u8) -> Result<AnimationOutput> {
    let cmd = "motion.export";
    let anim = Animation::new(doc, node, scale).ok_or_else(|| bad(cmd, "nothing to export (no node, no bounds, or too large)"))?;
    // MP4/H.264 and WebM/AV1 encode 4:2:0 and require even picture sizes. GIF and PNG frames keep
    // their exact bounds; video formats get a single transparent edge pixel when needed.
    let anim = if matches!(format, AnimFormat::Mp4 | AnimFormat::Webm) { anim.pad_even() } else { anim };
    if !anim.is_animated() {
        return Err(bad(cmd, "the layer has no Motion animation to export"));
    }
    // GIF falls back to a sane frame rate so its palette stays legible; PNG zip / WebM keep
    // the requested (or timeline) rate, with WebM allowing up to its own 120 fps cap.
    let fps = if fps == 0 {
        anim.timeline().fps
    } else if format == AnimFormat::Gif {
        fps.min(GIF_MAX_FPS)
    } else {
        fps.min(120)
    };
    let times = anim.frame_times(fps);
    if times.len() as u32 > MAX_FRAMES {
        return Err(bad(cmd, format!("too many frames ({} > {MAX_FRAMES}); lower fps or duration", times.len())));
    }
    let size = anim.size();
    // WebM is streamed: `encode_webm` renders each frame itself. Rendering them up front first
    // would double the (already slow) AV1 work, so only Gif/PngZip pre-build a frame Vec.
    let (bytes, ext) = match format {
        AnimFormat::Mp4 => {
            let mut rendered = Vec::with_capacity(times.len());
            for t in &times {
                let pm = anim.render(*t).ok_or_else(|| bad(cmd, "a frame failed to render"))?;
                rendered
                    .push(figcraft_motion_codec::FrameRgba::new(pm.width(), pm.height(), figcraft_render::to_rgba8(&pm)).map_err(|e| bad(cmd, e))?);
            }
            let o = figcraft_motion_codec::Mp4Options { fps, crf: (100.0 - quality as f64 / 4.0) as f32 };
            let bytes = figcraft_motion_codec::encode_mp4(&rendered, &o, [255, 255, 255]).map_err(|e| bad(cmd, e))?;
            (bytes, "mp4")
        }
        AnimFormat::Webm => {
            // WebM is streamed through the codec crate. Frames are fed by a closure that draws
            // on demand, so the (slow) AV1 work is not doubled by pre-building every Pixmap.
            let mut rendered = Vec::with_capacity(times.len());
            for t in &times {
                let pm = anim.render(*t).ok_or_else(|| bad(cmd, "a frame failed to render"))?;
                rendered
                    .push(figcraft_motion_codec::FrameRgba::new(pm.width(), pm.height(), figcraft_render::to_rgba8(&pm)).map_err(|e| bad(cmd, e))?);
            }
            let o = figcraft_motion_codec::WebmOptions { fps, quality, speed, ..Default::default() };
            (figcraft_motion_codec::encode_webm(&rendered, &o).map_err(|e| bad(cmd, e))?, "webm")
        }
        _ => {
            let mut frames = Vec::with_capacity(times.len());
            for t in &times {
                let pm = anim.render(*t).ok_or_else(|| bad(cmd, "a frame failed to render"))?;
                frames.push(figcraft_motion_codec::FrameRgba::new(pm.width(), pm.height(), figcraft_render::to_rgba8(&pm)).map_err(|e| bad(cmd, e))?);
            }
            match format {
                AnimFormat::Gif => {
                    let step = (1000.0 / fps as f64).round().max(1.0) as u32;
                    (figcraft_motion_codec::encode_gif(&frames, &vec![step; frames.len()], anim.timeline().looping).map_err(|e| bad(cmd, e))?, "gif")
                }
                AnimFormat::PngZip => {
                    let name = doc.node(node).map(|n| n.name.clone()).unwrap_or_else(|| "animation".into());
                    (figcraft_motion_codec::write_png_zip(&frames, &name).map_err(|e| bad(cmd, e))?, "zip")
                }
                AnimFormat::Webm | AnimFormat::Mp4 => return Err(bad(cmd, "handled above")),
            }
        }
    };
    Ok(AnimationOutput { bytes, ext, frames: times.len(), fps, width: size.0, height: size.1, looping: anim.timeline().looping })
}

/// The `motion.export` command: no document write, no undo — it only reads and encodes.
pub fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "motion.export";
    let ids = req_targets(cmd, s, p)?;
    let node = ids[0];
    let format = match string(p, "format").as_deref() {
        None | Some("gif") => AnimFormat::Gif,
        Some(f) => AnimFormat::parse(f).ok_or_else(|| bad(cmd, "format: gif|webm|png|zip|png-zip"))?,
    };
    let scale = num(p, "scale").unwrap_or(1.0);
    let fps = num(p, "fps").unwrap_or(0.0) as u32;
    let quality = num(p, "quality").unwrap_or(85.0) as u8;
    let speed = num(p, "speed").unwrap_or(9.0) as u8;
    let out = encode_animation(s.doc()?, node, format, scale, fps, quality, speed)?;
    Ok(json!({
        "format": out.ext,
        "label": format.label(),
        "data": base64::engine::general_purpose::STANDARD.encode(&out.bytes),
        "frames": out.frames,
        "fps": out.fps,
        "width": out.width,
        "height": out.height,
        "loop": out.looping,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::geom::Vec2;
    use figcraft_doc::{
        NodeKind,
        anim::motion::{AnimProperty, AnimValue, Ease},
    };

    fn animated_doc() -> (Document, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 24.0, 24.0).unwrap();
        let id = d.create(frame, NodeKind::Rectangle, "Dot", 2.0, 2.0, 8.0, 8.0).unwrap();
        {
            let n = d.node_mut(id).unwrap();
            let tr = n.motion.ensure_track(AnimProperty::Opacity);
            tr.set_key(0, AnimValue::Scalar(0.0), None);
            tr.set_key(500, AnimValue::Scalar(1.0), Some(Ease::Linear));
            let pos = n.motion.ensure_track(AnimProperty::Position);
            pos.set_key(0, AnimValue::Vec2(Vec2::new(2.0, 2.0)), None);
            pos.set_key(500, AnimValue::Vec2(Vec2::new(14.0, 2.0)), None);
        }
        d.timelines.insert(frame, TimelineSettings { duration_ms: 500, fps: 10, looping: true });
        (d, frame, id)
    }

    #[test]
    fn frame_times_loop_vs_one_shot() {
        let tl = TimelineSettings { duration_ms: 1000, fps: 10, looping: true };
        let t = frame_times(&tl, 10);
        assert_eq!(t.len(), 10, "looping: 0..duration excludes the wrap-around frame");
        assert!((t[0] - 0.0).abs() < 1e-6 && (t[9] - 900.0).abs() < 1e-6);
        let tl = TimelineSettings { duration_ms: 1000, fps: 10, looping: false };
        let t = frame_times(&tl, 10);
        assert_eq!(t.len(), 11, "one-shot adds a final frame at duration");
        assert!((t[10] - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn animation_uses_the_frames_own_timeline_and_evaluates_motion() {
        let (d, frame, _) = animated_doc();
        let anim = Animation::new(&d, frame, 1.0).unwrap();
        assert!(anim.is_animated());
        assert_eq!(anim.timeline().duration_ms, 500, "the frame's own timeline wins over the page's");
        assert_eq!(anim.size(), (24, 24));
        let first = anim.render(0.0).unwrap();
        let mid = anim.render(250.0).unwrap();
        assert_eq!(first.data().len(), 24 * 24 * 4);
        assert_ne!(figcraft_render::to_rgba8(&first), figcraft_render::to_rgba8(&mid), "the Position track must actually move between frames");
    }

    #[test]
    fn gif_and_png_zip_encode_real_frames() {
        let (d, frame, _) = animated_doc();
        let anim = Animation::new(&d, frame, 1.0).unwrap();
        let times = anim.frame_times(10);
        let frames: Vec<figcraft_motion_codec::FrameRgba> = times
            .iter()
            .map(|t| {
                let pm = anim.render(*t).unwrap();
                figcraft_motion_codec::FrameRgba::new(pm.width(), pm.height(), figcraft_render::to_rgba8(&pm)).unwrap()
            })
            .collect();
        let gif = figcraft_motion_codec::encode_gif(&frames, &vec![100; frames.len()], true).unwrap();
        assert_eq!(&gif[0..6], b"GIF89a", "a real animated GIF");
        assert!(gif.len() > 100);
        let zip = figcraft_motion_codec::write_png_zip(&frames, "Card").unwrap();
        assert_eq!(&zip[0..2], b"PK", "a real zip");
        let mut zs = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        assert_eq!(zs.len(), frames.len());
        assert!(zs.by_name("Card_0001.png").is_ok());
    }

    #[test]
    fn mp4_export_produces_an_mp4_container() {
        use serde_json::json;
        let (d, frame, _) = animated_doc();
        let mut s = Session::with_document(d);
        let out = s.execute("motion.export", &json!({"id": frame.0, "format": "mp4", "fps": 10, "scale": 0.5})).unwrap();
        assert_eq!(out["format"], "mp4");
        let data = base64::engine::general_purpose::STANDARD.decode(out["data"].as_str().unwrap()).unwrap();
        // ISO BMFF: ftyp box first, isom brand, and an avc1 sample entry somewhere in moov.
        assert_eq!(&data[4..8], b"ftyp", "an ISO BMFF file");
        assert!(data.windows(4).any(|w| w == b"isom"), "isom brand");
        assert!(data.windows(4).any(|w| w == b"avc1"), "H.264 sample entry");
        assert!(data.len() > 200, "real encoded frames, not just a header");
    }

    #[test]
    fn motion_export_command_produces_a_gif() {
        use serde_json::json;
        let (d, frame, _) = animated_doc();
        let mut s = Session::with_document(d);
        let v = s.execute("motion.export", &json!({"id": frame.0, "format": "gif", "fps": 10, "scale": 1.0})).unwrap();
        assert_eq!(v["format"], "gif");
        assert_eq!(v["loop"], true);
        assert_eq!(v["width"], 24);
        assert_eq!(v["frames"], 5, "looping 500ms at 10fps");
        let raw = base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap()).unwrap();
        assert_eq!(&raw[0..6], b"GIF89a");
    }

    #[test]
    fn motion_export_command_produces_a_webm() {
        use serde_json::json;
        let (d, frame, _) = animated_doc();
        let mut s = Session::with_document(d);
        // 24x24 (even, 4:2:0-safe), 5 looping frames; speed 10 keeps the test quick. The command
        // must stream AV1 itself (no up-front frame Vec) and return a decodable EBML/WebM + meta.
        let v = s.execute("motion.export", &json!({"id": frame.0, "format": "webm", "fps": 10, "speed": 10, "scale": 1.0})).unwrap();
        assert_eq!(v["format"], "webm");
        assert_eq!(v["label"], "WebM (AV1)");
        assert_eq!(v["width"], 24);
        assert_eq!(v["height"], 24);
        assert_eq!(v["frames"], 5, "looping 500ms at 10fps");
        assert_eq!(v["fps"], 10);
        assert_eq!(v["loop"], true);
        let raw = base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap()).unwrap();
        assert_eq!(&raw[0..4], &[0x1A, 0x45, 0xDF, 0xA3], "a real EBML/WebM file");
        assert!(raw.windows(5).any(|w| w == b"V_AV1"), "AV1 codec id");
        assert!(raw.len() > 200, "real encoded AV1 frames, not just a header");
    }

    #[test]
    fn export_refuses_unanimated_and_junk_params() {
        use serde_json::json;
        let mut d = Document::new("t");
        let page = d.first_page().unwrap();
        let id = d.create(page, NodeKind::Rectangle, "Still", 0.0, 0.0, 10.0, 10.0).unwrap();
        let mut s = Session::with_document(d);
        assert!(s.execute("motion.export", &json!({"id": id.0})).is_err(), "no animation");
        assert!(s.execute("motion.export", &json!({"id": id.0, "format": "mp4"})).is_err(), "unsupported format refused");
    }
}
