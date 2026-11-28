//! 逐层外置材质（`figcraft_render::shade`）的 GPU 后端：一层底图进，一层像素出。
//!
//! 与整页合成（`passes_for` + `children_layer` + `assemble_steps`）的区别只有「输入边界」：
//! 这里的 canvas 是**某一层自己的底图**，`rect_px` 恒为整块，`origin_px` 描述「这块窗口在节点里
//! 的位置」。所以不再需要挖空主图，也不必把子树重渲一遍——子树由 `draw_layer` 的普通递归
//! 画在材质结果之上。
//!
//! 颜色通道：渲染器给的是**直通 RGBA**，而 runner 的 canvas 纹理是 `Bgra8Unorm`，进出一律换序。
#![cfg(feature = "gpu")]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_render::shade::{ShadeBoard, ShadeRequest, Shaded, ShaderSpec};
use figcraft_render::wgsl_effect as fx;

/// 一页里可见且绑了材质的图层，装成 `ShadeBoard` 需要的形状。
///
/// 与 `wgsl_runner::passes_for` 用的是同一份绑定（`plugin_data` + `wgsl_material` 的解析器），
/// 所以两条通道认的是同一批节点、同一套参数；换 shader 文件不改变节点的匹配方式。
pub fn specs(doc: &Document, page: NodeId) -> HashMap<NodeId, Arc<ShaderSpec>> {
    let mut out = HashMap::new();
    for id in doc.subtree(page) {
        let Some(n) = doc.node(id) else { continue };
        if matches!(n.kind, NodeKind::Page { .. }) || !visible(doc, id) {
            continue;
        }
        let Some(json) = n.plugin_data.get(crate::wgsl_material::BINDING_KEY) else { continue };
        let Some(binding) = crate::wgsl_material::WgslMaterialBinding::from_json(json) else { continue };
        let effect_id = crate::wgsl_material::ensure_loaded(&binding.path);
        let (stage, reach_doc, blur) = {
            let reg = fx::shared().lock().unwrap_or_else(|e| e.into_inner());
            let stage = reg.stage(&effect_id);
            let reach = reg.reach(&effect_id).map(str::to_owned);
            let value = reach
                .as_ref()
                .and_then(|f| binding.params.get(f).copied().or_else(|| reg.params(&effect_id).iter().find(|p| &p.field == f).map(|p| p.default)));
            // A shadow reaches `blur + |offset| + |spread|`, not just its blur radius. A window
            // grown by blur alone clips the shadow at the layer edge (a band of it vanishes), so
            // every offset/spread parameter in the shader is added to the reach here.
            let mut offset = 0.0_f64;
            for p in reg.params(&effect_id) {
                let leaf = p.field.rsplit('.').next().unwrap_or(&p.field);
                if matches!(leaf, "dx" | "dy" | "spread" | "frost") {
                    let v = binding.params.get(&p.field).copied().unwrap_or(p.default);
                    offset = offset.max(v.abs());
                }
            }
            let blur = reg.blur(&effect_id).map(|(input, field)| {
                let radius_doc = binding
                    .params
                    .get(&field)
                    .copied()
                    .or_else(|| reg.params(&effect_id).iter().find(|p| &p.field == &field).map(|p| p.default))
                    .unwrap_or(1.0)
                    .clamp(1.0, 100.0);
                figcraft_render::shade::BlurSpec { input, field, radius_doc }
            });
            (stage, value.map_or(0.0, |v| v.abs() + offset + 4.0), blur)
        };
        out.insert(id, Arc::new(ShaderSpec { effect_id, path: binding.path, params: binding.params, stage, reach_doc, blur }));
    }
    out
}

/// 把这一帧登记的每一层都算出来，装回板上。返回真正交付的层数。
///
/// 一层一次 `composite_steps`：区域就是这个层自己，没有并集、没有跨节点共享的读回缓冲。
/// 调用点在 worker 线程上（不是 UI 线程），所以这里的同步等待不会卡住输入。
pub fn shade_pending(board: &ShadeBoard, runner: &mut crate::wgsl_runner::WgslRunner, doc: &Document) -> usize {
    let requests = board.take_requests();
    if requests.is_empty() {
        return 0;
    }
    let t0 = std::time::Instant::now();
    let mut done = 0usize;
    let mut timing = ShadeTiming::default();
    let mut px = 0u64;
    for req in &requests {
        if crate::canvas::render_worker::render_log() {
            eprintln!(
                "[shade] req node={:?} layer={}x{} node_px={:?} origin_px={:?} bounds={:?} scale={:.3}",
                req.node, req.size.0, req.size.1, req.node_px, req.origin_px, req.bounds, req.scale
            );
        }
        px += u64::from(req.size.0) * u64::from(req.size.1);
        if let Some(shaded) = shade_one(req, runner, doc, &mut timing) {
            board.put_shared(req.node, std::sync::Arc::new(shaded));
            done += 1;
        }
    }
    if crate::canvas::render_worker::render_log() {
        eprintln!(
            "[shade] {} layers {:.1}ms (pass_for {:.1} + swap {:.1} + gpu {:.1} + extract {:.1}) ok={done}/{} mpx={:.2}",
            requests.len(),
            t0.elapsed().as_secs_f64() * 1000.0,
            timing.pass,
            timing.swap,
            timing.gpu,
            timing.extract,
            requests.len(),
            px as f64 / 1.0e6,
        );
    }
    done
}

