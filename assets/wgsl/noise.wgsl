// FigCraft — 噪点/纹理（外置 WGSL，contract v2）。
//
// 这是 Figma `SHADER_SOURCE_NOISE_EFFECT_FRAGMENT` 的逐函数移植（GLSL → WGSL），
// 不是「看着像」的重画：哈希、梯度噪声、三层 fbm、density 窗口、三种混合模式全部照抄。
//
// Figma 的做法（wasm 里的 uniform 名就是证据）：
//   · 坐标 = tileToNodeSpace(uv) - nodeSpaceCenterPoint，再 × noiseSize（每轴频率）
//     —— 所以噪点锚在节点中心、随文档缩放，不随屏幕缩放；
//   · 值 = 3 层梯度噪声（改进 Perlin，双线性 + smoothstep 权重），每层 seed 由
//     fract(sin(seed*91.3458 + octave*57.123 + 12.345)*43758.5453) 给出；
//   · density 是一个**窗口**而不是阈值：
//       lo = 10*(0.25 - d/4)（duo 的第二色用 0.75），hi = lo + 10*(d/2)，b = v*10
//       keep = step(lo,b) * (1 - step(hi,b))
//     d=0 只留最亮的峰，d=100 全留 —— 所以 density 的量程是 0..100，不是 0..1；
//   · type 0 = multi（RGB 三通道各自一份噪声 + 第四份做 alpha），
//     type 1 = mono（0.213/0.715/0.072 亮度），type 2 = duo（两个互补窗口，两个颜色）；
//   · alpha = density × color.a × 图层自身 alpha。
//
// 坐标：`nodeCenterPx` 是节点中心在本窗口里的设备像素，`nodeRotation` 是世界变换的旋转角；
// 两者把 frag 转回节点轴，所以噪点锚在节点中心，旋转图层时整个图案跟着转。
//
// 接线：content（预乘 + mip，只读 rgb 与 a）、fields（形状覆盖）、rect 不扩；
//       输出整层 straight alpha，子树已烤进 content，不要再叠 `Step::Over`。

struct Uniforms {
    resolution     : vec2<f32>,
    regionOriginPx : vec2<f32>,
    nodeSizePx     : vec2<f32>,
    nodeCenterPx   : vec2<f32>,
    nodeRotation   : f32,   // 弧度，节点局部 → 屏幕的旋转
    kind           : f32,   // 0 multi / 1 mono / 2 duo
    size           : f32,   // 文档 px —— 一个噪声单元的大小（Figma 存的是它的倒数）
    density        : f32,   // 0..100
    seed           : f32,
    color_r        : f32,
    color_g        : f32,
    color_b        : f32,
    color_a        : f32,
    sec_r          : f32,
    sec_g          : f32,
    sec_b          : f32,
    sec_a          : f32,
    dpr            : f32,
};

@group(0) @binding(0) var content : texture_2d<f32>;
@group(0) @binding(1) var samp    : sampler;
@group(0) @binding(2) var<uniform> u : Uniforms;

// @fx id noise
// @fx label Noise
// @fx stage layer
// @fx param u.kind float 0 2 1 1 "Type (0 multi 1 mono 2 duo)"
// @fx param u.size float 1 100 1 40 "Size"
// @fx param u.density float 0 100 1 50 "Density"
// @fx param u.seed float 0 100 1 0 "Seed"
// @fx param u.color_r float 0 1 0.01 0 "Color R"
// @fx param u.color_g float 0 1 0.01 0 "Color G"
// @fx param u.color_b float 0 1 0.01 0 "Color B"
// @fx param u.color_a float 0 1 0.01 1 "Color A"
// @fx param u.sec_r float 0 1 0.01 1 "Second R"
// @fx param u.sec_g float 0 1 0.01 1 "Second G"
// @fx param u.sec_b float 0 1 0.01 1 "Second B"
// @fx param u.sec_a float 0 1 0.01 1 "Second A"

struct VSOut {
    @builtin(position) pos : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi : u32) -> VSOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var out : VSOut;
    let xy = corners[vi];
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

fn texel_uv(uv : vec2<f32>) -> vec2<f32> {
    let t = 0.5 / max(u.resolution, vec2<f32>(1.0, 1.0));
    return clamp(uv, t, vec2<f32>(1.0, 1.0) - t);
}

/// Figma 的 `i()`：格点哈希。
fn hash21(p : vec2<f32>) -> f32 {
    let n = dot(p, vec2<f32>(12.9898, 78.233));
    let m = n - 3.14159 * floor(n / 3.14159);
    return fract(sin(m * 2.71828) * 438.5453 + 1.73205);
}

/// Figma 的 `p()`：一个方向上的梯度分量。
fn grad1(q : vec2<f32>, dir : vec2<f32>, s : f32) -> f32 {
    var t = dot(dir, q) * 0.25;
    t = sin(t * 0.25) * (s + 312.0);
    return 2.0 * (fract(t) - 0.5);
}

fn rnd1(x : f32) -> f32 {
    return fract(sin(x * 91.3458 + 12.345) * 43758.5453);
}

/// Figma 的 `x()`：格点上的单位梯度。
fn gradient_at(p : vec2<f32>, z : f32) -> vec2<f32> {
    let r = hash21(p);
    let s1 = z * r * 1.61803 + 18.345;
    let s2 = z * r * 0.61803 + 21.543;
    let gx = grad1(p, vec2<f32>(12.42, 78.23) * rnd1(z + 2.6457), s1);
    let gy = grad1(p, vec2<f32>(-19.98, 31.21) * rnd1(z + 3.6055), s2);
    let l = max(length(vec2<f32>(gx, gy)), 1e-6);
    return vec2<f32>(gx / l, gy / l);
}

