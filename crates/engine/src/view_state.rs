use super::{Deserialize, Serialize};

/// The canvas view of a document: what's at the centre of the viewport and the zoom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    /// Page coordinate at the viewport centre.
    pub cx: f64,
    pub cy: f64,
    /// Zoom (1 = 100%).
    pub zoom: f64,
    /// Viewport size in logical pixels (set by the UI; used by zoom-to-fit).
    pub vw: f64,
    pub vh: f64,
    /// Height in screen points of the strip the floating bottom toolbar covers; set by the UI
    /// every frame, 0 when hidden. Input state, never saved with the document (#2152).
    #[serde(skip)]
    pub bottom_inset: f64,
    pub pixel_grid: bool,
    pub layout_grids: bool,
    pub rulers: bool,
    pub outlines: bool,
    /// Outline mode for the selected layers only (the rest draws normally).
    pub outlines_selection: bool,
    /// Outline mode also outlines hidden layers (View ▸ Outlines ▸ Include hidden layers).
    pub outline_hidden: bool,
    /// Outline mode also draws every layer's box (View ▸ Outlines ▸ Include object bounds).
    pub outline_bounds: bool,
    /// Outline the masks (they don't draw themselves).
    pub mask_outlines: bool,
    pub pixel_preview: bool,
    /// Pixel preview resolution: rendered pixels per page pixel (1 or 2).
    pub pixel_scale: u8,
    pub show_slices: bool,
    pub comments: bool,
    /// Others' cursors in a live session (View ▸ Multiplayer cursors).
    pub multiplayer_cursors: bool,
    pub frame_outlines: bool,
    /// Soft-proof sRGB: the canvas marks the colours of a Display P3 file that sRGB can't show.
    pub proof_srgb: bool,
    pub snap_geometry: bool,
    pub snap_objects: bool,
    pub snap_pixel: bool,
    pub ui_hidden: bool,
    /// Minimized interface (Shift+Cmd/Ctrl+\): the navigation bar and both sidebars are collapsed
    /// and the toolbar stays; the right sidebar shows while something is selected.
    pub ui_minimized: bool,
    /// This view's zoom limits: a board has its own range, a design file Figma's full range.
    /// Input state, never saved with the document.
    #[serde(skip)]
    pub min_zoom: f64,
    #[serde(skip)]
    pub max_zoom: f64,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            cx: 0.0,
            cy: 0.0,
            zoom: 1.0,
            min_zoom: Self::MIN_ZOOM,
            max_zoom: Self::MAX_ZOOM,
            vw: 1200.0,
            vh: 800.0,
            bottom_inset: 0.0,
            pixel_grid: true,
            layout_grids: true,
            rulers: false,
            outlines: false,
            outlines_selection: false,
            outline_hidden: false,
            outline_bounds: false,
            mask_outlines: false,
            pixel_preview: false,
            pixel_scale: 1,
            show_slices: false,
            comments: true,
            multiplayer_cursors: true,
            frame_outlines: false,
            proof_srgb: false,
            snap_geometry: true,
            snap_objects: true,
            snap_pixel: true,
            ui_hidden: false,
            ui_minimized: false,
        }
    }
}

