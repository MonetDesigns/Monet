//! 摆放：被演示的画框 / 设备机身 / 覆盖层在舞台里的位置 —— `ui-egui/src/present/layout.rs`
//! 的 GPUI 移植。纯函数，不改状态；输入与渲染共用同一份几何。

use figcraft_doc::Document;
use figcraft_doc::NodeId;
use figcraft_doc::proto::{Device, DeviceColor, OverlayAnchor, OverlayPosition};
use figcraft_engine::devices::FrameInfo;
use figcraft_geom::Rect;

use super::scroll::{clamp_scroll, page_settings};
use super::{OverlayState, PresentState, ScaleMode};

/// 舞台里的一个矩形（窗口坐标，px）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectPx {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectPx {
    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= f64::from(self.x) && p.0 <= f64::from(self.x + self.w) && p.1 >= f64::from(self.y) && p.1 <= f64::from(self.y + self.h)
    }
}

/// page ↔ 屏幕映射。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub page_min: (f64, f64),
    pub origin: (f64, f64),
    pub scale: f64,
}

impl Place {
    pub fn pt(&self, x: f64, y: f64) -> (f32, f32) {
        ((self.origin.0 + (x - self.page_min.0) * self.scale) as f32, (self.origin.1 + (y - self.page_min.1) * self.scale) as f32)
    }