/// 一层请求的分段耗时，供 `[shade]` 日志（统一解析路径里由调用方汇总）。
#[derive(Default, Clone, Copy)]
pub struct ShadeTiming {
    pub pass: f64,
    pub swap: f64,
    pub gpu: f64,
    pub extract: f64,
}

/// 同步产出一层的像素。内置效果与 WGSL 材质共用这一条路径：绘制到该层时由解析器调用。
pub fn shade_one(req: &ShadeRequest, runner: &mut crate::wgsl_runner::WgslRunner, doc: &Document, timing: &mut ShadeTiming) -> Option<Shaded> {
    let t_pass = std::time::Instant::now();
    let mut pass = pass_for(req, doc);
    timing.pass += t_pass.elapsed().as_secs_f64() * 1000.0;
    let (w, h) = req.size;
    // 外置材质自己不实现 blur：`@fx blur <input> <field>` 让宿主按 Figma 的可分离 box 先把
    // 输入糊好，shader 只采样糊后的纹理。这样背景模糊、图层模糊和未来任何材质共用同一条
    // 契约，不需要在每个 WGSL 文件里各写一套 tap 循环。
    let mut canvas = if req.content.is_some() && req.stage == fx::Stage::Layer {
        vec![0u8; (w as usize) * (h as usize) * 4]
    } else {
        req.backdrop.as_ref().clone()
    };
    if let Some(blur) = &req.spec.blur {
        let radius_px = (blur.radius_doc * req.scale).max(0.0);
        match blur.input {
            fx::BlurInput::Backdrop => {
                canvas = figcraft_render::blur::box_premultiplied(&req.backdrop, w, h, radius_px);
            }
            fx::BlurInput::Content => {
                if let Some(content) = &req.content {
                    pass.content = Some(Arc::new(figcraft_render::blur::box_premultiplied(content, w, h, radius_px)));
                }
            }
        }
    }
    // runner 的 canvas 是 BGRA：进去换一次，出来的也换回来。**两种输入都按预乘 alpha
    // 上传**（`layer` 的 content 与 `backdrop` 的底图同一契约），所以画布初值也必须是
    // 预乘值；shader 用 `unpack` 还原成直通做运算，输出直通，由 ALPHA_BLENDING 重新合成。
    // 读回时不能再乘一次 alpha（那会把每个抗锯齿边压暗成黑边）。
    let t_swap = std::time::Instant::now();
    crate::canvas::render_worker::rgba_to_bgra(&mut canvas);
    timing.swap += t_swap.elapsed().as_secs_f64() * 1000.0;
    if let Ok(dir) = std::env::var("FIGCRAFT_SHADE_DUMP") {
        if let Some(inp) = figcraft_render::shade::from_premultiplied(&canvas, w, h) {
            let _ = inp.save_png(std::path::Path::new(&dir).join(format!("in-{:?}-{}.png", req.node, req.key)));
        }
    }
    let t_gpu = std::time::Instant::now();
    let out = runner.composite_steps(&canvas, w, h, &[crate::wgsl_runner::Step::Pass(&pass)], None);
    timing.gpu += t_gpu.elapsed().as_secs_f64() * 1000.0;
    let mut out = out?;
    let t_swap = std::time::Instant::now();
    crate::canvas::render_worker::rgba_to_bgra(&mut out);
    timing.swap += t_swap.elapsed().as_secs_f64() * 1000.0;
    // Canvas input and the blended output are both premultiplied.
    let t_extract = std::time::Instant::now();
    let px_out = figcraft_render::shade::from_premultiplied(&out, w, h)?;
    timing.extract += t_extract.elapsed().as_secs_f64() * 1000.0;
    if let Ok(dir) = std::env::var("FIGCRAFT_SHADE_DUMP") {
        let _ = px_out.save_png(std::path::Path::new(&dir).join(format!("shade-{:?}-{}.png", req.node, req.key)));
    }
    Some(Shaded { key: req.key, pixmap: px_out, origin_px: req.origin_px })
}