impl ViewState {
    /// Figma's design canvas camera clamps zoom to 1/64 … 256× (1.5625% … 25600%).
    pub const MIN_ZOOM: f64 = 0.015625;
    pub const MAX_ZOOM: f64 = 256.0;
    /// FigJam's whiteboard stops much earlier: 2% … 800%.
    pub const BOARD_MIN_ZOOM: f64 = 0.02;
    pub const BOARD_MAX_ZOOM: f64 = 8.0;
    /// The default view for a document kind: boards use FigJam's 2%–800%, everything else Figma's
    /// design range.
    pub fn for_kind(kind: figcraft_doc::FileKind) -> Self {
        let mut v = Self::default();
        if kind == figcraft_doc::FileKind::Board {
            v.min_zoom = Self::BOARD_MIN_ZOOM;
            v.max_zoom = Self::BOARD_MAX_ZOOM;
        }
        v
    }
    /// Page coordinate at the viewport's top-left.
    pub fn origin(&self) -> (f64, f64) {
        let z = self.zoom.max(1e-9);
        (self.cx - self.vw / 2.0 / z, self.cy - self.vh / 2.0 / z)
    }
    /// Clamp a requested zoom to this view's limits; junk (`NaN`) keeps the current zoom.
    pub fn clamp_zoom(&self, zoom: f64) -> f64 {
        if zoom.is_finite() { zoom.clamp(self.min_zoom, self.max_zoom) } else { self.zoom }
    }
    pub fn set_zoom_at(&mut self, zoom: f64, anchor: (f64, f64)) {
        let z = self.clamp_zoom(zoom);
        let old = self.zoom.max(1e-9);
        // Keep the page point under `anchor` fixed.
        self.cx = anchor.0 + (self.cx - anchor.0) * old / z;
        self.cy = anchor.1 + (self.cy - anchor.1) * old / z;
        self.zoom = z;
    }
    /// Fit a page rectangle into the viewport with margin.
    pub fn fit(&mut self, r: figcraft_geom::Rect, max_zoom: f64) {
        if !(r.width() > 0.0 || r.height() > 0.0) || !r.x0.is_finite() {
            self.cx = r.center().x;
            self.cy = r.center().y;
            return;
        }
        let margin = 64.0;
        let zx = (self.vw - margin * 2.0).max(50.0) / r.width().max(1e-6);
        let zy = (self.vh - margin * 2.0).max(50.0) / r.height().max(1e-6);
        self.zoom = zx.min(zy).clamp(self.min_zoom, max_zoom.min(self.max_zoom));
        self.cx = r.center().x;
        self.cy = r.center().y;
    }
    /// Fit a page rectangle for Zoom to selection: a 40-point margin around the area *above* the
    /// floating bottom toolbar (`vw` by `vh - bottom_inset`), centred in that area (#2152).
    pub fn fit_selection(&mut self, r: figcraft_geom::Rect, max_zoom: f64) {
        if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
            return;
        }
        let inset = if self.bottom_inset.is_finite() { self.bottom_inset.clamp(0.0, (self.vh - 1.0).max(0.0)) } else { 0.0 };
        let area_h = (self.vh - inset).max(1.0);
        if !(r.width() > 0.0 || r.height() > 0.0) {
            self.cx = r.center().x;
            self.cy = r.center().y + (self.vh - area_h) / 2.0 / self.zoom.max(1e-6);
            return;
        }
        let mx = (self.vw / 4.0).clamp(0.0, 40.0);
        let my = (area_h / 4.0).clamp(0.0, 40.0);
        let zx = (self.vw - mx * 2.0).max(1.0) / r.width().max(1e-6);
        let zy = (area_h - my * 2.0).max(1.0) / r.height().max(1e-6);
        self.zoom = zx.min(zy).clamp(self.min_zoom, max_zoom.min(self.max_zoom));
        self.cx = r.center().x;
        self.cy = r.center().y + (self.vh - area_h) / 2.0 / self.zoom;
    }

    /// The zoom from which the pixel grid shows (508%, and it shows **at** 508% too).
    pub const PIXEL_GRID_ZOOM: f64 = 5.08;
    /// Whether the pixel grid draws: it's on and the canvas shows 508% or more. The test rounds to
    /// the percentage the UI prints, so "508%" on the label always means the grid is there (a raw
    /// `>= 5.08` can miss by a float ULP at exactly 508%).
    pub fn pixel_grid_shown(&self) -> bool {
        self.pixel_grid && (self.zoom * 100.0).round() >= Self::PIXEL_GRID_ZOOM * 100.0
    }
    /// Pixel preview's resolution (rendered pixels per page pixel) when it's on and zoomed in past
    /// it; below that the canvas is already coarser than the preview.
    pub fn pixel_preview_scale(&self) -> Option<f64> {
        let k = f64::from(self.pixel_scale.clamp(1, 2));
        (self.pixel_preview && self.zoom > k).then_some(k)
    }
    /// A wheel or trackpad scroll of (`dx`, `dy`) screen points: the canvas pans with it (content
    /// follows the fingers); with Shift a plain vertical wheel pans sideways.
    pub fn scroll(&mut self, dx: f64, dy: f64, shift: bool) {
        let (dx, dy) = if shift && dx == 0.0 { (dy, 0.0) } else { (dx, dy) };
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let z = self.zoom.max(1e-6);
        self.cx -= dx / z;
        self.cy -= dy / z;
    }
    /// How near the viewport's edge a drag must hold the pointer to pan the canvas (screen px).
    pub const AUTOPAN_ZONE_PX: f64 = 15.0;
    /// Auto-pan speed gained for each pixel the pointer is closer to the edge (screen px/s).
    pub const AUTOPAN_RATE: f64 = 40.0;
    /// The fastest auto-pan, at the edge and beyond it (screen px/s).
    pub const AUTOPAN_MAX: f64 = 600.0;
    /// The speed (screen px/s) the canvas pans toward an edge while a drag holds the pointer
    /// `d` px inside it (negative: outside), as measured in the incumbent: `40 x (15 - d)`
    /// clamped to 0..600 (#2015). Constant while the pointer stays put, at every zoom.
    pub fn autopan_speed(d: f64) -> f64 {
        if d.is_nan() {
            return 0.0;
        }
        (Self::AUTOPAN_RATE * (Self::AUTOPAN_ZONE_PX - d)).clamp(0.0, Self::AUTOPAN_MAX)
    }
    /// The velocity (screen px/s, positive: right and down) the canvas auto-pans at while a drag
    /// holds the pointer at (`px`, `py`) in a viewport of `width` x `height` (both from its
    /// top-left corner): toward each edge the pointer is near, each axis by its own distance, so a
    /// corner pans both ways.
    pub fn autopan_velocity(px: f64, py: f64, width: f64, height: f64) -> (f64, f64) {
        let axis = |p: f64, len: f64| {
            if p < len / 2.0 { -Self::autopan_speed(p) } else { Self::autopan_speed(len - p) }
        };
        (axis(px, width), axis(py, height))
    }
    /// Auto-pan for `dt` seconds with the pointer at (`px`, `py`) from the viewport's top-left
    /// corner: moves the view by the velocity ([`Self::autopan_velocity`]) in screen px. True when
    /// it moved.
    pub fn autopan(&mut self, px: f64, py: f64, dt: f64) -> bool {
        let (vx, vy) = Self::autopan_velocity(px, py, self.vw, self.vh);
        if !(dt.is_finite() && dt > 0.0) || (vx == 0.0 && vy == 0.0) {
            return false;
        }
        let z = self.zoom.max(1e-6);
        self.cx += vx * dt / z;
        self.cy += vy * dt / z;
        true
    }
    pub fn page_to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        let (ox, oy) = self.origin();
        ((x - ox) * self.zoom, (y - oy) * self.zoom)
    }
    pub fn screen_to_page(&self, x: f64, y: f64) -> (f64, f64) {
        let (ox, oy) = self.origin();
        let z = self.zoom.max(1e-9);
        (ox + x / z, oy + y / z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zoom to selection fits the area *above* a floating bottom toolbar, so the selected thing
    /// lands centred in what the user can actually see (#2152).
    #[test]
    fn fit_selection_accounts_for_the_bottom_toolbar() {
        let rect = figcraft_geom::Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut plain = ViewState { vw: 1000.0, vh: 800.0, ..Default::default() };
        plain.fit_selection(rect, 256.0);
        let mut inset = ViewState { vw: 1000.0, vh: 800.0, bottom_inset: 100.0, ..Default::default() };
        inset.fit_selection(rect, 256.0);
        assert!(inset.zoom < plain.zoom, "less usable height means a smaller zoom: {} vs {}", inset.zoom, plain.zoom);
        assert!(inset.cy > plain.cy, "the selection sits lower on the page to clear the toolbar: {} vs {}", inset.cy, plain.cy);
        assert!((inset.cy - (50.0 + 50.0 / inset.zoom)).abs() < 1e-9, "its centre lands at the middle of the area above the toolbar");
    }
}