    pub fn rect(&self, r: Rect) -> RectPx {
        let (x0, y0) = self.pt(r.x0, r.y0);
        let (x1, y1) = self.pt(r.x1, r.y1);
        RectPx { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }

    /// 屏幕点 `q` 下的页坐标。
    pub fn page(&self, q: (f64, f64)) -> (f64, f64) {
        (self.page_min.0 + (q.0 - self.origin.0) / self.scale.max(1e-9), self.page_min.1 + (q.1 - self.origin.1) / self.scale.max(1e-9))
    }
}

/// 一次摆放的结果：与 eGUI 的 `Layout` 同义（去掉绘制资源部分）。
#[derive(Clone, Debug)]
pub struct Geom {
    /// 舞台本身（窗口坐标）：顶栏以下、侧栏以外的区域。
    pub area: RectPx,
    pub screen: RectPx,
    pub clip: RectPx,
    pub place: Place,
    pub device: Option<Device>,
    /// The external bezel PNG and its screen rectangle, when from the catalogue.
    pub frame: Option<FrameInfo>,
    pub landscape: bool,
    pub color: Option<DeviceColor>,
    pub scale: f64,
    pub overlays: Vec<(NodeId, Place, RectPx)>,
    pub closing: Vec<(NodeId, Place, RectPx)>,
    pub pane: bool,
    /// 整窗演示的平移范围（Fit 之外内容超出可用区时 > 0）。
    pub pan_range: (f64, f64),
}

impl Geom {
    /// 活动层：最上层覆盖层，否则被演示的画框；写作 (root, place, clip)。
    pub fn active(&self, current: NodeId) -> (NodeId, Place, RectPx) {
        match self.overlays.last() {
            Some((id, pl, _)) => (*id, *pl, self.screen),
            None => (current, self.place, self.clip),
        }
    }
}

/// 设备屏幕“立着”时的宽（横屏取短边）。
pub fn upright_w(w: f32, h: f32, landscape: bool) -> f32 {
    if landscape { h } else { w }
}

/// 屏幕四周的机身宽 + 屏幕圆角。
pub fn bezel(device: &str, w: f32) -> (f32, f32) {
    match device {
        "MacBook Air" => ((w * 0.025).max(6.0), 0.0),
        "Desktop" => ((w * 0.02).max(6.0), 0.0),
        d if d.starts_with("iPad") => ((w * 0.05).max(8.0), w * 0.04),
        _ => ((w * 0.06).max(6.0), w * 0.13),
    }
}

/// 设备颜色带宽度（eGUI `device::rim`）。
pub fn rim(w: f32, b: f32) -> f32 {
    (w * 0.016).clamp(1.5, (b * 0.5).max(1.5))
}

/// 机身比屏幕多出多少：(左右各一, 上, 下)。
pub fn body_outset(device: &str, w: f32, h: f32, landscape: bool) -> (f32, f32, f32) {
    let uw = upright_w(w, h, landscape);
    let (b, _) = bezel(device, uw);
    match device {
        "MacBook Air" => (b + uw * 0.08, b, b + (uw * 0.022).max(6.0)),
        "Desktop" => (b, b, b + uw * 0.08 + 6.0),
        _ if landscape => (b, b + 3.0, b + 3.0),
        _ => (b + 3.0, b, b),
    }
}

/// 覆盖层相对屏幕的矩形（eGUI `transition::overlay_rect`）。
fn overlay_rect(pos: OverlayPosition, base: RectPx, size: (f32, f32), anchor: Option<RectPx>, offset: Option<(f32, f32)>) -> RectPx {
    let (w, h) = size;
    let (bx, by) = (base.x, base.y);
    let (cx, cy) = (bx + (base.w - w) / 2.0, by + (base.h - h) / 2.0);
    let (x, y) = match pos {
        OverlayPosition::Center => (cx, cy),
        OverlayPosition::TopLeft => (bx, by),
        OverlayPosition::TopCenter => (cx, by),
        OverlayPosition::TopRight => (bx + base.w - w, by),
        OverlayPosition::BottomLeft => (bx, by + base.h - h),
        OverlayPosition::BottomCenter => (cx, by + base.h - h),
        OverlayPosition::BottomRight => (bx + base.w - w, by + base.h - h),
        OverlayPosition::Manual => match (anchor, offset) {
            (Some(a), Some(o)) => (a.x + o.0, a.y + o.1),
            (Some(a), None) => (a.x, a.y + a.h),
            (None, _) => (cx, cy),
        },
    };
    RectPx { x, y, w, h }
}

/// 覆盖层锚定：`anchor_to` 优先把覆盖层贴到触发层，之外才按屏幕位置（eGUI
/// `anchored_overlay_rect`）。
fn anchored_overlay_rect(
    pos: OverlayPosition,
    anchor_to: OverlayAnchor,
    base: RectPx,
    size: (f32, f32),
    anchor: Option<RectPx>,
    offset: Option<(f32, f32)>,
) -> RectPx {
    let on_trigger = anchor.and_then(|a| {
        anchor_to.offset(pos, [f64::from(a.w), f64::from(a.h)], [f64::from(size.0), f64::from(size.1)]).map(|[x, y]| (x as f32, y as f32))
    });
    match on_trigger {
        Some(o) => overlay_rect(OverlayPosition::Manual, base, size, anchor, Some(o)),
        None => overlay_rect(pos, base, size, anchor, offset),
    }
}

/// 在窗口区域 `area`（窗口坐标）里摆出当前帧 / 设备。`pane`：预览窗格（恒 Fit、恒画机身、
/// 留白 12），否则整窗演示（留白 32、按 `st.scale`）。
pub fn compute(st: &PresentState, disp: &Document, area: RectPx, pane: bool, ppp: f32) -> Option<Geom> {
    let settings = page_settings(disp, st.current);
    let device = figcraft_engine::devices::presented_device(&settings);
    let landscape_eff = settings.landscape();
    let color_name = figcraft_engine::devices::device_color(&settings).map(|c| c.name).unwrap_or_default();
    let mut frame = figcraft_engine::devices::frame_info(&settings, landscape_eff, &color_name);
    let landscape = settings.landscape();
    let fills = settings.responsive && device.is_none();
    let fb = disp.world_bounds(st.current)?;
    let show_body = pane || st.device_frame;
    // 有外置 frame PNG 时它自带颜色；没有时退回内置几何机身（要求 colours 非空）。
    let body = device.as_ref().filter(|d| show_body && (frame.is_some() || !d.colors.is_empty())).cloned();
    let scale_mode = if pane { ScaleMode::Fit } else { st.scale };
    let pad = if !fills && matches!(scale_mode, ScaleMode::Fit | ScaleMode::Actual) { if pane { super::PANE_PAD } else { super::PAD } } else { 0.0 };
    let avail_w = (area.w - 2.0 * pad).max(1.0);
    let avail_h = (area.h - 2.0 * pad).max(1.0);
    let (sw, sh) = device.as_ref().map_or((fb.width() as f32, fb.height() as f32), |d| (d.width as f32, d.height as f32));
    let outer = |scale: f64| -> (f32, f32, f32, f32, f32) {
        let (w, h) = ((f64::from(sw) * scale) as f32, (f64::from(sh) * scale) as f32);
        if let Some(fi) = frame.as_ref() {
            // 屏幕 w×h 对应 PNG 里的 screen_rect[2]×[3] 像素；整张 PNG 按比例放大。
            let k = w / fi.screen_rect[2].max(1.0) as f32;
            let png_w = (fi.png_size[0] as f32 * k).max(w);
            let png_h = (fi.png_size[1] as f32 * k).max(h);
            let left = fi.screen_rect[0] as f32 * k;
            let top = fi.screen_rect[1] as f32 * k;
            (left, top, (png_h - top - h).max(0.0), png_w, png_h)
        } else if let Some(d) = body.as_ref() {
            let (side, top, bottom) = body_outset(&d.name, w, h, landscape);
            (side, top, bottom, w + 2.0 * side, h + top + bottom)
        } else {
            (0.0, 0.0, 0.0, w, h)
        }
    };
    let factor = |ow: f32, oh: f32| -> f64 {
        let (kx, ky) = (f64::from(avail_w / ow.max(1.0)), f64::from(avail_h / oh.max(1.0)));
        match scale_mode {
            ScaleMode::Fit => kx.min(ky),
            ScaleMode::FitWidth => kx,
            ScaleMode::Fill => kx.max(ky),
            ScaleMode::Actual => 1.0,
        }
    };
    let mut scale = if fills { 1.0 } else { factor(sw, sh) };
    if body.is_some() {
        let o = outer(scale);
        scale *= factor(o.3, o.4);
    }
    let max = if scale_mode == ScaleMode::Fit { 1.0 } else { 16.0 };
    let scale = if scale.is_finite() { scale.clamp(0.01, max) } else { 0.01 };
    let (side, top, _bottom, whole_w, whole_h) = outer(scale);
    let over = |d: f32| if d > 0.5 { d } else { 0.0 };
    let excess = (over(whole_w - avail_w), over(whole_h - avail_h));
    let pan_range = (f64::from(excess.0), f64::from(excess.1));
    // 拖拽 / 惯性进行中允许越界拉伸；其它时候夹在范围内（eGUI layout()）。
    let live = matches!(st.drag_scroll, Some((super::Scroller::View, _))) || st.fling.is_some_and(|f| f.target == super::Scroller::View);
    let pan = if live { st.offset(super::Scroller::View) } else { clamp_scroll(st.offset(super::Scroller::View), pan_range) };
    let along = |excess: f32, min: f32, centre: f32, len: f32, pan: f64| if excess > 0.0 { min - pan as f32 } else { centre - len / 2.0 };
    let (ax, ay) =
        (along(excess.0, area.x + pad, area.x + area.w / 2.0, whole_w, pan.0), along(excess.1, area.y + pad, area.y + area.h / 2.0, whole_h, pan.1));
    // 对齐到设备像素：否则机身 / 玻璃 / 位图落在半像素上，缩放的每一帧都会在边缘抖一下。
    let ppp = ppp.max(0.25);
    let snap = |v: f32| (v * ppp).round() / ppp;
    let screen = RectPx {
        x: snap(ax + side),
        y: snap(ay + top),
        w: snap((f64::from(sw) * scale) as f32).max(1.0 / ppp),
        h: snap((f64::from(sh) * scale) as f32).max(1.0 / ppp),
    };
    // 真实外框：用已 snap 的 screen 反算 PNG 的位置 / 尺寸 / 窗口圆角，保证机身与内容
    // 共用一套对齐后的几何，消除半像素缝隙。
    if let Some(fi) = frame.as_mut() {
        let k = f64::from(screen.w) / fi.screen_rect[2].max(1.0);
        let png_w = (snap((fi.png_size[0] * k) as f32)).max(screen.w);
        let png_h = (snap((fi.png_size[1] * k) as f32)).max(screen.h);
        let dx = snap((fi.screen_rect[0] * k) as f32);
        let dy = snap((fi.screen_rect[1] * k) as f32);
        let radius = snap((fi.radii[0] * k) as f32).min(screen.w / 2.0).min(screen.h / 2.0);
        fi.placed = Some([dx, dy, png_w, png_h, radius]);
    }
    let place = Place {
        page_min: (fb.x0, fb.y0),
        origin: (
            f64::from(screen.x + (screen.w - (fb.width() as f32 * scale as f32)) / 2.0),
            f64::from(screen.y + (screen.h - (fb.height() as f32 * scale as f32)) / 2.0),
        ),
        scale,
    };
    let place_overlay = |o: &OverlayState| -> Option<(NodeId, Place, RectPx)> {
        let ob = disp.world_bounds(o.id)?;
        let size = ((ob.width() * scale) as f32, (ob.height() * scale) as f32);
        let offset = o.offset.map(|[x, y]| ((x * scale) as f32, (y * scale) as f32));
        let anchor = o.anchor.map(|a| place.rect(a));
        let r = anchored_overlay_rect(o.position, o.anchor_to, screen, size, anchor, offset);
        Some((o.id, Place { page_min: (ob.x0, ob.y0), origin: (f64::from(r.x), f64::from(r.y)), scale }, r))
    };
    let overlays = st.overlays.iter().filter_map(place_overlay).collect();
    let closing = st.closing.iter().filter_map(|(_, o)| place_overlay(o)).collect();
    let clip = if device.is_some() { screen } else { area };
    let color = body.as_ref().and(figcraft_engine::devices::device_color(&settings));
    Some(Geom { area, screen, clip, place, device, frame, landscape, color, scale, overlays, closing, pane, pan_range })
}