/// 一层 → 一次 GPU pass。几何导出与 `passes_for` 逐字段对齐：两条通道对同一节点必须打包出
/// 同一份 uniform，否则「逐层」与「整页」的像素没法比对。
fn pass_for(req: &ShadeRequest, doc: &Document) -> crate::wgsl_runner::RunPass {
    let (rw_px, rh_px) = req.size;
    let sc = req.scale.max(1e-9);
    let [bx0, by0, bx1, by1] = req.bounds;
    let (cx, cy) = ((bx0 + bx1) / 2.0, (by0 + by1) / 2.0);
    // The node's real extent, not the AABB: a rotated rectangle's AABB is wider than the shape, and
    // shaders use this as half-axes (shadow) or as the noise's spatial period.
    let dw = f64::from(req.node_px.0) / sc;
    let dh = f64::from(req.node_px.1) / sc;
    let weights = doc.world_transform(req.node).as_coeffs();
    let rotation = weights[1].atan2(weights[0]);
    let center_px = ((cx - bx0) * sc + f64::from(req.origin_px.0), (cy - by0) * sc + f64::from(req.origin_px.1));
    let radii_doc = req.radii_px.map(|v| f64::from(v) / sc);
    let uniform_corners = radii_doc.iter().all(|v| (v - radii_doc[0]).abs() < 1e-6);

    let mut ex = fx::EngineExports::default();
    ex.set("resolution", vec![f64::from(rw_px), f64::from(rh_px)]);
    ex.set("nodeSize", vec![dw, dh]);
    ex.set("quad", vec![dw / 2.0, dh / 2.0, cx, cy]);
    ex.set("canvasSize", vec![f64::from(rw_px) / sc, f64::from(rh_px) / sc]);
    ex.set("nodeOrigin", vec![bx0, by0]);
    ex.set("nodeSpaceCenterPoint", vec![cx, cy]);
    ex.set("cornerRadii", radii_doc.to_vec());
    ex.set("cornerSmoothings", vec![req.smoothing; 4]);
    ex.set("hasUniformCorners", vec![if uniform_corners { 1.0 } else { 0.0 }]);
    ex.set("dpr", vec![sc]);
    ex.set("nodeSizePx", vec![f64::from(req.node_px.0), f64::from(req.node_px.1)]);
    ex.set("regionOriginPx", vec![f64::from(req.origin_px.0), f64::from(req.origin_px.1)]);
    let is_rect = crate::wgsl_runner::analytic_rect(doc, req.node);
    ex.set("isRectangle", vec![f64::from(u8::from(is_rect))]);
    if req.local {
        // The host already rotated the layer/backdrop into the node's own frame. Every shader must
        // see a plain axis-aligned node here; reporting the world rotation would rotate it twice.
        ex.set("nodeRotation", vec![0.0]);
        ex.set("nodeCenterPx", vec![f64::from(req.node_px.0) * 0.5, f64::from(req.node_px.1) * 0.5]);
    } else {
        ex.set("nodeRotation", vec![rotation]);
        ex.set("nodeCenterPx", vec![center_px.0, center_px.1]);
    }
    ex.set("scale", vec![1.0]);
    ex.set("time", vec![material_time()]);

    let params: &BTreeMap<String, f64> = &req.spec.params;
    let (uniforms, field_band) = crate::wgsl_runner::pack_uniforms(&req.spec.effect_id, params, &ex, dw, dh, sc);
    let field_bytes = if req.local {
        crate::wgsl_runner::prepare_fields_local(doc, req.node, rw_px, rh_px, sc, req.origin_px, req.node_px, req.radii_px)
    } else {
        crate::wgsl_runner::prepare_fields(doc, req.node, rw_px, rh_px, sc, req.origin_px)
    };
    crate::wgsl_runner::RunPass {
        effect: req.spec.effect_id.clone(),
        node: req.node,
        // 这一层就是整块 canvas：区域从 (0,0) 起，尺寸即纹理尺寸。
        rect_px: [0, 0, rw_px, rh_px],
        radii_px: req.radii_px,
        node_px: req.node_px,
        origin_px: req.origin_px,
        field_bytes,
        field_band,
        uniforms,
        content: req.content.clone(),
        dpr: sc as f32,
    }
}

/// 这些材质在**自己那层之外**还要读多少设备像素 —— 分带渲染靠它决定每条带多渲染多少行。
///
/// 材质的空间 reach 就是它的透镜带：`depth`（文档单位，已经由 [`figcraft_doc::Glass::
/// depth_reach_doc`] 换算过）加一单位的 bevel，乘回 `scale` 变设备像素。不把它算进 overlap，
/// 每条带就会在带边界处采到被截断的底图：每条带各画一遍自己的边缘高光，接缝处留下亮带。
/// 材质的阅读距离完全由参数决定，与底图无关，所以这里只需要遍历绑定，不需要底图像素。
pub fn reach_px(doc: &Document, page: NodeId, scale: f64) -> f64 {
    let mut worst: f64 = 0.0;
    for (id, spec) in specs(doc, page) {
        let Some(n) = doc.node(id) else { continue };
        let short = n.width.min(n.height).max(1.0);
        let depth = spec.params.iter().find(|(k, _)| k.as_str() == "depth" || k.ends_with(".depth")).map(|(_, v)| *v).unwrap_or(20.0);
        worst = worst.max(figcraft_doc::Glass::depth_reach_doc(depth, short));
    }
    if worst <= 0.0 {
        return 0.0;
    }
    (worst + 1.0) * scale.max(1e-9) + 4.0
}

fn visible(doc: &Document, id: NodeId) -> bool {
    let mut cur = Some(id);
    while let Some(nid) = cur {
        let Some(n) = doc.node(nid) else { return false };
        if !n.visible {
            return false;
        }
        cur = n.parent;
    }
    true
}

/// 与整页通道共用的材质时钟（同一原点，`time` 才对得上）。
fn material_time() -> f64 {
    static CLOCK: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    CLOCK.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}