fn smooth01(t : f32) -> f32 { return t * t * (3.0 - 2.0 * t); }
fn lerp2(a : f32, b : f32, t : f32) -> f32 { return a + t * (b - a); }

/// Screen offset → the node's axes. The noise is anchored to the node, so a rotated layer must
/// rotate its pattern too; otherwise the grain stays upright and visibly swims across the fill.
fn node_axes(v : vec2<f32>, angle : f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * v.x + s * v.y, -s * v.x + c * v.y);
}

/// Figma 的 `aj()`：双线性 + smoothstep 的梯度噪声。
fn value_noise(z : f32, q : vec2<f32>) -> f32 {
    let i0 = floor(q);
    let f = q - i0;
    let s = vec2<f32>(smooth01(f.x), smooth01(f.y));
    let n00 = dot(f, gradient_at(i0, z));
    let n10 = dot(f - vec2<f32>(1.0, 0.0), gradient_at(i0 + vec2<f32>(1.0, 0.0), z));
    let n01 = dot(f - vec2<f32>(0.0, 1.0), gradient_at(i0 + vec2<f32>(0.0, 1.0), z));
    let n11 = dot(f - vec2<f32>(1.0, 1.0), gradient_at(i0 + vec2<f32>(1.0, 1.0), z));
    return lerp2(lerp2(n00, n10, s.x), lerp2(n01, n11, s.x), s.y);
}

/// Figma 的 `ay()`：每层的 seed。
fn octave_seed(s : i32, o : i32) -> f32 {
    return fract(sin(f32(s) * 91.3458 + f32(o) * 57.123 + 12.345) * 43758.5453);
}

/// Figma 的 `bb()`：3 层 fbm，权重 1, 1/2, 1/4。
fn fbm(s : i32, q : vec2<f32>) -> f32 {
    var acc = 0.0;
    var p = q;
    var amp = 1.0;
    for (var i = 0; i < 3; i = i + 1) {
        acc = acc + value_noise(octave_seed(s, i), p) / amp;
        p = p * 2.0;
        amp = amp * 2.0;
    }
    return acc;
}

/// Figma 的 `d()`：density 窗口。
fn density_window(v : f32, density : f32, inverted : bool) -> f32 {
    var lo : f32;
    if (inverted) {
        lo = 10.0 * (0.75 - density / 4.0);
    } else {
        lo = 10.0 * (0.25 - density / 4.0);
    }
    let hi = lo + 10.0 * (density / 2.0);
    let b = v * 10.0;
    return step(lo, b) * (1.0 - step(hi, b));
}

fn unpack(p : vec4<f32>) -> vec4<f32> {
    return vec4<f32>(p.rgb / max(p.a, 1e-5), p.a);
}

@fragment
fn fs(in : VSOut) -> @location(0) vec4<f32> {
    let frag = u.regionOriginPx + in.uv * max(u.resolution, vec2<f32>(1.0, 1.0));
    let base = unpack(textureSampleLevel(content, samp, texel_uv(in.uv), 0.0));

    // 节点中心为原点、文档单位；Figma 的 noiseSize 是「每文档单位的频率」。先转回节点轴，
    // 旋转图层里的噪点才跟着图层转。
    let p = node_axes(frag - u.nodeCenterPx, u.nodeRotation) / max(u.dpr, 1e-4);
    let freq = 1.0 / max(u.size, 0.5);
    let q = p * freq;

    let s = i32(round(u.seed));
    let n0 = (fbm(s + 0, q) + 1.0) * 0.5;
    let n1 = (fbm(s + 1, q) + 1.0) * 0.5;
    let n2 = (fbm(s + 2, q) + 1.0) * 0.5;
    let n3 = (fbm(s + 3, q) + 1.0) * 0.5;

    let col = vec3<f32>(u.color_r, u.color_g, u.color_b);
    let sec = vec3<f32>(u.sec_r, u.sec_g, u.sec_b);

    var noise_rgb = col;
    var amount = 0.0;   // 0..1，噪点在本层上的占比

    let kind = i32(round(u.kind));
    if (kind == 0) {
        // multi：三个通道各自的噪声（2t-0.5，和图里一样，负值最终会被目标格式裁掉）。
        noise_rgb = clamp(vec3<f32>(2.0 * n0 - 0.5, 2.0 * n1 - 0.5, 2.0 * n2 - 0.5), vec3<f32>(0.0), vec3<f32>(1.0));
        amount = density_window(n3, u.density, false) * u.color_a;
    } else if (kind == 1) {
        // mono：亮度。
        let l = 0.213 * n0 + 0.715 * n1 + 0.072 * n2;
        noise_rgb = col;
        amount = density_window(l, u.density, false) * u.color_a;
    } else {
        // duo：互补的两个窗口，第二色优先。
        let l = 0.213 * n0 + 0.715 * n1 + 0.072 * n2;
        let a1 = density_window(l, u.density, false);
        if (a1 >= 1.0) {
            noise_rgb = sec;
            amount = u.sec_a;
        } else {
            noise_rgb = col;
            amount = density_window(l, u.density, true) * u.color_a;
        }
    }

    // Figma 的 noise 是层内的一道 paint：颜色按占比推上去，覆盖不变（a_n 已乘过 tile alpha）。
    let k = clamp(amount, 0.0, 1.0);
    let rgb = mix(base.rgb, noise_rgb, k);
    return vec4<f32>(rgb, base.a);
}
