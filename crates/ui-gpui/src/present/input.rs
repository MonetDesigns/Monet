//! 指针、滚轮、键盘、定时器 —— `ui-egui/src/present/input.rs` 的 GPUI 移植。
//!
//! GPUI 的元素回调只负责把事件塞进本模块的队列；真正的处理在每帧 `sync_present` 里用
//! `&mut PresentState` 跑（与 eGUI 在 `player()` 里直接改 Presenter 等价）。

use std::cell::RefCell;

use figcraft_doc::proto::{Direction, KeyMods, Trigger};
use figcraft_doc::{Document, NodeId};

use super::layout::{Geom, Place};
use super::scroll::{COAST_MIN, DECAY_DRAG, DECAY_WHEEL, RUBBER_PTS, clamp_scroll, rubber_band, scrollers_at, units_per_point};
use super::{Fling, PresentState, Scroller};

/// 一帧内收到的输入事件（窗口坐标）。
#[derive(Clone, Debug)]
pub enum Ev {
    Move(f64, f64),
    Down(f64, f64),
    Up(f64, f64),
    Wheel(f64, f64, f64, f64),
    Key(String, KeyMods),
}

thread_local! {
    /// 由元素回调投递，`sync_present` 每帧取走。
    pub static EVENTS: RefCell<Vec<Ev>> = const { RefCell::new(Vec::new()) };
    /// 调试：上一帧的 hotspot 矩形（窗口坐标），`present.hot` 控制命令读取。
    pub static HOT_DBG: RefCell<Vec<(NodeId, super::layout::RectPx)>> = const { RefCell::new(Vec::new()) };
    /// 调试：上一帧的指针位置与悬停层。
    pub static PTR_DBG: RefCell<(Option<(f64, f64)>, Vec<NodeId>)> = const { RefCell::new((None, Vec::new())) };
}

pub const CURSOR_DEFAULT: u8 = 0;
pub const CURSOR_POINTER: u8 = 1;
pub const CURSOR_GRAB: u8 = 2;

pub fn push(e: Ev) {
    EVENTS.with(|q| q.borrow_mut().push(e));
}

fn mods_empty(m: KeyMods) -> bool {
    !m.ctrl && !m.alt && !m.shift && !m.cmd
}

/// 演示层是否希望消费这个按键（原型触发器优先，其次播放器自己的键）。
pub fn wants_key(doc: &Document, root: NodeId, key: &str, mods: KeyMods, pane: bool) -> bool {
    let used =
        doc.subtree(root).into_iter().any(|id| doc.node(id).is_some_and(|n| n.interactions.iter().any(|it| it.trigger.fires_on_key(key, mods))));
    if used {
        return true;
    }
    if !mods_empty(mods) {
        return false;
    }
    matches!(key.to_ascii_lowercase().as_str(), "r" | "arrowleft" | "arrowright" | "space" | "n") || (key.eq_ignore_ascii_case("z") && !pane)
}

fn find_trigger(doc: &Document, id: NodeId, want: impl Fn(&Trigger) -> bool) -> Option<usize> {
    doc.node(id)?.interactions.iter().position(|it| want(&it.trigger))
}

/// 指针下最上层的可见图层（eGUI `chrome.rs::layer_at`）。
fn layer_at(doc: &Document, root: NodeId, place: &Place, q: (f64, f64)) -> Option<NodeId> {
    let mut hidden: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    let mut hit = None;
    for id in doc.subtree(root) {
        let Some(n) = doc.node(id) else { continue };
        if !n.visible || n.parent.is_some_and(|p| hidden.contains(&p)) {
            hidden.insert(id);
            continue;
        }
        if let Some(b) = doc.world_bounds(id) {
            let r = place.rect(b);
            if q.0 >= f64::from(r.x) && q.0 <= f64::from(r.x + r.w) && q.1 >= f64::from(r.y) && q.1 <= f64::from(r.y + r.h) {
                hit = Some(id);
            }
        }
    }
    hit
}

/// 方向向量（屏幕坐标，y 向下）。
fn direction_vec(d: Direction) -> (f64, f64) {
    match d {
        Direction::Left => (-1.0, 0.0),
        Direction::Right => (1.0, 0.0),
        Direction::Top => (0.0, -1.0),
        Direction::Bottom => (0.0, 1.0),
    }
}

/// 拖拽过程中让过渡跟随指针：进度 = 沿方向的拖拽比例。
fn follow_drag(st: &mut PresentState, pos: Option<(f64, f64)>, down: bool) {
    if let Some((from, dir, len, dur)) = st.follow {
        match pos.filter(|_| down) {
            Some(q) => {
                let along = ((q.0 - from.0) * dir.0 + (q.1 - from.1) * dir.1) / len.max(1e-6);
                let p = along.clamp(0.0, 1.0);
                st.transition_start = std::time::Instant::now() - std::time::Duration::from_secs_f64(p * dur);
            }
            None => st.follow = None,
        }
    }
}

/// 松开拖拽后按指针速度继续滑行（越界则回弹）。
fn release_drag_scroll(st: &mut PresentState, down: bool, velocity: (f64, f64), now: f64, scale: f64) {
    if !down && let Some((t, _)) = st.drag_scroll.take() {
        let Some(base) = st.design.as_ref() else { return };
        let u = units_per_point(t, scale);
        let r = st.limits(base, t);
        let along = |v: f64, r: f64| if r > 0.0 { -v * u } else { 0.0 };
        st.fling = Some(Fling { target: t, vel: (along(velocity.0, r.0), along(velocity.1, r.1)), decay: DECAY_DRAG, bounce: true, at: now });
    }
}

/// 滚轮滚动最内层仍能动的画框；都动不了时平移整窗。
fn wheel_scroll(st: &mut PresentState, base: &Document, targets: Vec<Scroller>, wheel: (f64, f64), dt: f64, scale: f64) {
    for t in targets {
        let u = units_per_point(t, scale);
        let cur = st.offset(t);
        let next = clamp_scroll((cur.0 - wheel.0 * u, cur.1 - wheel.1 * u), st.limits(base, t));
        if (next.0 - cur.0).abs() > 1e-6 || (next.1 - cur.1).abs() > 1e-6 {
            st.set_offset(t, next);
            st.scroll_anim = None;
            st.fling = None;
            let inst = ((next.0 - cur.0) / dt.max(1e-4), (next.1 - cur.1) / dt.max(1e-4));
            let prev = st.wheel.filter(|w| w.0 == t).map_or(inst, |w| w.1);
            let a = 1.0 - (-dt / 0.05).exp();
            st.wheel = Some((t, (prev.0 + (inst.0 - prev.0) * a, prev.1 + (inst.1 - prev.1) * a)));
            return;
        }
    }
    st.wheel = None;
}

fn begin_drag_scroll(st: &mut PresentState, disp: &Document, root: NodeId, place: Place, start: (f64, f64), d: (f64, f64)) {
    let Some(base) = st.design.clone() else { return };
    let horizontal = d.0.abs() >= d.1.abs();
    let scrolls = scrollers_at(st, &base, disp, root, place, start);
    let on_axis = |range: (f64, f64)| if horizontal { range.0 > 0.0 } else { range.1 > 0.0 };
    let mobile = super::scroll::page_settings(&base, root).device().is_some_and(|d| d.rotates);
    let target = scrolls
        .iter()
        .copied()
        .find(|f| on_axis(st.range(&base, *f)))
        .or_else(|| scrolls.first().copied())
        .map(Scroller::Frame)
        .or((!mobile && st.pan_range != (0.0, 0.0)).then_some(Scroller::View));
    if let Some(t) = target {
        st.fling = None;
        st.drag_scroll = Some((t, clamp_scroll(st.offset(t), st.limits(&base, t))));
    }
}

fn drag_scrolls(st: &mut PresentState, down: bool, pos: Option<(f64, f64)>, scale: f64) {
    if down
        && let (Some((t, from)), Some(start), Some(q)) = (st.drag_scroll, st.press, pos)
        && let Some(base) = st.design.as_ref()
    {
        let (d, u) = ((q.0 - start.0, q.1 - start.1), units_per_point(t, scale));
        let r = st.limits(base, t);
        let limit = RUBBER_PTS * u;
        let to = (rubber_band(from.0 - d.0 * u, r.0, limit), rubber_band(from.1 - d.1 * u, r.1, limit));
        st.set_offset(t, to);
        st.scroll_anim = None;
        st.cursor = CURSOR_GRAB;
    }
}

/// 一帧的输入处理。
pub fn tick(st: &mut PresentState, geom: Option<&Geom>, now: f64) {
    let events = EVENTS.with(|q| std::mem::take(&mut *q.borrow_mut()));
    st.clock = now;
    let Some(g) = geom else { return };

    // 收拢本帧事件。
    let mut pos = st.pointer;
    let mut pressed: Vec<(f64, f64)> = Vec::new();
    let mut released = false;
    let mut down_now = st.down;
    let mut wheel = (0.0f64, 0.0f64);
    let mut keys: Vec<(String, KeyMods)> = Vec::new();
    let mut last_move_at = st.last_move_at;
    let mut velocity = (0.0f64, 0.0f64);
    for e in events {
        match e {
            Ev::Move(x, y) => {
                let t = std::time::Instant::now();
                if let (Some(p), Some(t0)) = (pos, last_move_at) {
                    let dt = t.duration_since(t0).as_secs_f64().clamp(1e-3, 0.1);
                    velocity = ((x - p.0) / dt, (y - p.1) / dt);
                }
                pos = Some((x, y));
                last_move_at = Some(t);
            }
            Ev::Down(x, y) => {
                pos = Some((x, y));
                down_now = true;
                pressed.push((x, y));
            }
            Ev::Up(x, y) => {
                pos = Some((x, y));
                down_now = false;
                released = true;
            }
            Ev::Wheel(x, y, dx, dy) => {
                pos = Some((x, y));
                wheel = (wheel.0 + dx, wheel.1 + dy);
            }
            Ev::Key(k, m) => keys.push((k, m)),
        }
    }
    st.pointer = pos;
    st.down = down_now;
    st.last_move_at = last_move_at;
    st.inside = pos.is_some_and(|q| {
        q.0 >= f64::from(g.area.x) && q.0 <= f64::from(g.area.x + g.area.w) && q.1 >= f64::from(g.area.y) && q.1 <= f64::from(g.area.y + g.area.h)
    }) || st.press.is_some();
    st.cursor = CURSOR_DEFAULT;
    follow_drag(st, pos, down_now);
    // eGUI `input`：指针不在播放区（舞台）里时当作没有指针（拖动中除外），
    // 否则 Inspect 侧栏 / 顶栏下方的空白区也会命中热区、触发悬停交互。
    let pos = pos.filter(|_| st.inside);

    let Some(disp) = st.disp.clone() else { return };
    let (root, place, clip) = g.active(st.current);
    st.shown = Some((root, place, clip));
    // 响应式原型：被演示画框按设备屏幕 / 可用区改尺寸（eGUI `respond_to`，在 layout 里做）。
    let base = st.laid.clone().unwrap_or_else(|| disp.clone());
    let settings = super::scroll::page_settings(&base, st.current);
    let fit =
        settings.responsive.then(|| settings.device().map_or((f64::from(g.area.w.round()), f64::from(g.area.h.round())), |d| (d.width, d.height)));
    st.respond_to(fit);
    let base = st.laid.clone().unwrap_or(base);
    let scope = disp.top_level_frame(root).unwrap_or(root);
    let evaluated = figcraft_doc::anim::motion::evaluate(&disp, scope, st.time_ms);
    // hotspot（屏幕坐标）：引擎按层级从底到顶给出；裁到当前可视区，超出的点不到。
    let hot: Vec<(NodeId, super::layout::RectPx)> = figcraft_engine::proto_run::hotspots(&evaluated, root)
        .into_iter()
        .filter_map(|(id, b)| {
            let r = place.rect(b);
            let (x0, y0) = (r.x.max(clip.x), r.y.max(clip.y));
            let (x1, y1) = ((r.x + r.w).min(clip.x + clip.w), (r.y + r.h).min(clip.y + clip.h));
            (x1 > x0 && y1 > y0).then_some((id, super::layout::RectPx { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }))
        })
        .collect();
    st.hot = hot.clone();
    HOT_DBG.with(|q| *q.borrow_mut() = hot.clone());
    PTR_DBG.with(|q| *q.borrow_mut() = (pos, st.hovered.clone()));

    // Inspect：悬停即更新目标层（eGUI `present__player.rs`：只在指针位于舞台且裁切区内时更新）。
    if st.inspect
        && st.inside
        && let Some(q) = pos
        && q.0 >= f64::from(clip.x)
        && q.0 <= f64::from(clip.x + clip.w)
        && q.1 >= f64::from(clip.y)
        && q.1 <= f64::from(clip.y + clip.h)
        && let Some(id) = layer_at(&disp, root, &place, q)
    {
        st.inspected = Some(id);
    }

    // “悬停/按下时”临时状态：离开就回退。
    if let Some(tmp) = st.temp.clone() {
        let leave = if tmp.press { released || !down_now } else { pos.is_none_or(|q| !tmp.rect.contains(q)) };
        if leave {
            st.revert(now);
            return;
        }
    }

    // 滚轮：最内层可滚画框，否则整窗平移。范围用未滚动的基础文档算（eGUI 用 `p.doc`）。
    if wheel != (0.0, 0.0)
        && let Some(q) = pos
        && st.inside
    {
        let mut targets: Vec<Scroller> = scrollers_at(st, &base, &disp, root, place, q).into_iter().map(Scroller::Frame).collect();
        if st.pan_range != (0.0, 0.0) {
            targets.push(Scroller::View);
        }
        let dt = (now - st.last_input).clamp(1.0 / 500.0, 0.1);
        wheel_scroll(st, &base, targets, wheel, dt, g.scale);
    } else if wheel == (0.0, 0.0)
        && let Some((t, v)) = st.wheel.take()
        && v.0.hypot(v.1) > COAST_MIN * units_per_point(t, g.scale)
    {
        st.fling = Some(Fling { target: t, vel: v, decay: DECAY_WHEEL, bounce: false, at: now });
    }
    st.last_input = now;

    // 指针在 hotspot 上：手型光标。
    let hits: Vec<(NodeId, super::layout::RectPx)> = hot.iter().rev().filter(|(_, r)| pos.is_some_and(|q| r.contains(q))).copied().collect();
    if !hits.is_empty() {
        st.cursor = CURSOR_POINTER;
    }
    let pick = |want: &dyn Fn(&Trigger) -> bool| -> Option<(NodeId, usize, super::layout::RectPx)> {
        hits.iter().find_map(|(id, r)| find_trigger(&evaluated, *id, want).map(|k| (*id, k, *r)))
    };

    // 松开拖拽 → 惯性：每帧开头判定（eGUI 在 input 顶部调用 `release_drag_scroll`）。
    release_drag_scroll(st, down_now, velocity, now, g.scale);

    // 进入 / 离开 / 悬停。
    let ids: Vec<NodeId> = hits.iter().map(|h| h.0).collect();
    let entered: Vec<(NodeId, super::layout::RectPx)> = hits.iter().filter(|(id, _)| !st.hovered.contains(id)).copied().collect();
    let left: Vec<NodeId> = st.hovered.iter().filter(|id| !ids.contains(id)).copied().collect();
    st.hovered = ids;
    for id in left {
        if let Some(k) = find_trigger(&evaluated, id, |t| *t == Trigger::MouseLeave)
            && st.gesture(id, k, None, now, root)
        {
            return;
        }
    }
    for (id, r) in &entered {
        if let Some(k) = find_trigger(&evaluated, *id, |t| *t == Trigger::MouseEnter)
            && st.gesture(*id, k, Some(*r), now, root)
        {
            return;
        }
    }
    if st.temp.is_none()
        && !entered.is_empty()
        && let Some((id, k, r)) = pick(&|t| *t == Trigger::OnHover)
        && entered.iter().any(|e| e.0 == id)
    {
        // 临时状态与过渡由 `fire_from` 自己建（它拿得到触发层矩形）。
        if st.fire_from(id, k, Some(r), now, true) {
            return;
        }
    }

    // 按下 / 拖拽 / 松开。eGUI 只在指针位于播放区内时才认按下（`over_area`），
    // 否则在 Inspect 侧栏等区域点一下也会闪热区提示。
    if let Some(&(px, py)) = pressed.first().filter(|_| st.inside) {
        st.press = Some((px, py));
        st.dragged = false;
        st.wheel = None;
        // 按下会抓住惯性；只有还在越界回弹中的那一个继续弹（eGUI `overscrolled`）。
        let overscrolled =
            |f: &Fling| st.design.as_ref().is_some_and(|b| clamp_scroll(st.offset(f.target), st.limits(b, f.target)) != st.offset(f.target));
        st.fling = st.fling.filter(overscrolled);
        // 点在最上层覆盖层外：按需关闭它。
        if let (Some((_, _, orect)), Some(o), Some(q)) = (g.overlays.last(), st.overlays.last(), pos)
            && !orect.contains(q)
        {
            if o.close_on_outside {
                st.close_overlay(now);
                st.press = None;
                st.hovered.clear();
            }
            return;
        }
        if let Some((id, k, r)) = pick(&|t| *t == Trigger::MouseDown)
            && st.gesture(id, k, Some(r), now, root)
        {
            return;
        }
        if let Some((id, k, r)) = pick(&|t| *t == Trigger::OnPress) {
            // 临时状态由 `fire_from` 建，确保用的是该交互自己的过渡时长。
            if st.fire_from(id, k, Some(r), now, true) {
                return;
            }
        }
    }
    if down_now
        && !st.dragged
        && let (Some(start), Some(q)) = (st.press, pos)
        && (start.0 - q.0).hypot(start.1 - q.1) > 6.0
    {
        st.dragged = true;
        let at_start: Vec<(NodeId, super::layout::RectPx)> = hot.iter().rev().filter(|(_, r)| r.contains(start)).copied().collect();
        let d = (q.0 - start.0, q.1 - start.1);
        let drags = |id: NodeId| {
            evaluated.node(id)?.interactions.iter().position(|it| it.trigger == Trigger::OnDrag && it.transition.accepts_drag(d.0 as f32, d.1 as f32))
        };
        if let Some((id, k, r)) = at_start.iter().find_map(|(id, r)| drags(*id).map(|k| (*id, k, *r))) {
            let it = evaluated.node(id).and_then(|n| n.interactions.get(k)).cloned();
            if st.fire_from(id, k, Some(r), now, true) {
                st.press = None;
                let fdur = it.as_ref().map_or(0.0, |it| f64::from(it.duration_ms) / 1000.0);
                let (dir, len) = it.as_ref().map_or(((0.0, 0.0), 1.0), |it| {
                    let (x, y) = it.transition.direction().map_or_else(
                        || {
                            if d.0.abs() >= d.1.abs() { (d.0.signum(), 0.0) } else { (0.0, d.1.signum()) }
                        },
                        direction_vec,
                    );
                    let len = if x != 0.0 { f64::from(g.screen.w) } else { f64::from(g.screen.h) };
                    ((x, y), len.max(1.0))
                });
                st.follow = Some((start, dir, len, fdur));
                let _ = r;
                return;
            }
        }
        begin_drag_scroll(st, &disp, root, place, start, d);
    }
    drag_scrolls(st, down_now, pos, g.scale);
    if released {
        if st.press.is_some() {
            let dragged = st.dragged;
            st.press = None;
            st.dragged = false;
            st.follow = None;
            if let Some((id, k, r)) = pick(&|t| *t == Trigger::MouseUp)
                && st.gesture(id, k, Some(r), now, root)
            {
                return;
            }
            if !dragged {
                if let Some((id, k, r)) = pick(&|t| *t == Trigger::OnClick) {
                    // 热区矩形一起传：OpenOverlay 的锚点（Manual / anchor_to）取自它，丢了就
                    // 退回屏幕居中，浮层位置就会错（eGUI 的 onClick 传 `Some(r)`）。
                    if st.fire_from(id, k, Some(r), now, true) {
                        return;
                    }
                }
                // widget 的可点部分：帧后交给插件回调（eGUI `widget_clicks`）。
                if let Some(q) = pos
                    && let Some(t) = layer_at(&disp, root, &place, q).and_then(|hit| {
                        let (px, py) = place.page(q);
                        figcraft_engine::widgets::target_at(&disp, hit, figcraft_geom::Point::new(px, py))
                    })
                {
                    st.widget_clicks.push(t);
                    return;
                }
                // 点空白处：闪一下可点区域。
                if st.hints {
                    st.flash_until = now + super::FLASH_SECS;
                }
            }
        }
    }
    if down_now {
        return;
    }

    // 按键：原型触发器优先，其次播放器自己的键。
    for (key, mods) in &keys {
        let found = evaluated.subtree(root).into_iter().find_map(|id| find_trigger(&evaluated, id, |t| t.fires_on_key(key, *mods)).map(|k| (id, k)));
        if let Some((id, k)) = found {
            if st.fire_node(id, k, now) {
                return;
            }
            continue;
        }
        if mods_empty(*mods) && key.eq_ignore_ascii_case("r") {
            st.restart(st.start, now);
            return;
        }
        // 响应式铺满的演示没有尺寸档位可切（eGUI `Z if !pane && (device.is_some() || fit.is_none())`）。
        if mods_empty(*mods) && key.eq_ignore_ascii_case("z") && !g.pane && (g.device.is_some() || st.fit.is_none()) {
            st.cycle_scale();
            return;
        }
        let forward = match key.to_ascii_lowercase().as_str() {
            "arrowright" | "space" | "n" => Some(true),
            "arrowleft" => Some(false),
            _ => None,
        };
        if let Some(forward) = forward
            && st.overlays.is_empty()
        {
            let page = evaluated.page_of(st.current).unwrap_or(st.current);
            let frames = evaluated.top_frames(page);
            if let Some(i) = frames.iter().position(|f| *f == st.current) {
                let next = if forward { frames.get(i + 1) } else { i.checked_sub(1).and_then(|j| frames.get(j)) };
                if let Some(n) = next.copied() {
                    st.go(n);
                    return;
                }
            }
        }
    }

    // 定时器：画面到达后开始的 AfterDelay（过渡播放期间暂停）。
    if st.from.is_none()
        && let Some(((id, k), due)) = next_delay(st, &evaluated, root, now)
        && due <= 0.0
    {
        st.delays_fired.push((id, k));
        st.fire_node(id, k, now);
    }
}

/// `root` 子树里最早到期且未触发的 AfterDelay，以及距到期秒数。
pub fn next_delay(st: &PresentState, doc: &Document, root: NodeId, now: f64) -> Option<((NodeId, usize), f64)> {
    let since = st.overlays.last().filter(|o| o.id == root).map_or(st.arrived, |o| o.opened);
    let mut next: Option<((NodeId, usize), f64)> = None;
    for id in doc.subtree(root) {
        let Some(n) = doc.node(id) else { continue };
        for (k, it) in n.interactions.iter().enumerate() {
            if let Trigger::AfterDelay { ms } = it.trigger
                && !st.delays_fired.contains(&(id, k))
            {
                let due = since + f64::from(ms) / 1000.0 - now;
                if next.is_none_or(|(_, d)| due < d) {
                    next = Some(((id, k), due));
                }
            }
        }
    }
    next
}

/// 等待中的鼠标触发器（带 delay 的 enter / leave / down / up）。
#[derive(Clone, Debug)]
pub struct Waiting {
    pub node: NodeId,
    pub k: usize,
    pub rect: Option<super::layout::RectPx>,
    pub root: NodeId,
    pub due: f64,
}
