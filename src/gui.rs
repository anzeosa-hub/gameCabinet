//! egui 桌面界面：苹果拟态玻璃（Glassmorphism）风格。
//! 手法：全屏渐变底 + 彩色辉光 → 半透明白面板 + 细描边 + 大圆角 + 柔和阴影。
//! 边界规范参考 emilkowalski/skills：长文本截断+tooltip、占位纹理、扫描态 spinner。

use crate::library::GameEntry;
use eframe::emath::pos2;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

/// 后台扫描线程完成后回传的消息
pub struct ScanMsg {
    pub library: crate::library::Library,
    pub dir: PathBuf,
}

/// 后台「按名称重新匹配」完成后回传的消息
pub struct RematchMsg {
    pub idx: usize,
    pub new_name: String,
    pub appid: Option<u32>,
    pub matched: Option<String>,
    pub cover_local: Option<String>,
    pub cover_url: Option<String>,
    pub header_url: Option<String>,
    pub error: Option<String>,
}

/// 后台下载「横版英雄图」完成后回传的消息（相对库根目录的路径）
pub struct HeroMsg {
    pub idx: usize,
    pub rel: String,
}

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
/// 当前主题：true=夜色（默认），false=日间。设计令牌函数都读它。
static THEME_DARK: AtomicBool = AtomicBool::new(true);
fn is_dark() -> bool {
    THEME_DARK.load(Ordering::Relaxed)
}

// ---- 动效基础设施 ----
// 设计令牌（颜色/填充）本身不知道「当前这块内容淡入到几成」，所以走一个渲染期全局量：
// 默认 1.0（不透明）；详情切换 / 空态入场时把它调低，令牌函数自动带上这个 alpha，
// 整块内容就整体淡入 —— 不用给每个控件单独穿一个 alpha 参数。
// AtomicF32 尚未稳定，用 AtomicU32 存 f32 位模式。
static UI_ALPHA: AtomicU32 = AtomicU32::new(0x3F80_0000); // 1.0f32.to_bits()
fn ui_alpha() -> f32 {
    f32::from_bits(UI_ALPHA.load(Ordering::Relaxed))
}
/// 设置全局内容透明度（0=全透，1=不透明）。用完必须还原成 1.0，
/// 否则会影响同一帧后续绘制（比如弹层、状态栏）。
fn set_ui_alpha(a: f32) {
    UI_ALPHA.store(a.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}
/// 按系数缩放颜色 alpha（局部动画用：列表错峰入场等）
fn fa(c: egui::Color32, k: f32) -> egui::Color32 {
    let a = (c.a() as f32 * k.clamp(0.0, 1.0)) as u8;
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}
/// 全局内容透明度的应用：令牌函数统一用它包一层
fn ua(c: egui::Color32) -> egui::Color32 {
    fa(c, ui_alpha())
}
/// 缓动：smoothstep（两端慢中间快）。所有入场位移/淡入统一用它，
/// 避免线性动画那种「机器感」。
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
/// 两个颜色的线性插值（逐通道）。滑块、卡片 hover 底色都用它做补间。
fn mixc(c0: egui::Color32, c1: egui::Color32, k: f32) -> egui::Color32 {
    let k = k.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
    egui::Color32::from_rgba_unmultiplied(
        f(c0.r(), c1.r()),
        f(c0.g(), c1.g()),
        f(c0.b(), c1.b()),
        f(c0.a(), c1.a()),
    )
}

// ---- 玻璃设计令牌（Apple 风：SF 蓝 + 冷灰夜色 / 日间清玻璃） ----
// from_rgba_unmultiplied 非 const，这里用函数而非常量；全部随主题切换
fn glass_fill() -> egui::Color32 {
    // 卡片底：低不透明度（透出亚克力模糊层，玻璃感优先）
    if is_dark() {
        ua(egui::Color32::from_rgba_unmultiplied(20, 24, 34, 70))
    } else {
        ua(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 64))
    }
}
fn glass_fill_hover() -> egui::Color32 {
    if is_dark() {
        ua(egui::Color32::from_rgba_unmultiplied(42, 48, 62, 150))
    } else {
        ua(egui::Color32::from_rgba_unmultiplied(248, 250, 255, 190))
    }
}
fn glass_stroke() -> egui::Color32 {
    // 描边：夜色用白（受光边），日间用深蓝灰
    if is_dark() {
        ua(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 22))
    } else {
        ua(egui::Color32::from_rgba_unmultiplied(30, 42, 80, 38))
    }
}
fn text_sub() -> egui::Color32 {
    // 次要文字：保持较高不透明度，透在壁纸上也要可读（对比度规范）
    if is_dark() {
        ua(egui::Color32::from_rgba_unmultiplied(226, 232, 246, 210))
    } else {
        ua(egui::Color32::from_rgba_unmultiplied(64, 74, 96, 225))
    }
}
fn text_main() -> egui::Color32 {
    if is_dark() {
        ua(egui::Color32::from_rgb(240, 244, 252))
    } else {
        ua(egui::Color32::from_rgb(26, 32, 46))
    }
}

// 控件底色：必须自带足够不透明度，否则壁纸透过来会把字吞掉
fn button_fill() -> egui::Color32 {
    if is_dark() {
        ua(egui::Color32::from_rgba_unmultiplied(34, 38, 50, 185))
    } else {
        ua(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 185))
    }
}
fn button_fill_hover() -> egui::Color32 {
    if is_dark() {
        ua(egui::Color32::from_rgba_unmultiplied(54, 60, 78, 205))
    } else {
        ua(egui::Color32::from_rgba_unmultiplied(238, 243, 254, 228))
    }
}
fn button_fill_active() -> egui::Color32 {
    if is_dark() {
        ua(egui::Color32::from_rgba_unmultiplied(22, 26, 36, 225))
    } else {
        ua(egui::Color32::from_rgba_unmultiplied(220, 228, 246, 240))
    }
}
fn accent_fill() -> egui::Color32 {
    ua(egui::Color32::from_rgba_unmultiplied(10, 132, 255, 240))
}
/// 悬停浮起的浅色罩（夜色=白雾，日间=淡影）
fn hover_veil() -> egui::Color32 {
    if is_dark() {
        ua(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 28))
    } else {
        ua(egui::Color32::from_rgba_unmultiplied(20, 30, 60, 16))
    }
}

/// 玻璃卡片 Frame：半透明白 + 圆角 + 细描边
fn glass_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(glass_fill())
        .rounding(egui::Rounding::same(14.0))
        .stroke(egui::Stroke::new(1.0_f32, glass_stroke()))
        .inner_margin(egui::Margin::same(14.0))
}

/// 弹窗玻璃框：近不透明底 + 大圆角 + 细描边 + 悬浮阴影
/// （默认 Window 框太浅，标题栏和正文对比度都不够，统一换成这张）
fn dialog_frame() -> egui::Frame {
    let fill = if is_dark() {
        egui::Color32::from_rgba_unmultiplied(20, 22, 32, 242)
    } else {
        egui::Color32::from_rgba_unmultiplied(250, 251, 255, 246)
    };
    egui::Frame::none()
        .fill(fill)
        .rounding(egui::Rounding::same(16.0))
        .stroke(egui::Stroke::new(1.0_f32, glass_stroke()))
        .shadow(egui::epaint::Shadow {
            offset: egui::vec2(0.0, 14.0),
            blur: 44.0,
            spread: 0.0,
            color: egui::Color32::from_black_alpha(150),
        })
        .inner_margin(egui::Margin::same(14.0))
}

/// SF 蓝主按钮：实心填充 + 白字（主操作专用，如「选择游戏文件夹」「保存」）
/// 统一 30px 高：和单行输入框（desired_height 30）在同一行严格对齐
fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(egui::Color32::WHITE))
            .fill(accent_fill())
            .rounding(egui::Rounding::same(10.0))
            .min_size(egui::vec2(0.0, 30.0)),
    )
}

/// 「运行中」按钮：游戏正开着时顶替「启动游戏」，绿色实心，点击强制关闭
fn running_button(ui: &mut egui::Ui) -> egui::Response {
    let c = if is_dark() {
        egui::Color32::from_rgb(48, 178, 122)
    } else {
        egui::Color32::from_rgb(30, 152, 102)
    };
    ui.add(
        egui::Button::new(egui::RichText::new("■ 运行中").color(egui::Color32::WHITE))
            .fill(c)
            .rounding(egui::Rounding::same(10.0))
            .min_size(egui::vec2(0.0, 30.0)),
    )
    .on_hover_text("点击强制关闭游戏")
}

/// 自绘标题栏的窗口控制按钮：最小化('m') / 最大化切换('x') / 关闭('c')。
/// 图标几何取自 Lucide（24 栅格、圆角、圆头描边），矢量绘制、任意尺寸清晰，
/// 不再用系统字体的符号（缺字形会变豆腐块）。
fn window_control_button(ui: &mut egui::Ui, ctx: &egui::Context, kind: char, maximized: bool) {
    let size = egui::vec2(34.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    // 悬停补间 t：底罩淡入（不是硬切），图标从 0.94 微放大到 1.0 —— 有「被托起」的手感
    let t = ui
        .ctx()
        .animate_value_with_time(egui::Id::new(("wctl", kind)), if resp.hovered() { 1.0 } else { 0.0 }, 0.12);
    let base_c = if is_dark() {
        egui::Color32::from_rgb(222, 226, 234)
    } else {
        egui::Color32::from_rgb(58, 64, 80)
    };
    // 关闭键悬停：底罩渐红 + 图标渐白（红底上白字才够对比度）
    let c = if kind == 'c' {
        mixc(base_c, egui::Color32::from_rgb(255, 255, 255), t)
    } else {
        base_c
    };
    if t > 0.002 {
        let tint = if kind == 'c' {
            fa(egui::Color32::from_rgb(232, 76, 76), t * 0.94)
        } else {
            fa(hover_veil(), t)
        };
        ui.painter().rect_filled(rect, 7.0, tint);
    }
    let p = ui.painter();
    let ctr = rect.center();
    let box_px = 18.0; // 图标在 24 栅格下占 18px 见方
    let sc = box_px / 24.0 * (0.94 + 0.06 * ease(t));
    let pt = |x: f32, y: f32| egui::pos2(ctr.x + (x - 12.0) * sc, ctr.y + (y - 12.0) * sc);
    let sw = 1.7_f32;
    let stroke = egui::Stroke::new(sw, c);
    let cap = sw / 2.0;
    // Lucide 的线条是圆头（round cap），末端补圆点还原观感
    let dot = |a: egui::Pos2| p.circle_filled(a, cap, c);
    let draw_x = |p: &egui::Painter| {
        p.line_segment([pt(18.0, 6.0), pt(6.0, 18.0)], stroke);
        p.line_segment([pt(6.0, 6.0), pt(18.0, 18.0)], stroke);
        for (x, y) in [(18.0, 6.0), (6.0, 18.0), (6.0, 6.0), (18.0, 18.0)] {
            p.circle_filled(pt(x, y), cap, c);
        }
    };
    match kind {
        'm' => {
            // 最小化：横线（Lucide minus）
            let a = pt(5.0, 12.0);
            let b = pt(19.0, 12.0);
            p.line_segment([a, b], stroke);
            dot(a);
            dot(b);
        }
        'x' => {
            if maximized {
                // 还原：两个叠放的圆角方块（Lucide copy 风格）
                let back = egui::Rect::from_two_pos(pt(9.0, 4.0), pt(20.0, 15.0));
                let front = egui::Rect::from_two_pos(pt(4.0, 9.0), pt(15.0, 20.0));
                p.rect_stroke(back, 2.0 * sc, stroke);
                p.rect_stroke(front, 2.0 * sc, stroke);
            } else {
                // 最大化：单个圆角方块（Lucide square）
                let r = egui::Rect::from_two_pos(pt(4.0, 4.0), pt(20.0, 20.0));
                p.rect_stroke(r, 2.0 * sc, stroke);
            }
        }
        'c' => draw_x(p),
        _ => {}
    }
    if resp.clicked() {
        match kind {
            'm' => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            'x' => ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized)),
            'c' => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            _ => {}
        }
    }
}

/// 「库」按钮前的 2x2 网格图标（Lucide layout-grid 风格），矢量描边
fn draw_lib_grid(p: &egui::Painter, rect: egui::Rect, c: egui::Color32) {
    let sc = rect.width() / 24.0;
    let pt = |x: f32, y: f32| egui::pos2(rect.left() + x * sc, rect.top() + y * sc);
    let stroke = egui::Stroke::new(2.0_f32, c);
    for (x, y) in [(3.0, 3.0), (14.0, 3.0), (3.0, 14.0), (14.0, 14.0)] {
        let r = egui::Rect::from_two_pos(pt(x, y), pt(x + 7.0, y + 7.0));
        p.rect_stroke(r, 2.0 * sc, stroke);
    }
}

/// 太阳图标（Lucide sun：中心圆 + 8 道光芒），矢量
fn draw_sun(p: &egui::Painter, c: egui::Pos2, r: f32, col: egui::Color32) {
    p.circle_filled(c, r * 0.52, col);
    let s = egui::Stroke::new(1.3_f32, col);
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let (dx, dy) = a.sin_cos();
        p.line_segment(
            [
                egui::pos2(c.x + dx * r * 0.74, c.y + dy * r * 0.74),
                egui::pos2(c.x + dx * r * 1.05, c.y + dy * r * 1.05),
            ],
            s,
        );
    }
}

/// 月亮图标（Lucide moon 的月牙）：外圆减去右上偏移的内圆，
/// 两圆求交后采样两段弧拼成闭合多边形（Mesh 三角扇填充，半透明底上也不会穿帮）。
fn draw_moon(p: &egui::Painter, c: egui::Pos2, r: f32, col: egui::Color32) {
    let dx = 0.62 * r;
    let dy = -0.62 * r;
    let c1 = egui::pos2(c.x + dx, c.y + dy);
    let r1 = 0.85 * r;
    let d = (dx * dx + dy * dy).sqrt();
    let a = (d * d + r * r - r1 * r1) / (2.0 * d);
    let h2 = r * r - a * a;
    if h2 <= 0.0 {
        p.circle_filled(c, r, col);
        return;
    }
    let h = h2.sqrt();
    let (ux, uy) = (dx / d, dy / d);
    // 两交点
    let pa = egui::pos2(c.x + a * ux - h * uy, c.y + a * uy + h * ux);
    let pb = egui::pos2(c.x + a * ux + h * uy, c.y + a * uy - h * ux);
    let ang = |o: egui::Pos2, q: egui::Pos2| (q.y - o.y).atan2(q.x - o.x);
    let norm = |mut t: f32| {
        while t < 0.0 {
            t += std::f32::consts::TAU;
        }
        while t >= std::f32::consts::TAU {
            t -= std::f32::consts::TAU;
        }
        t
    };
    // 外弧：从 pa 到 pb，走远离内圆的那一侧（经过方向 -u）
    let far = ang(c, egui::pos2(c.x - ux, c.y - uy));
    let a0 = ang(c, pa);
    let a1 = ang(c, pb);
    let ccw = norm(a1 - a0);
    let sweep = if norm(far - a0) <= ccw { ccw } else { ccw - std::f32::consts::TAU };
    let mut pts: Vec<egui::Pos2> = Vec::new();
    let n = 20;
    for i in 0..=n {
        let t = a0 + sweep * (i as f32) / (n as f32);
        pts.push(egui::pos2(c.x + r * t.cos(), c.y + r * t.sin()));
    }
    // 内弧：从 pb 回到 pa，走靠近外圆心的那一侧（经过方向 c1→c = -u）
    let near = ang(c1, c);
    let b0 = ang(c1, pb);
    let b1 = ang(c1, pa);
    let ccw2 = norm(b1 - b0);
    let sweep2 = if norm(near - b0) <= ccw2 { ccw2 } else { ccw2 - std::f32::consts::TAU };
    for i in 0..=n {
        let t = b0 + sweep2 * (i as f32) / (n as f32);
        pts.push(egui::pos2(c1.x + r1 * t.cos(), c1.y + r1 * t.sin()));
    }
    let mut mesh = egui::Mesh::default();
    // 手动铺三角扇（egui 0.27 的 Mesh 没有 add_triangle_fan）
    let base = mesh.vertices.len() as u32;
    for &q in &pts {
        mesh.colored_vertex(q, col);
    }
    for i in 1..pts.len() - 1 {
        mesh.add_triangle(base, base + i as u32, base + i as u32 + 1);
    }
    p.add(egui::Shape::mesh(mesh));
}

/// 主题切换胶囊：日/夜两段式（iOS 分段控件风）。
/// 返回 Some(是否切到夜色) 表示用户点了切换。
fn theme_capsule(ui: &mut egui::Ui, dark: bool) -> Option<bool> {
    let seg_w = 26.0_f32;
    let h = 20.0_f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(seg_w * 2.0 + 4.0, h), egui::Sense::hover());
    let p = ui.painter();
    // 轨道：必须比卡片底更「实」一点（浅灰），白色滑块才浮得起来。
    // 之前轨道直接用 button_fill()：日间是白 185，白色滑块叠上去等于隐形 —— 看起来根本不像开关。
    let track = if is_dark() {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20)
    } else {
        egui::Color32::from_rgba_unmultiplied(26, 36, 62, 28)
    };
    p.rect_filled(rect, h * 0.5, track);
    p.rect_stroke(rect, h * 0.5, egui::Stroke::new(1.0_f32, glass_stroke()));
    // 激活段指示器（圆角滑块）
    let seg = |i: usize| {
        egui::Rect::from_min_size(
            rect.min + egui::vec2(2.0 + i as f32 * seg_w, 2.0),
            egui::vec2(seg_w, h - 4.0),
        )
    };
    // 滑块补间：t 从 0（日间）到 1（夜色）平滑过渡，滑动动效就靠它
    let t = ui
        .ctx()
        .animate_value_with_time(ui.id().with("theme_knob"), if dark { 1.0 } else { 0.0 }, 0.16);
    let a = seg(0);
    let b = seg(1);
    let knob_rect = egui::Rect::from_min_max(
        egui::pos2(egui::lerp(a.min.x..=b.min.x, t), a.min.y),
        egui::pos2(egui::lerp(a.max.x..=b.max.x, t), a.max.y),
    );
    // 滑块：日间=纯白 + 两层递减黑投影（浮起的手感，iOS 分段控件同款做法）；
    // 夜间=微亮白。颜色随 t 连续插值，滑动过程中不会突变。
    let knob_r = (h - 4.0) * 0.5;
    let shadow_k = 1.0 - ease(t);
    if shadow_k > 0.01 {
        p.rect_filled(
            knob_rect.translate(egui::vec2(0.0, 1.6)).expand2(egui::vec2(1.2, 1.2)),
            knob_r,
            egui::Color32::from_black_alpha((30.0 * shadow_k) as u8),
        );
        p.rect_filled(
            knob_rect.translate(egui::vec2(0.0, 0.6)).expand2(egui::vec2(0.6, 0.6)),
            knob_r,
            egui::Color32::from_black_alpha((24.0 * shadow_k) as u8),
        );
    }
    // 颜色插值走全局 mixc（Color32 逐通道线性混合）
    let knob = mixc(
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 255),
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 42),
        t,
    );
    p.rect_filled(knob_rect, knob_r, knob);
    // 两段图标：左=太阳（日间），右=月亮（夜色）。
    // 激活色用主题主色，未激活色必须是「真·浅灰」——用 text_sub 灰蓝太深，
    // 和主色拉不开差距，看起来像两个都激活了。颜色随滑块 t 连续淡变。
    let active_c = text_main();
    let inactive_c = if is_dark() {
        egui::Color32::from_rgba_unmultiplied(140, 150, 170, 120)
    } else {
        egui::Color32::from_rgba_unmultiplied(172, 180, 196, 255)
    };
    for (i, is_sun) in [(0usize, true), (1usize, false)] {
        let r = seg(i);
        let c = r.center();
        // t=0 日间：太阳亮月亮灰；t=1 夜色：反之
        let col = if is_sun {
            mixc(active_c, inactive_c, t)
        } else {
            mixc(inactive_c, active_c, t)
        };
        if is_sun {
            draw_sun(p, c, 5.2, col);
        } else {
            draw_moon(p, c, 5.4, col);
        }
    }
    // 命中：两段各自的透明交互面
    let resp_sun = ui
        .interact(seg(0), ui.id().with("theme_sun"), egui::Sense::click())
        .clicked();
    let resp_moon = ui
        .interact(seg(1), ui.id().with("theme_moon"), egui::Sense::click())
        .clicked();
    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    if resp_sun {
        Some(false) // 切到日间
    } else if resp_moon {
        Some(true) // 切到夜色
    } else {
        None
    }
}

/// 细进度条（只读展示：白轨道 + 主色填充），宽度吃满所在容器
fn thin_progress(ui: &mut egui::Ui, progress: u8, color: egui::Color32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 6.0), egui::Sense::hover());
    let p = ui.painter();
    let track_c = if is_dark() {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26)
    } else {
        egui::Color32::from_rgba_unmultiplied(20, 30, 60, 22)
    };
    p.rect_filled(rect, 3.0, track_c);
    let fw = rect.width() * progress.min(100) as f32 / 100.0;
    if fw > 1.0 {
        p.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(fw, rect.height())),
            3.0,
            color,
        );
    }
}

/// 属性表的一行：定宽键列（60px）+ 值列。
/// 不用 `egui::Grid` —— Grid 首帧是【故意不可见】的（egui grid.rs:
/// `ui.set_visible(prev_state.is_some())`，为了避开首帧列宽抖动），需要第二帧才画出来。
/// 而 egui 是反应式的（没人 request_repaint 就不重绘），结果就是「表格空白一拍才出现」。
/// 自己排版没这个问题，也少一次布局往返。
fn prop_row(ui: &mut egui::Ui, key: &str, add_value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(60.0, 20.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(key).color(text_sub()).size(13.0));
            },
        );
        add_value(ui);
    });
    ui.add_space(6.0);
}

// ---- 三层虚化背景 ----// 用模糊 + 遮罩烘进纹理 alpha，模拟「氛围底 + 清晰主视觉 + 原图封面」的层次：
//   · 模糊层：高斯模糊（半径约 10）把背景图柔化；两层画面做 0.5s 交叉淡入避免切换突兀
//   · 压暗层：一层半透明暗色叠加，保证前景面板文字对比度
//   · 快速缩放：低质快缩放模式本身也带一点柔化，省算力
//  背景图取横版英雄图（宽幅美术图），封面取竖版封面：
//   · 背景铺满：等比铺满、横向居中、纵向顶部对齐
//   · 顶部径向遮罩：中心在顶边中点、横向很宽、0.3 以内实心 → 1.0 全透
//   · 封面：右侧停靠、底对齐、清晰不糊
//   · 顶部留白专门用来露背景图
//   背景来源：横版英雄图（library_hero.jpg 等宽幅美术图），封面取竖版封面图；
//   关键：铺的是「横版英雄图」而非竖版封面——竖版封面铺满宽窗口只能裁成一条再糊掉
//
// 落到 egui（没有 Effect，等价做法是把模糊和遮罩烘进纹理的 alpha）：
//   L1 整窗氛围底：横版图 → 缩到 512 → 强高斯模糊 → 顶部径向遮罩 → 铺满整窗 + 压暗
//   L2 详情主视觉：同一张图 → 缩到 900 → 只轻模糊（明显更清楚）→ 柔和径向遮罩 → 叠在 L1 上，
//                  边缘化开、没有卡片硬边，这就是「第二层更清晰」的那层
//   L3 封面：原图不处理，清晰展示（详情页头部右侧 + 列表缩略图）

/// 背景遮罩类型
#[derive(Clone, Copy, PartialEq)]
enum BackdropMask {
    /// 顶部径向遮罩：中心在顶边中点、横向很宽、0.3 以内实心 → 1.0 全透
    RadialTop,
    /// 柔和的径向「聚光」：中心 (cu,cv)、半径 (rx,ry) 都是**屏幕归一化**坐标，
    /// 靠一圈 45% 的过渡带化到透明，所以叠上去不会出现卡片硬边。
    /// 用它叠出「详情区比整窗更清晰」的那第二层。
    Spot { cu: f32, cv: f32, rx: f32, ry: f32 },
}

/// 等比铺满目标区域所需的 UV 子矩形（横向居中、纵向顶部对齐）
fn cover_uv(size: [usize; 2], area: egui::Vec2) -> egui::Rect {
    let ta = size[0] as f32 / size[1].max(1) as f32;
    let aa = area.x / area.y.max(1.0);
    if aa > ta {
        // 区域比图更宽：宽度铺满，纵向取顶部一段
        let f = (ta / aa).clamp(0.0, 1.0);
        egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, f))
    } else {
        // 区域比图更高：纵向铺满，横向居中裁切
        let f = (aa / ta).clamp(0.0, 1.0);
        let x = (1.0 - f) * 0.5;
        egui::Rect::from_min_max(pos2(x, 0.0), pos2(1.0 - x, 1.0))
    }
}

/// 生成一层背景纹理：缩小 → 高斯模糊 → 烘焙 alpha 遮罩
/// 纹理尺寸决定「视觉模糊量」：缩得越小、放大后越糊。之前缩到 128 再 blur(3.0)，
/// 放大到整屏等效于几十像素的糊 —— 这就是上一版整块变成一坨橙色色斑的原因。
/// 现在按显示尺寸给足分辨率，模糊半径才是真正的「视觉模糊量」。
///
/// `area_aspect` 是目标区域的宽高比：给了它，Spot 遮罩就按屏幕坐标算（会抵消 cover 裁切带来的放大），
/// 不给就按纹理坐标算。
fn backdrop_tex(
    ctx: &egui::Context,
    library_dir: &Path,
    textures: &mut std::collections::HashMap<String, egui::TextureHandle>,
    src: &str,
    max_edge: u32,
    blur: f32,
    mask: BackdropMask,
    area_aspect: Option<f32>,
) -> Option<egui::TextureHandle> {
    let tag = match mask {
        BackdropMask::RadialTop => "rt".to_string(),
        BackdropMask::Spot { cu, cv, rx, ry } => {
            format!("sp{cu:.2}-{cv:.2}-{rx:.2}-{ry:.2}")
        }
    };
    let aa_key = area_aspect.map(|a| (a * 20.0).round() / 20.0).unwrap_or(0.0);
    let key = format!("__bg:{src}|{max_edge}|{:.0}|{tag}|{aa_key:.2}", blur * 10.0);
    if let Some(t) = textures.get(&key) {
        return Some(t.clone());
    }
    let img = image::open(library_dir.join(src)).ok()?;
    let (w0, h0) = (img.width().max(2), img.height().max(2));
    let scale = (max_edge as f32 / w0.max(h0) as f32).min(1.0);
    let w = ((w0 as f32 * scale).round() as u32).max(8);
    let h = ((h0 as f32 * scale).round() as u32).max(8);
    let small =
        image::imageops::resize(&img.to_rgba8(), w, h, image::imageops::FilterType::Triangle);
    let mut out = if blur > 0.05 { image::imageops::blur(&small, blur) } else { small };
    // 纹理坐标 → 屏幕归一化坐标（抵消 cover 裁切）
    let map = area_aspect.map(|aa| {
        let r = cover_uv([w0 as usize, h0 as usize], egui::vec2(aa, 1.0));
        (r.min.x, r.width().max(1e-4), r.min.y, r.height().max(1e-4))
    });
    let fw = (w as f32 - 1.0).max(1.0);
    let fh = (h as f32 - 1.0).max(1.0);
    for (x, y, px) in out.enumerate_pixels_mut() {
        let u = x as f32 / fw;
        let v = y as f32 / fh;
        let a = match mask {
            BackdropMask::RadialTop => {
                let dx = (u - 0.5) / 2.0;
                let dy = v / 0.9;
                ((1.0 - (dx * dx + dy * dy).sqrt()) / 0.7).clamp(0.0, 1.0)
            }
            BackdropMask::Spot { cu, cv, rx, ry } => {
                // Spot 按屏幕归一化给；有裁切信息就换算过去，否则按纹理归一化
                let (su, sv) = match map {
                    Some((u0, uw, v0, vh)) => ((u - u0) / uw, (v - v0) / vh),
                    None => (u, v),
                };
                let dx = (su - cu) / rx.max(0.01);
                let dy = (sv - cv) / ry.max(0.01);
                // 55% 半径以内保持实心，再往外 45% 化到全透
                ((1.0 - (dx * dx + dy * dy).sqrt()) / 0.45).clamp(0.0, 1.0)
            }
        };
        let a = a * a * (3.0 - 2.0 * a); // smoothstep，边缘过渡更自然
        px[3] = (a * 255.0) as u8;
    }
    let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], out.as_raw());
    let t = ctx.load_texture(key.clone(), ci, egui::TextureOptions::LINEAR);
    textures.insert(key, t.clone());
    Some(t)
}

/// 画一个带纹理的圆角矩形（egui 的 RectShape 支持 fill_texture_id + uv，Image 控件内部就是这么画的）
fn tex_round_rect(
    p: &egui::Painter,
    rect: egui::Rect,
    tex: &egui::TextureHandle,
    uv: egui::Rect,
    rounding: egui::Rounding,
    tint: egui::Color32,
) {
    let mut s = egui::epaint::RectShape::filled(rect, rounding, tint);
    s.fill_texture_id = tex.id();
    s.uv = uv;
    p.add(s);
}

/// 竖直渐变遮罩（上浅下深，保证面板文字对比度）
fn vgradient(p: &egui::Painter, rect: egui::Rect, top: egui::Color32, bottom: egui::Color32) {
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    p.add(egui::Shape::mesh(mesh));
}

/// 详情顶部操作栏的按钮点击结果
#[derive(Default)]
struct DetailAct {
    edit: bool,
    save: bool,
    cancel: bool,
    launch: bool,
    /// 当前游戏正在运行，用户点了「运行中」按钮要求强制关闭
    stop: bool,
    open_dir: bool,
}

/// 详情操作按钮组。
/// 注意：调用方若是 right_to_left 布局，先添加的按钮显示在更右侧。
fn detail_action_buttons(
    ui: &mut egui::Ui,
    editing: bool,
    has_exe: bool,
    has_dir: bool,
    running: bool,
) -> DetailAct {
    let mut a = DetailAct::default();

    if editing {
        if primary_button(ui, "保存").clicked() {
            a.save = true;
        }
        if ui.button("取消").clicked() {
            a.cancel = true;
        }
        ui.add_space(4.0);
    } else if ui.button("编辑").clicked() {
        a.edit = true;
    }

    let mut dir_btn = ui.button("打开目录");
    if !has_dir {
        dir_btn = dir_btn.on_hover_text("该条目未记录文件夹路径");
    }
    if dir_btn.clicked() {
        a.open_dir = true;
    }

    if running {
        // 正在运行：按钮顶替成绿色「运行中」，点击强制关闭
        if running_button(ui).clicked() {
            a.stop = true;
        }
    } else {
        // 编辑态下「保存」已经是主色按钮，启动就不再抢主色，避免两个蓝按钮打架
        let mut launch_btn = if editing {
            ui.button("▶ 启动游戏")
        } else {
            primary_button(ui, "▶ 启动游戏")
        };
        if !has_exe {
            launch_btn = launch_btn
                .on_hover_text("该条目未记录可执行文件（可右键左侧卡片「重命名并重新匹配」）");
        }
        if launch_btn.clicked() {
            a.launch = true;
        }
    }

    a
}

/// 详情页只读展示用的数据快照（避免在闭包里同时持有 &self.games 与 &mut self.edit_*）
struct InfoSnap {
    progress: u8,
    rating: u8,
    playtime: f32,
    /// 当前正在运行的会话已耗时（秒）；0 = 没在运行
    running_secs: f32,
    note: String,
}

/// 时长的人话格式：库里存的是小时（f32），但「0.02 小时」没人看得懂
fn fmt_playtime(hours: f32) -> String {
    if hours <= 0.0 {
        return "未记录".to_string();
    }
    let total_min = (hours * 60.0).round() as u64;
    if total_min == 0 {
        return "不足 1 分钟".to_string();
    }
    if total_min < 60 {
        return format!("{total_min} 分钟");
    }
    let (h, m) = (total_min / 60, total_min % 60);
    if m == 0 {
        format!("{h} 小时")
    } else {
        format!("{h} 小时 {m} 分钟")
    }
}

/// 显示名：别名 > Steam 匹配名 > 原始名
/// （扫描名常是「Enigma.of.Fear.Build.25414」这种垃圾名，匹配成功后应展示 Steam 正名）
fn display_name(g: &GameEntry) -> &str {
    if let Some(a) = g.alias.as_deref() {
        if !a.trim().is_empty() {
            return a;
        }
    }
    if let Some(m) = g.matched_name.as_deref() {
        if !m.trim().is_empty() {
            return m;
        }
    }
    &g.name
}

/// 应用玻璃主题（随主题胶囊切换：夜色 / 日间）
fn apply_theme(ctx: &egui::Context) {
    // 必须从纯色主题出发，不能克隆当前主题再改 dark_mode：
    // dark_mode 只是个标志位，不会重算已固化的底色字段。
    let mut v = if is_dark() {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    // 全局兜底：任何没显式指定颜色的文字走主题主色，杜绝「浅底配浅字」
    v.override_text_color = Some(text_main());
    // 面板底：低不透明度，保持通透
    v.panel_fill = if is_dark() {
        egui::Color32::from_rgba_unmultiplied(14, 16, 24, 60)
    } else {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 44)
    };
    v.window_fill = if is_dark() {
        egui::Color32::from_rgba_unmultiplied(24, 26, 36, 200)
    } else {
        egui::Color32::from_rgba_unmultiplied(252, 253, 255, 235)
    };
    v.extreme_bg_color = if is_dark() {
        egui::Color32::from_rgba_unmultiplied(10, 12, 18, 130)
    } else {
        egui::Color32::from_rgba_unmultiplied(238, 242, 252, 110)
    };
    // 控件（按钮/下拉/输入框）：玻璃底 + 主题字色，绝不许浅底配浅字
    v.widgets.noninteractive.bg_fill = if is_dark() {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 8)
    } else {
        egui::Color32::from_rgba_unmultiplied(20, 30, 60, 8)
    };
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, text_sub());
    // 分隔线（面板拖拽分隔条 / ui.separator）走 bg_stroke：默认的灰白太抢眼，
    // 会把「整窗一张连贯背景」硬生生切成左右两块，改成极淡的主题色
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(
        1.0_f32,
        if is_dark() {
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26)
        } else {
            egui::Color32::from_rgba_unmultiplied(20, 30, 60, 28)
        },
    );
    v.widgets.inactive.bg_fill = button_fill();
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.4_f32, text_main());
    v.widgets.hovered.bg_fill = button_fill_hover();
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.4_f32, text_main());
    v.widgets.active.bg_fill = button_fill_active();
    v.widgets.active.fg_stroke = egui::Stroke::new(1.4_f32, text_main());
    v.widgets.open.bg_fill = button_fill();
    v.widgets.open.fg_stroke = egui::Stroke::new(1.4_f32, text_main());
    // 关键：Button / ComboBox / 弹窗标题栏的默认底色走的是 weak_bg_fill，不是 bg_fill。
    // 只改 bg_fill 的话，这些控件会保留 egui 默认主题的白灰/深灰底，配字直接吞掉。
    v.widgets.noninteractive.weak_bg_fill = if is_dark() {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 10)
    } else {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 80)
    };
    v.widgets.inactive.weak_bg_fill = button_fill();
    v.widgets.hovered.weak_bg_fill = button_fill_hover();
    v.widgets.active.weak_bg_fill = button_fill_active();
    v.widgets.open.weak_bg_fill = button_fill();
    v.widgets.inactive.rounding = egui::Rounding::same(10.0);
    v.widgets.hovered.rounding = egui::Rounding::same(10.0);
    v.widgets.active.rounding = egui::Rounding::same(10.0);
    v.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(10, 132, 255, 80);
    v.selection.stroke = egui::Stroke::new(1.2_f32, egui::Color32::from_rgb(120, 180, 255));
    let mut s = (*ctx.style()).clone();
    s.visuals = v;
    s.spacing.item_spacing = egui::vec2(10.0_f32, 10.0_f32);
    s.spacing.window_margin = egui::Margin::same(14.0_f32);
    s.spacing.button_padding = egui::vec2(14.0_f32, 7.0_f32);
    // SF 排版层级：大标题醒目、正文清爽
    s.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(24.0_f32),
    );
    s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(15.0_f32));
    s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(15.0_f32));
    s.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.0_f32));
    ctx.set_style(s);
}

/// 加载系统中文字体（egui 默认字体不含 CJK 字形，中文会显示为豆腐块）
fn install_cjk_fonts(ctx: &egui::Context) {
    let candidates: &[&str] = &[
        r"C:\Windows\Fonts\msyh.ttc",     // 微软雅黑
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\msyhbd.ttc",   // 雅黑粗体（兜底）
        r"C:\Windows\Fonts\simhei.ttf",   // 黑体
        r"C:\Windows\Fonts\simsun.ttc",   // 宋体
        "/System/Library/Fonts/PingFang.ttc", // macOS
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", // Linux
    ];
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("cjk".into(), egui::FontData::from_owned(data).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".into());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("cjk".into());
            ctx.set_fonts(fonts);
            return;
        }
    }
    eprintln!("未找到系统中文字体，界面中文可能显示异常");
}

/// 内嵌的应用图标（assets/icon.png，编译期打包进二进制）
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// 解码应用图标为窗口图标数据（任务栏 / alt-tab 用）
fn app_icon_raw() -> egui::IconData {
    let img = image::load_from_memory(APP_ICON_PNG)
        .expect("内置应用图标解码失败")
        .into_rgba8();
    let (w, h) = img.dimensions();
    egui::IconData { width: w, height: h, rgba: img.into_raw() }
}

pub fn run() -> anyhow::Result<()> {
    // 临时调试：GAME_CABINET_WIN=780x560 以指定逻辑尺寸启动（验证小窗口布局用）
    let mut inner = egui::vec2(1080.0_f32, 740.0_f32);
    if let Ok(v) = std::env::var("GAME_CABINET_WIN") {
        let mut it = v.split(['x', 'X']);
        if let (Some(a), Some(b)) = (it.next(), it.next()) {
            if let (Ok(w), Ok(h)) = (a.parse::<f32>(), b.parse::<f32>()) {
                inner = egui::vec2(w, h);
            }
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(inner)
            .with_min_inner_size(egui::vec2(560.0_f32, 420.0_f32))
            // 窗口真透明：系统亚克力模糊透出桌面（配合 window-vibrancy）
            .with_transparent(true)
            // 去掉系统原生标题栏：改由 egui 自绘玻璃顶栏（拖拽 + 最小/最大/关闭）
            .with_decorations(false)
            // 允许缩放（最大化/还原；无边框下系统不提供边缘拖拽手柄，靠最大化按钮和本机快捷键）
            .with_resizable(true)
            // 任务栏 / 窗口图标（alt-tab、任务栏都显示它）
            .with_icon(app_icon_raw()),
        ..Default::default()
    };
    eframe::run_native(
        "游戏柜 · game-cabinet",
        options,
        Box::new(|cc: &eframe::CreationContext| -> Box<dyn eframe::App> {
            apply_theme(&cc.egui_ctx);
            install_cjk_fonts(&cc.egui_ctx);
            Box::new(CabinetApp::default())
        }),
    )
    .map_err(|e| anyhow::anyhow!("{:?}", e))?;
    Ok(())
}

struct CabinetApp {
    library_path: PathBuf,
    library_dir: PathBuf,
    games: Vec<GameEntry>,
    selected: Option<usize>,
    textures: std::collections::HashMap<String, egui::TextureHandle>,
    placeholder: Option<egui::TextureHandle>,
    status: String,
    scan_rx: Option<Receiver<ScanMsg>>,
    rematch_rx: Option<Receiver<RematchMsg>>,
    vibrancy_done: bool,
    /// 主题：true=夜色（默认），false=日间（顶栏胶囊切换）
    dark: bool,
    /// 顶栏左上角的应用图标纹理（首次用到时从内嵌 PNG 懒加载）
    icon_tex: Option<egui::TextureHandle>,
    /// 窗口是否处于最大化态（自绘标题栏的还原/最大化按钮用，自己跟踪避免依赖平台 API）
    maximized: bool,
    /// L1 整窗氛围底：横版英雄图强模糊 + 顶部径向遮罩
    bg_wide: Option<egui::TextureHandle>,
    /// L2 详情主视觉：同一张图轻模糊 + 柔和径向遮罩（比 L1 明显清晰）
    bg_detail: Option<egui::TextureHandle>,
    /// 当前背景源（本地相对路径）；变化时重建两层 + 触发交叉淡入
    bg_src: Option<String>,
    /// 切换游戏时的上一组背景（交叉淡入用）
    bg_wide_prev: Option<egui::TextureHandle>,
    bg_detail_prev: Option<egui::TextureHandle>,
    bg_fade: f32,
    bg_fading: bool,
    /// 变更后还需要主动补渲染的帧数（egui 反应式：没人叫它就不画）
    settle: u8,
    /// 英雄图懒加载：已在飞行的请求 / 试过但没有结果的 appid（避免反复重试）
    hero_rx: Option<Receiver<Option<HeroMsg>>>,
    hero_tried: std::collections::HashSet<u32>,
    /// 封面平均色缓存（列表卡片做毛玻璃底用）
    avg: std::collections::HashMap<String, egui::Color32>,
    // 编辑缓冲
    edit_progress: u8,
    edit_rating: u8,
    edit_playtime: String,
    edit_note: String,
    /// 详情页是否处于编辑态（默认只读，点「编辑」才进表单，避免误改）
    editing: bool,
    // 右键弹窗：重命名并重新匹配 / 设置别名
    rename_idx: Option<usize>,
    rename_buf: String,
    alias_idx: Option<usize>,
    alias_buf: String,
    // ---- 动效计时器（秒；全部是「一次性入场」用的一次性进度）----
    /// 详情内容入场进度 0→1（切换游戏时重置，配合背景交叉淡入）
    detail_t: f32,
    /// 上一帧的选中项：用来检测「切换了游戏」以重置入场进度
    detail_key: Option<usize>,
    /// 列表错峰入场计时（列表首次出现 / 扫描结束后重新计时）
    list_t: f32,
    /// 空状态欢迎卡入场进度
    intro_t: f32,
    /// 扫描完成提示的剩余时长（>0 时展示对勾 + 文字，之后淡出）
    scan_done_t: f32,
    /// 「库」下拉菜单是否展开（自管理，为了能做补间展开动画）
    lib_menu: bool,
    /// 正在运行的游戏会话（启动时记下子进程句柄与起始时刻，退出时把时长累加进库）
    running: Vec<RunningGame>,
}

/// 一次「启动游戏」会话。
///
/// 为什么要持着 `Child` 不放：只有留着句柄才能 `try_wait()` 知道它什么时候退出，
/// 从而算出本次玩了多久。句柄本身不阻止进程运行，也不影响游戏。
struct RunningGame {
    /// 列表下标（仅作快速路径，定位仍以 `title` 为准，避免期间扫描/重排导致错位）
    idx: usize,
    /// 显示名：用来在条目里重新定位
    title: String,
    child: std::process::Child,
    started: std::time::Instant,
    /// 已经写进 library.json 的秒数。
    /// 时长按分钟增量回写（而不是等退出才写一次），这样即使程序被强杀，最多只丢 1 分钟。
    flushed_secs: f32,
}

impl Default for CabinetApp {
    fn default() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let lib_path = cwd.join("library.json");
        let lib = crate::library::Library::load(&lib_path);
        let dir = lib_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| cwd.clone());
        CabinetApp {
            library_path: lib_path,
            library_dir: dir,
            games: lib.games,
            selected: None,
            textures: Default::default(),
            placeholder: None,
            status: String::new(),
            scan_rx: None,
            rematch_rx: None,
            vibrancy_done: false,
            dark: true,
            icon_tex: None,
            maximized: false,
            bg_wide: None,
            bg_detail: None,
            bg_src: None,
            bg_wide_prev: None,
            bg_detail_prev: None,
            bg_fade: 1.0,
            bg_fading: false,
            settle: 0,
            hero_rx: None,
            hero_tried: Default::default(),
            avg: Default::default(),
            edit_progress: 0,
            edit_rating: 0,
            edit_playtime: String::new(),
            edit_note: String::new(),
            editing: false,
            rename_idx: None,
            rename_buf: String::new(),
            alias_idx: None,
            alias_buf: String::new(),
            detail_t: 1.0,
            detail_key: None,
            list_t: 0.0,
            intro_t: 0.0,
            scan_done_t: 0.0,
            lib_menu: false,
            running: Vec::new(),
        }
    }
}

impl CabinetApp {
    /// 选中游戏时，把它的字段载入编辑缓冲
    fn load_edit_buffer(&mut self, i: usize) {
        if let Some(g) = self.games.get(i) {
            self.edit_progress = g.progress;
            self.edit_rating = g.rating;
            self.edit_playtime = format!("{}", g.playtime_hours);
            self.edit_note = g.note.clone();
        }
    }

    /// 把编辑缓冲写回选中的游戏并保存
    fn save_selected(&mut self) {
        if let Some(i) = self.selected {
            if let Some(g) = self.games.get_mut(i) {
                g.progress = self.edit_progress;
                g.rating = self.edit_rating;
                g.playtime_hours = self.edit_playtime.trim().parse().unwrap_or(0.0);
                g.note = self.edit_note.clone();
            }
        }
        let lib = crate::library::Library {
            games: self.games.clone(),
        };
        match lib.save(&self.library_path) {
            Ok(()) => self.status = "已保存".into(),
            Err(e) => self.status = format!("保存失败: {e}"),
        }
    }

    /// 按给定名称后台重新匹配：搜 Steam → 取第一个候选 → 下载封面
    fn start_rematch(&mut self, idx: usize, term: String) {
        if term.is_empty() {
            self.status = "名称为空，无法匹配".into();
            return;
        }
        // 这个游戏的图放哪：自包含的游戏文件夹内 .cabinet；散落 exe 才回落 covers
        let art_rel = self
            .games
            .get(idx)
            .map(|g| crate::library::art_dir(&self.library_dir, g))
            .unwrap_or_else(|| PathBuf::from("covers"));
        let in_cabinet = art_rel != Path::new("covers");
        let dir = self.library_dir.clone();
        let key = std::env::var("STEAMGRIDDB_API_KEY").unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        self.rematch_rx = Some(rx);
        self.status = format!("正在为「{term}」搜索 Steam…");
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let res: anyhow::Result<RematchMsg> = rt
                .block_on(async {
                    let client = reqwest::Client::builder()
                        .user_agent("game-cabinet/0.1 (local game manager)")
                        .timeout(std::time::Duration::from_secs(15))
                        .build()?;
                    let mut cans = crate::steam::search_steam(&term, &client, 6).await?;
                    if cans.is_empty() && !key.is_empty() {
                        cans = crate::steam::search_sgdb(&term, &key, &client, 6).await?;
                    }
                    let c = cans
                        .into_iter()
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Steam 没搜到「{term}」，换个名字试试"))?;
                    let (cover, header) =
                        crate::steam::resolve_cover(c.source, c.id, &key, &client).await?;
                    let out = if in_cabinet { dir.join(&art_rel) } else { dir.join("covers") };
                    std::fs::create_dir_all(&out)?;
                    let file_name = if in_cabinet {
                        "cover.jpg".to_string()
                    } else {
                        let base = if c.source == "sgdb" {
                            format!("sgdb_{}", c.id)
                        } else {
                            c.id.to_string()
                        };
                        format!("{base}_cover.jpg")
                    };
                    let cover_path = out.join(&file_name);
                    let mut ok =
                        crate::steam::download_cover(&client, &cover, &cover_path).await.is_ok();
                    if !ok {
                        ok = crate::steam::download_cover(&client, &header, &cover_path)
                            .await
                            .is_ok();
                    }
                    Ok(RematchMsg {
                        idx,
                        new_name: term.clone(),
                        appid: if c.source == "steam" { Some(c.id) } else { None },
                        matched: Some(c.name),
                        // 无论图落在 .cabinet 还是 covers，都记「相对库根」的路径 ——
                        // 加载端 library_dir.join(cover_local) 因此完全不用改
                        cover_local: if ok {
                            cover_path
                                .strip_prefix(&dir)
                                .ok()
                                .map(|p| p.to_string_lossy().replace('\\', "/"))
                        } else {
                            None
                        },
                        cover_url: Some(cover),
                        header_url: Some(header),
                        error: if ok { None } else { Some("封面下载失败".into()) },
                    })
                });
            let msg: RematchMsg = res.unwrap_or_else(|e: anyhow::Error| RematchMsg {
                idx,
                new_name: String::new(),
                appid: None,
                matched: None,
                cover_local: None,
                cover_url: None,
                header_url: None,
                error: Some(e.to_string()),
            });
            let _ = tx.send(msg);
        });
    }

    /// 选择游戏文件夹，后台线程扫描 + 抓封面
    fn start_scan(&mut self, dir: PathBuf) {
        let key = std::env::var("STEAMGRIDDB_API_KEY").unwrap_or_default();
        let covers = dir.join("covers");
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.library_dir = dir.clone();
        self.library_path = dir.join("library.json");
        self.status = "扫描中…".into();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            match rt.block_on(crate::scan_and_fetch(&dir, Some(covers.clone()), 4, &key)) {
                Ok(lib) => {
                    let _ = tx.send(ScanMsg {
                        library: lib,
                        dir: dir.clone(),
                    });
                }
                Err(e) => eprintln!("扫描失败: {e}"),
            }
        });
    }

    /// 打开已有的库文件
    fn open_library(&mut self, p: PathBuf) {
        let lib = crate::library::Library::load(&p);
        if let Some(parent) = p.parent() {
            self.library_dir = parent.to_path_buf();
        }
        self.library_path = p;
        self.games = lib.games;
        self.selected = None;
        self.editing = false;
        self.textures.clear();
        self.status = format!("已打开库：{} 款", self.games.len());
    }

    /// 打开外部程序：工作目录设为 exe 所在文件夹（不少单机游戏靠相对路径读资源）
    ///
    /// 启动的同时记下会话：拿着子进程句柄，退出后自动把这局玩了多久累加进「时长」。
    fn launch_exe(&mut self, i: usize, title: &str) {
        // 已在运行就别重复启动：多开一般不是本意，而且两个会话会重复计时
        if self.running.iter().any(|r| r.title == title) {
            self.status = format!("「{title}」已在运行中");
            return;
        }
        match self.games.get(i).and_then(|g| g.executable.clone()) {
            Some(exe) => {
                let mut cmd = std::process::Command::new(&exe);
                if let Some(dir) = Path::new(&exe).parent() {
                    cmd.current_dir(dir);
                }
                match cmd.spawn() {
                    Ok(child) => {
                        self.running.push(RunningGame {
                            idx: i,
                            title: title.to_string(),
                            child,
                            started: std::time::Instant::now(),
                            flushed_secs: 0.0,
                        });
                        self.status = format!("已启动：{title}");
                    }
                    Err(e) => self.status = format!("启动失败：{e}"),
                }
            }
            None => {
                self.status = "该条目没有可执行文件，右键左侧卡片「重命名并重新匹配」试试".into();
            }
        }
    }

    /// 强制关闭正在运行的游戏（详情页「运行中」按钮），并立刻结算这一段时长
    fn stop_game(&mut self, title: &str) {
        if let Some(k) = self.running.iter().position(|r| r.title == title) {
            let mut r = self.running.remove(k);
            let _ = r.child.kill();
            let _ = r.child.wait(); // 回收进程句柄，避免杀掉后留下悬挂的子进程
            let elapsed = r.started.elapsed().as_secs_f32();
            // 与自然退出保持一致：清掉「已启动」提示、整局不足 1 分钟不计入
            self.status.clear();
            if elapsed >= 60.0 {
                let delta = elapsed - r.flushed_secs;
                self.add_playtime(r.idx, &r.title, delta);
            }
        }
    }

    /// 轮询正在运行的游戏：① 按分钟增量回写时长 ② 进程退出时结算本次会话
    ///
    /// 为什么放主线程而不是开个线程去 `wait()`：`try_wait()` 是非阻塞的，一次几微秒；
    /// 而结算要改 `self.games` 并落盘，放主线程省掉一整套跨线程同步。
    fn poll_running(&mut self, ctx: &egui::Context) {
        if self.running.is_empty() {
            return;
        }
        // 运行期间保持低频刷新：时长显示要跟着走，但没必要每帧
        ctx.request_repaint_after(std::time::Duration::from_millis(500));

        // ① 增量回写：每满 1 分钟把这一段写进库。
        //    这样即使程序被强杀 / 直接关机，最多只丢 1 分钟，而不是整段归零。
        //    （先在循环里收集，出了循环再写盘——循环持有 running 的可变借用，
        //      里面不能同时借 self 调别的 &mut self 方法）
        let mut pending: Vec<(usize, String, f32)> = Vec::new();
        for r in self.running.iter_mut() {
            let elapsed = r.started.elapsed().as_secs_f32();
            if elapsed >= 60.0 && elapsed - r.flushed_secs >= 60.0 {
                let delta = elapsed - r.flushed_secs;
                r.flushed_secs = elapsed;
                pending.push((r.idx, r.title.clone(), delta));
            }
        }
        for (idx, title, delta) in pending {
            self.add_playtime(idx, &title, delta);
        }

        // ② 找出已经退出的会话
        let mut ended: Vec<usize> = Vec::new();
        for (k, r) in self.running.iter_mut().enumerate() {
            let exited = match r.child.try_wait() {
                Ok(Some(_)) => true, // 正常退出
                Err(_) => true,      // 句柄失效，当作已退出
                Ok(None) => false,   // 还在跑
            };
            if exited {
                ended.push(k);
            }
        }
        for k in ended.into_iter().rev() {
            let r = self.running.remove(k);
            let elapsed = r.started.elapsed().as_secs_f32();
            // 会话结束：清掉先前的「已启动」提示，免得顶栏留着一条过时信息
            self.status.clear();
            // 整局不足 1 分钟：多半是误点、或启动器拉起主程序后自身秒退，不记入，免得污染时长
            if elapsed < 60.0 {
                continue;
            }
            let delta = elapsed - r.flushed_secs;
            self.add_playtime(r.idx, &r.title, delta);
        }
        ctx.request_repaint();
    }

    /// 把一段时长（秒）累加到某个游戏上并立即落盘。
    ///
    /// 定位策略：先看启动时记下的下标还对不对（名字也要对得上），不对再按显示名全库找。
    /// 这样即使中途扫描过、列表重排过，也不会把时长加到别人头上。
    fn add_playtime(&mut self, idx: usize, title: &str, secs: f32) {
        if secs < 1.0 {
            return;
        }
        let same = self
            .games
            .get(idx)
            .map(|g| display_name(g) == title)
            .unwrap_or(false);
        let idx = if same {
            idx
        } else {
            match self.games.iter().position(|g| display_name(g) == title) {
                Some(i) => i,
                None => return,
            }
        };
        self.games[idx].playtime_hours += secs / 3600.0;
        let lib = crate::library::Library {
            games: self.games.clone(),
        };
        if let Err(e) = lib.save(&self.library_path) {
            self.status = format!("时长保存失败：{e}");
        }
    }

    /// 选中游戏时后台补一张横版英雄图（Steam 的 library_hero.jpg）。
    /// 有了它背景才是宽幅美术图，而不是把竖版封面裁一条放大糊掉——这是背景好看的关键。
    ///
    /// 取图顺序：先「当前选中」那款，再按列表顺序把库里还缺英雄图的依次补上（预取），
    /// 这样点过去就有图，不会出现「先糊一版封面、过一会儿才换成英雄图」的迟缓感。
    /// 同一时刻只飞一个请求（串行，别拿几十个并发去打 CDN），失败不重试。
    fn maybe_fetch_hero(&mut self, ctx: &egui::Context) {
        if self.hero_rx.is_some() {
            return; // 已经有一个请求在飞
        }
        // 优先当前选中，其次按顺序扫全库，找到第一个「有 appid 但还没有英雄图」的条目
        let mut pick: Option<(usize, u32)> = None;
        let order = std::iter::once(self.selected)
            .flatten()
            .chain(0..self.games.len());
        for i in order {
            let g = match self.games.get(i) {
                Some(g) => g,
                None => continue,
            };
            let appid = match g.steam_appid {
                Some(a) => a,
                None => continue,
            };
            // 「当前选中」那款认真查一次磁盘：库搬过 / 图被删过时 hero_local 只是条失效路径，
            // 不能既画不出背景、又把兜底封面挡在外面。其余条目走便宜的判断（预取扫描每帧都跑，
            // 别给每个游戏都 stat 一次）
            let has_hero = if self.selected == Some(i) {
                g.hero_local.as_deref().map(|s| self.hero_ok(s)).unwrap_or(false)
            } else {
                g.hero_local
                    .as_deref()
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
            };
            if has_hero || self.hero_tried.contains(&appid) {
                continue;
            }
            pick = Some((i, appid));
            break;
        }
        let (i, appid) = match pick {
            Some(v) => v,
            None => return,
        };
        // 这个游戏的图放哪：自包含的游戏文件夹内 .cabinet；散落 exe 才回落 covers
        let art_rel = self
            .games
            .get(i)
            .map(|g| crate::library::art_dir(&self.library_dir, g))
            .unwrap_or_else(|| PathBuf::from("covers"));
        let in_cabinet = art_rel != Path::new("covers");
        self.hero_tried.insert(appid);
        let dir = self.library_dir.clone();
        let (tx, rx) = mpsc::channel();
        self.hero_rx = Some(rx);
        // 关键：后台线程干完必须主动把 UI 叫醒（request_repaint）。
        // egui 空闲时不重绘，异步结果又没人通知它 —— 这正是「背景慢一拍」的根因：
        // 图早就下好了，却要等用户下一次动鼠标/点击才被画出来。
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let res: Option<HeroMsg> = rt.block_on(async {
                let client = reqwest::Client::builder()
                    .user_agent("game-cabinet/0.1 (local game manager)")
                    .timeout(std::time::Duration::from_secs(20))
                    .build()
                    .ok()?;
                let url = crate::steam::resolve_hero(appid, &client).await?;
                let out = if in_cabinet { dir.join(&art_rel) } else { dir.join("covers") };
                std::fs::create_dir_all(&out).ok()?;
                let path = out.join(if in_cabinet {
                    "hero.jpg".to_string()
                } else {
                    format!("{appid}_hero.jpg")
                });
                crate::steam::download_cover(&client, &url, &path).await.ok()?;
                // 没有这张图时 CDN 会甩一个很小的占位/HTML 过来，按体积筛掉
                let sz = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                if sz < 15_000 {
                    let _ = std::fs::remove_file(&path);
                    return None;
                }
                Some(HeroMsg {
                    idx: i,
                    rel: path
                        .strip_prefix(&dir)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default(),
                })
            });
            let _ = tx.send(res);
            // 通知 UI：不管是拿到图还是确认没有，都立刻重绘，别让结果干等到下次输入
            ctx.request_repaint();
        });
    }

    /// 背景源（优先级）：
    /// hero_local 记的是相对路径，得确认文件真在磁盘上。
    /// 库被搬过、图被手工删过时它只是一条失效路径 —— 用它会既画不出背景、又把兜底封面挡住。
    fn hero_ok(&self, rel: &str) -> bool {
        let rel = rel.trim();
        !rel.is_empty() && self.library_dir.join(rel).exists()
    }

    ///   1. hero_local —— 横版英雄图（library_hero.jpg），详情页背景用的就是它
    ///   2. 同目录下已下载的横版头图 {appid}_header.jpg —— 老数据没记 hero_local 也能直接用
    ///   3. 竖版封面 —— 兜底。竖版图铺宽窗口只能裁中间一条，再糊就成一坨色斑，所以放最后
    fn backdrop_src(&self, i: usize) -> Option<String> {
        let g = self.games.get(i)?;
        if let Some(h) = g.hero_local.as_deref() {
            if self.hero_ok(h) {
                return Some(h.trim().to_string());
            }
        }
        if let Some(c) = g.cover_local.as_deref() {
            let p = Path::new(c);
            if let Some(parent) = p.parent() {
                // 新格式：图放在游戏文件夹的 .cabinet 里，头图固定叫 header.jpg
                let cand = parent.join("header.jpg");
                if self.library_dir.join(&cand).exists() {
                    return Some(cand.to_string_lossy().replace('\\', "/"));
                }
                // 旧格式：covers/ 下 {base}_header.jpg（老库照样认）
                if let Some(base) = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.strip_suffix("_cover"))
                {
                    let cand = parent.join(format!("{base}_header.jpg"));
                    if self.library_dir.join(&cand).exists() {
                        return Some(cand.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
            return Some(c.to_string());
        }
        None
    }

    /// 选中项变化时重建两层背景（旧图留着做交叉淡入）
    fn sync_window_backdrop(&mut self, ctx: &egui::Context) {
        let dir = self.library_dir.clone();
        let src = self.selected.and_then(|i| self.backdrop_src(i));
        if src == self.bg_src {
            return;
        }
        self.bg_wide_prev = self.bg_wide.take();
        self.bg_detail_prev = self.bg_detail.take();
        let has_prev = self.bg_wide_prev.is_some() || self.bg_detail_prev.is_some();
        self.bg_fade = if has_prev { 0.0 } else { 1.0 };
        self.bg_fading = has_prev;
        self.bg_src = src.clone();
        // 没上一张图时不会走交叉淡入，也就没人 request_repaint；这里补几帧兜底
        self.settle = self.settle.max(6);
        if let Some(s) = src.as_deref() {
            let screen = ctx.screen_rect();
            let aa = screen.width() / screen.height().max(1.0);
            // L1 整窗氛围底：缩到 512 + blur(9)，明显是「糊背景」，但构图还能看出来
            self.bg_wide = backdrop_tex(
                ctx,
                &dir,
                &mut self.textures,
                s,
                512,
                9.0,
                BackdropMask::RadialTop,
                None,
            );
            // L2 详情主视觉：缩到 900 只 blur(1.6)，画面基本清楚；聚光落在右侧详情区，
            // 左侧列表那半边让 L1 的糊底透上来 —— 两层靠柔和过渡衔接，没有卡片硬边
            self.bg_detail = backdrop_tex(
                ctx,
                &dir,
                &mut self.textures,
                s,
                900,
                1.6,
                BackdropMask::Spot {
                    cu: 0.66,
                    cv: 0.48,
                    rx: 0.54,
                    ry: 0.82,
                },
                Some(aa),
            );
        }
        if self.bg_wide.is_none() {
            // 新背景生成失败（没图/解码失败）：直接清掉旧图，不留残影
            self.bg_wide_prev = None;
            self.bg_detail_prev = None;
            self.bg_fading = false;
            self.bg_fade = 1.0;
        }
    }

    /// 整窗背景 = L1（糊）+ L2（清楚的那层，边缘化开）→ 压暗渐变
    fn paint_window_backdrop(&mut self, ctx: &egui::Context) {
        self.sync_window_backdrop(ctx);
        if self.bg_fading {
            self.bg_fade += ctx.input(|i| i.stable_dt).min(0.1) / 0.35;
            if self.bg_fade >= 1.0 {
                self.bg_fade = 1.0;
                self.bg_fading = false;
                self.bg_wide_prev = None;
                self.bg_detail_prev = None;
            }
            ctx.request_repaint();
        }
        let has_bg = self.bg_wide.is_some() || self.bg_wide_prev.is_some();
        if !has_bg {
            return; // 无选中/无封面：保持全透明，走系统亚克力
        }
        let screen = ctx.screen_rect();
        let p = ctx.layer_painter(egui::LayerId::background());
        let mut painted: Vec<(&egui::TextureHandle, f32)> = Vec::new();
        let inv = 1.0 - self.bg_fade;
        if let Some(t) = self.bg_wide_prev.as_ref() {
            painted.push((t, inv));
        }
        if let Some(t) = self.bg_detail_prev.as_ref() {
            painted.push((t, inv));
        }
        if let Some(t) = self.bg_wide.as_ref() {
            painted.push((t, self.bg_fade));
        }
        if let Some(t) = self.bg_detail.as_ref() {
            painted.push((t, self.bg_fade));
        }
        for (tex, a) in painted {
            if a <= 0.004 {
                continue;
            }
            let uv = cover_uv(tex.size(), screen.size());
            tex_round_rect(
                &p,
                screen,
                tex,
                uv,
                egui::Rounding::ZERO,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, (a * 255.0) as u8),
            );
        }
        // BorderDarken：整体压暗/提亮保证面板文字对比度。夜色压暗，日间罩一层浅雾。
        let (d0, d1) = if is_dark() {
            (
                egui::Color32::from_rgba_unmultiplied(10, 12, 22, 92),
                egui::Color32::from_rgba_unmultiplied(6, 8, 16, 168),
            )
        } else {
            (
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 46),
                egui::Color32::from_rgba_unmultiplied(244, 248, 255, 120),
            )
        };
        vgradient(&p, screen, d0, d1);
    }

    /// 右侧详情区：自适应内容（宽屏左右双栏 / 窄屏上下堆叠）
    /// enter = 入场进度 0→1（整块淡入 + 自下而上落定；封面另有 1.04→1.0 缩放）
    fn detail_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, i: usize, enter: f32) {
        // 先取好只读数据，后面要 &mut self 处理按钮动作，不能跨借用
        let title = display_name(&self.games[i]).to_string();
        let raw_name = self.games[i].name.clone();
        // 这个游戏是不是正开着（决定操作栏显示「启动游戏」还是「运行中」）
        let is_running = self.running.iter().any(|r| r.title == title);
        let has_exe = self.games[i].executable.is_some();
        let has_dir = self.games[i].folder.is_some();
        let editing = self.editing;

        // ---- 详情区骨架：头部 + 主体（属性表 + 备注）----
        //  ① 顶部一大段留白（CalculatedGameDetailsIndentation）：专门用来露背景图，形成「艺术区」
        //  ② 头部 DockPanel：左边=图标+大标题+按钮行（底对齐），右边=封面（底对齐、清晰）
        //  ③ 主体 DockPanel：左窄=「游戏详情」属性表，右宽=备注/描述
        // 之前那版是「详情区里再铺一张圆角卡片糊图」，糊得看不出来是什么，
        // 而且亮卡片怼在暗侧边栏上，硬生生切出一条卡片边界 —— 这版不再有那层。
        let win_h = ctx.screen_rect().height();
        // 窗口矮的时候把留白收掉，保证按钮和属性表还在首屏（不重复当初「要往下滑」的坑）
        let top_indent = if win_h < 620.0 { 10.0 } else { (win_h * 0.075).clamp(26.0, 80.0) };
        let cover_h = if win_h < 620.0 { 104.0 } else { (win_h * 0.27).clamp(120.0, 240.0) };

        let mut act = DetailAct::default();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // 入场位移：内容从下方 8px 处落定（和淡入同步，避免「凭空出现」）
                ui.add_space(top_indent + (1.0 - enter) * 8.0);
                let avail = ui.available_width();
                let narrow = avail < 560.0;

                // ---- ② 头部：左标题 / 右封面，都底对齐 ----
                let (tex, no_cover) = {
                    let dir = self.library_dir.clone();
                    let g = &self.games[i];
                    (
                        ensure_cover(ctx, &dir, &mut self.textures, &mut self.avg, g)
                            .unwrap_or_else(|| placeholder_tex(ctx, &mut self.placeholder)),
                        g.cover_local.is_none(),
                    )
                };
                let csz = tex.size();
                let caspect = csz[1] as f32 / csz[0].max(1) as f32;
                let ch = if narrow { 84.0 } else { cover_h };
                let cw = (ch / caspect.max(0.05)).min(avail * 0.34);

                // 左列固定高度：标题 + 胶囊行 + 按钮行（含 egui 的 item_spacing 10）用来做底对齐
                let left_h = if narrow { 112.0 } else { 118.0 };
                let head_h = ch.max(left_h);
                let (head_rect, _) = ui.allocate_exact_size(
                    egui::vec2(avail, head_h),
                    egui::Sense::hover(),
                );

                // 左列：大标题 + 状态胶囊 + 原始名，然后按钮行
                let left_w = (head_rect.width() - cw - 16.0).max(140.0);
                {
                    let mut left = ui.child_ui(
                        egui::Rect::from_min_max(
                            head_rect.min,
                            egui::pos2(head_rect.left() + left_w, head_rect.bottom()),
                        ),
                        egui::Layout::top_down(egui::Align::LEFT),
                    );
                    // 底对齐：把标题、按钮这一坨压到头部底边
                    left.add_space((head_h - left_h).max(0.0));
                    // 大标题带投影，压在照片背景上也读得清。
                    // 用 Color32::PLACEHOLDER 排版，同一个 galley 就能换色画两次（投影 + 正文）。
                    let tgal = {
                        let p = left.painter().clone();
                        fit_galley(
                            &p,
                            &title,
                            if narrow { 20.0 } else { 27.0 },
                            egui::Color32::PLACEHOLDER,
                            left_w,
                        )
                    };
                    let ty = tgal.size().y.max(24.0);
                    let (tr_rect, tr_resp) = left.allocate_exact_size(
                        egui::vec2(left_w, ty),
                        egui::Sense::hover(),
                    );
                    let tp = left.painter().clone();
                    // 大标题压在封面虚化底上要保可读性，但投影必须分主题：
                    // 夜色 = 浅色字压深底 → 深色投影（偏移收到 1.6/0.8，只「垫」一下，不糊字形）
                    // 日间 = 深色字压浅底 → 绝不能再叠黑投影：黑投影会把深字纵向撑高、笔画发毛，
                    //        观感就是「字被拉长变形」；浅底本身够清晰，直接画字最干净
                    if is_dark() {
                        let sh_a = ua(egui::Color32::from_black_alpha(150));
                        let sh_b = ua(egui::Color32::from_black_alpha(95));
                        tp.galley(tr_rect.min + egui::vec2(0.0, 1.6), tgal.clone(), sh_a);
                        tp.galley(tr_rect.min + egui::vec2(0.0, 0.8), tgal.clone(), sh_b);
                    }
                    tp.galley(tr_rect.min, tgal, text_main());
                    tr_resp.on_hover_text(&title);

                    left.add_space(4.0);
                    left.horizontal(|ui| {
                        if editing {
                            ui.label(
                                egui::RichText::new("· 编辑中")
                                    .size(12.0)
                                    .color(egui::Color32::from_rgb(240, 196, 96)),
                            );
                        } else if raw_name != title {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(format!("原始名：{raw_name}"))
                                        .size(11.5)
                                        .color(text_sub()),
                                )
                                .truncate(true),
                            )
                            .on_hover_text(&raw_name);
                        }
                    });
                    left.add_space(8.0);
                    // 必须包在 horizontal 里：left 是 top_down 布局，直接放按钮会一个按钮占一行
                    left.horizontal(|ui| {
                        act = detail_action_buttons(ui, editing, has_exe, has_dir, is_running);
                    });
                }

                // 右列：封面（清晰、只加圆角与投影，不糊）
                {
                    // 入场落定：封面从 1.04 缩回 1.0（与整块淡入同步）
                    let c_raw = egui::Rect::from_min_max(
                        egui::pos2(head_rect.right() - cw, head_rect.bottom() - ch),
                        head_rect.max,
                    );
                    let c_rect = egui::Rect::from_center_size(
                        c_raw.center(),
                        c_raw.size() * (1.0 + 0.04 * (1.0 - enter)),
                    );
                    let p = ui.painter().clone();
                    p.add(egui::epaint::RectShape::filled(
                        c_rect.translate(egui::vec2(0.0, 8.0)).expand2(egui::vec2(6.0, 6.0)),
                        egui::Rounding::same(11.0),
                        ua(egui::Color32::from_black_alpha(90)),
                    ));
                    let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    tex_round_rect(
                        &p,
                        c_rect,
                        &tex,
                        uv,
                        egui::Rounding::same(10.0),
                        ua(egui::Color32::WHITE),
                    );
                    p.rect_stroke(
                        c_rect,
                        10.0,
                        egui::Stroke::new(
                            1.0_f32,
                            ua(if is_dark() {
                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40)
                            } else {
                                egui::Color32::from_rgba_unmultiplied(30, 42, 80, 45)
                            }),
                        ),
                    );
                    if no_cover {
                        p.text(
                            c_rect.left_center() - egui::vec2(cw + 8.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            "未匹配封面\n右键卡片可重匹配",
                            egui::FontId::proportional(11.5),
                            text_sub(),
                        );
                    }
                }

                if act.edit {
                    self.load_edit_buffer(i);
                    self.editing = true;
                    self.settle = self.settle.max(4); // 编辑态换了另一套表单（Grid），补几帧
                }
                if act.cancel {
                    self.editing = false;
                    self.load_edit_buffer(i);
                    self.status = "已取消编辑".into();
                    self.settle = self.settle.max(4);
                }
                if act.save {
                    self.save_selected();
                    self.editing = false;
                    self.settle = self.settle.max(4);
                }
                if act.launch {
                    self.launch_exe(i, &title);
                }
                if act.stop {
                    self.stop_game(&title);
                }
                if act.open_dir {
                    match self.games.get(i).and_then(|g| g.folder.clone()) {
                        Some(folder) => {
                            let _ = std::process::Command::new("explorer").arg(folder).spawn();
                        }
                        None => self.status = "该条目没有文件夹路径".into(),
                    }
                }

                ui.add_space(14.0);

                // ---- ③ 主体：左属性表 / 右备注（左栏固定宽、右栏取剩余）----
                let cur_name = display_name(&self.games[i]).to_string();
                let snap = InfoSnap {
                    progress: self.games[i].progress,
                    rating: self.games[i].rating,
                    playtime: self.games[i].playtime_hours,
                    // 运行中的话，把「本次这一段」也算进去显示（还没到 1 分钟、尚未落盘的部分）
                    running_secs: self
                        .running
                        .iter()
                        .find(|r| r.title == cur_name)
                        .map(|r| r.started.elapsed().as_secs_f32())
                        .unwrap_or(0.0),
                    note: self.games[i].note.clone(),
                };
                let appid = self.games[i].steam_appid;
                let added = self.games[i].added_at.clone();
                let folder = self.games[i].folder.clone();
                let exe = self.games[i].executable.clone();
                let two_col = ui.available_width() >= 720.0;
                if two_col {
                    let pw = (ui.available_width() * 0.40).clamp(300.0, 400.0);
                    let nw = (ui.available_width() - pw - 14.0).max(220.0);
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_max_width(pw);
                            self.props_panel(
                                ui,
                                pw,
                                &snap,
                                appid,
                                &added,
                                folder.as_deref(),
                                exe.as_deref(),
                                i,
                            );
                        });
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(nw);
                            self.notes_panel(ui, nw, &snap);
                        });
                    });
                } else {
                    let w = ui.available_width();
                    self.props_panel(
                        ui,
                        w,
                        &snap,
                        appid,
                        &added,
                        folder.as_deref(),
                        exe.as_deref(),
                        i,
                    );
                    ui.add_space(12.0);
                    self.notes_panel(ui, w, &snap);
                }
                ui.add_space(10.0);
            });
    }

    /// 「游戏详情」属性表（标题 + 分隔线 + 两列 Grid）
    fn props_panel(
        &mut self,
        ui: &mut egui::Ui,
        w: f32,
        s: &InfoSnap,
        appid: Option<u32>,
        added: &str,
        folder: Option<&str>,
        exe: Option<&str>,
        i: usize,
    ) {
        glass_frame().show(ui, |ui| {
            ui.set_min_width((w - 28.0).max(150.0));
            ui.label(egui::RichText::new("游戏详情").size(14.0).strong().color(text_main()));
            ui.add_space(2.0);
            ui.separator();
            ui.add_space(4.0);
            if self.editing {
                prop_row(ui, "进度 %", |ui| {
                    ui.add(egui::Slider::new(&mut self.edit_progress, 0..=100));
                });
                prop_row(ui, "评分", |ui| {
                    ui.add(egui::Slider::new(&mut self.edit_rating, 0..=10));
                });
                prop_row(ui, "时长", |ui| {
                    ui.add_sized(
                        [76.0, 26.0],
                        egui::TextEdit::singleline(&mut self.edit_playtime).hint_text("0"),
                    );
                    ui.label(egui::RichText::new("小时").color(text_sub()).size(13.0));
                });
                prop_row(ui, "入库", |ui| {
                    ui.label(egui::RichText::new(short_date(added)).color(text_sub()).size(13.0));
                });
                if let Some(id) = appid {
                    prop_row(ui, "AppID", |ui| {
                        ui.label(egui::RichText::new(format!("{id}")).color(text_sub()).size(13.0));
                    });
                }
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("改完点顶部「保存」，「取消」放弃本次改动。")
                        .color(text_sub())
                        .size(11.5),
                );
                let _ = i;
                return;
            }
            prop_row(ui, "进度", |ui| {
                let (t, c) = if s.progress > 0 {
                    (format!("{} %", s.progress), text_main())
                } else {
                    ("—".to_string(), text_sub())
                };
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(t).color(c).size(13.5));
                    if s.progress > 0 {
                        ui.add_space(4.0);
                        thin_progress(ui, s.progress, accent_fill());
                    }
                });
            });
            prop_row(ui, "评分", |ui| {
                let (rt, rc) = if s.rating > 0 {
                    (format!("★ {} / 10", s.rating), egui::Color32::from_rgb(240, 196, 96))
                } else {
                    ("未评分".to_string(), text_sub())
                };
                ui.label(egui::RichText::new(rt).color(rc).size(13.5));
            });
            prop_row(ui, "时长", |ui| {
                // 运行中的这一段还没落盘，但显示上要立刻看得见（数字会一直走）
                let total = s.playtime + s.running_secs / 3600.0;
                let (tt, tc) = if total > 0.0 {
                    (fmt_playtime(total), text_main())
                } else {
                    ("未记录".to_string(), text_sub())
                };
                ui.label(egui::RichText::new(tt).color(tc).size(13.5));
            });
            prop_row(ui, "入库", |ui| {
                ui.label(egui::RichText::new(short_date(added)).color(text_sub()).size(13.0));
            });
            if let Some(id) = appid {
                prop_row(ui, "AppID", |ui| {
                    ui.label(egui::RichText::new(format!("{id}")).color(text_sub()).size(13.0));
                });
            }
            prop_row(ui, "目录", |ui| {
                match folder {
                    Some(f) => {
                        let name = Path::new(f)
                            .file_name()
                            .map(|x| x.to_string_lossy().into_owned())
                            .unwrap_or_else(|| f.to_string());
                        ui.add(
                            egui::Label::new(egui::RichText::new(name).color(text_main()).size(13.0))
                                .truncate(true),
                        )
                        .on_hover_text(f);
                    }
                    None => {
                        ui.label(egui::RichText::new("—").color(text_sub()).size(13.0));
                    }
                }
            });
            if let Some(e) = exe {
                prop_row(ui, "主程序", |ui| {
                    let name = Path::new(e)
                        .file_name()
                        .map(|x| x.to_string_lossy().into_owned())
                        .unwrap_or_else(|| e.to_string());
                    ui.add(
                        egui::Label::new(egui::RichText::new(name).color(text_sub()).size(13.0))
                            .truncate(true),
                    )
                    .on_hover_text(e);
                });
            }
        });
    }

    /// 备注栏（以后接上 Steam 商店详情就能并排放「描述」）
    fn notes_panel(&mut self, ui: &mut egui::Ui, w: f32, s: &InfoSnap) {
        glass_frame().show(ui, |ui| {
            ui.set_min_width((w - 28.0).max(150.0));
            ui.label(egui::RichText::new("备注").size(14.0).strong().color(text_main()));
            ui.add_space(2.0);
            ui.separator();
            ui.add_space(6.0);
            if self.editing {
                ui.add(
                    egui::TextEdit::multiline(&mut self.edit_note)
                        .desired_rows(7)
                        .desired_width(f32::INFINITY)
                        .hint_text("存档位置、修改器、通关心得…"),
                );
            } else if s.note.trim().is_empty() {
                ui.label(
                    egui::RichText::new("（空）点顶部「编辑」可以写点东西：存档位置、修改器、通关心得。")
                        .color(text_sub())
                        .size(13.0),
                );
            } else {
                ui.label(egui::RichText::new(&s.note).color(text_main()).size(14.0));
            }
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("点顶部「编辑」可修改以上信息。")
                    .color(text_sub())
                    .size(11.5),
            );
        });
    }
}

/// 「2026-10-08T22:58:00+08:00」→「2026-10-08」
fn short_date(s: &str) -> String {
    if s.len() >= 10 {
        s[..10].to_string()
    } else {
        s.to_string()
    }
}

/// 生成一张灰色占位纹理（无封面时使用，避免渲染无效纹理）
fn placeholder_tex(ctx: &egui::Context, cache: &mut Option<egui::TextureHandle>) -> egui::TextureHandle {
    if let Some(t) = cache {
        return t.clone();
    }
    let w = 200;
    let h = 300;
    let mut px = vec![0u8; w * h * 4];
    for i in 0..(w * h) {
        px[i * 4] = 46;
        px[i * 4 + 1] = 50;
        px[i * 4 + 2] = 62;
        px[i * 4 + 3] = 255;
    }
    let ci = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
    let t = ctx.load_texture("__placeholder", ci, egui::TextureOptions::LINEAR);
    *cache = Some(t.clone());
    t
}

/// 长文本按像素宽截断 + 省略号（break-ui 规范：截断必须配 hover 看全名，由调用方负责）
fn fit_galley(
    p: &egui::Painter,
    text: &str,
    size: f32,
    color: egui::Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let font = egui::FontId::proportional(size);
    let gal = p.layout_no_wrap(text.to_string(), font.clone(), color);
    if gal.size().x <= max_w {
        return gal;
    }
    let mut chars: Vec<char> = text.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        let g2 = p.layout_no_wrap(candidate, font.clone(), color);
        if g2.size().x <= max_w {
            return g2;
        }
    }
    gal
}

/// 左侧列表里的玻璃游戏卡片：封面缩略 + 名字 + 状态/评分 + 细进度条
/// 注意：整卡只 allocate 一次，内容全部用 painter 直绘 —— 禁止 ui.put
/// （ui.put 内部会 allocate_ui_at_rect 再次推进布局光标，导致相邻卡片重叠）
fn game_card(
    ui: &mut egui::Ui,
    tex: &egui::TextureHandle,
    g: &GameEntry,
    // 列表里的序号：给补间动画当稳定 id（名字会变、会重名，不能当 key）
    idx: usize,
    is_sel: bool,
    // tint = 该游戏封面的平均色：用来做卡片自带的毛玻璃底，卡片透出自身封面的虚化色
    tint: egui::Color32,
    // appear = 入场进度 0→1（列表错峰淡入 + 封面落定缩放共用）
    appear: f32,
    // running = 这个游戏正开着（从我们这里启动的），右上角点一颗绿点
    running: bool,
) -> egui::Response {
    let avail = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail, 78.0), egui::Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    // ---- 补间量（animate_value_with_time 自带缓出，且动画期间会自动请求重绘）----
    let th = ui.ctx().animate_value_with_time(
        egui::Id::new(("card_hov", idx)),
        if hovered { 1.0 } else { 0.0 },
        0.12,
    );
    let ts = ui.ctx().animate_value_with_time(
        egui::Id::new(("card_sel", idx)),
        if is_sel { 1.0 } else { 0.0 },
        0.18,
    );
    let a = appear;
    // 卡片底：默认 → hover 补间，选中再叠一层蓝（不是硬切）
    let base = mixc(glass_fill(), glass_fill_hover(), ease(th));
    let fill = mixc(base, egui::Color32::from_rgba_unmultiplied(10, 132, 255, 46), ease(ts));
    let p = ui.painter().clone();
    p.rect_filled(rect, 12.0, fa(fill, a));
    // 封面虚化色叠加：卡片自带一层自身封面的毛玻璃底（选中/hover 时更明显）
    let tint_a = (42.0 + 26.0 * ease(th) + 28.0 * ease(ts)).min(110.0) as u8;
    p.rect_filled(
        rect,
        12.0,
        fa(
            egui::Color32::from_rgba_unmultiplied(
                (tint.r() as f32 * 0.62) as u8,
                (tint.g() as f32 * 0.62) as u8,
                (tint.b() as f32 * 0.68) as u8,
                tint_a,
            ),
            a,
        ),
    );
    // 描边：默认 → hover → 选中，逐级补间；选中时线也稍微加粗
    let hov_stroke = if is_dark() {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 45)
    } else {
        egui::Color32::from_rgba_unmultiplied(30, 42, 80, 55)
    };
    let stroke_c = mixc(
        mixc(glass_stroke(), hov_stroke, ease(th)),
        egui::Color32::from_rgb(120, 180, 255),
        ease(ts),
    );
    p.rect_stroke(
        rect,
        12.0,
        egui::Stroke::new(1.0 + 0.2 * ease(ts), fa(stroke_c, a)),
    );
    // 选中标记：左侧高亮条从 0 宽滑入（iOS 列表选中观感）
    if ts > 0.01 {
        let bh = rect.height() * 0.46;
        let bar = egui::Rect::from_min_size(
            pos2(rect.min.x + 2.5, rect.center().y - bh * 0.5),
            egui::vec2(3.0 * ease(ts), bh),
        );
        p.rect_filled(
            bar,
            1.5,
            fa(egui::Color32::from_rgb(90, 165, 255), a * ease(ts)),
        );
    }

    // 封面缩略：圆角 + 柔化投影（egui 的 RectShape 没有 blur_width，用两层递减 alpha 叠出软边）
    let pad = 10.0_f32;
    let th_size = 58.0_f32;
    let sz = tex.size();
    let aspect = sz[1] as f32 / sz[0] as f32;
    let tw = (th_size / aspect).min(62.0);
    // 悬停放大约 3%，同时入场时从 1.06 缩回 1.0（解码落定）
    let grow = 0.03 * ease(th) + 0.06 * (1.0 - ease(a));
    let base_rect =
        egui::Rect::from_min_size(rect.min + egui::vec2(pad, pad), egui::vec2(tw, th_size));
    let img_rect = egui::Rect::from_center_size(base_rect.center(), base_rect.size() * (1.0 + grow));
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    p.add(egui::epaint::RectShape::filled(
        img_rect.translate(egui::vec2(0.0, 3.0)).expand2(egui::vec2(3.5, 3.5)),
        egui::Rounding::same(9.0),
        fa(egui::Color32::from_black_alpha(26), a),
    ));
    p.add(egui::epaint::RectShape::filled(
        img_rect.translate(egui::vec2(0.0, 1.5)).expand2(egui::vec2(1.5, 1.5)),
        egui::Rounding::same(8.0),
        fa(egui::Color32::from_black_alpha(38), a),
    ));
    // 封面本身按入场进度淡入（解码完成不再「啪」地跳出来）
    tex_round_rect(&p, img_rect, tex, uv, egui::Rounding::same(7.0), fa(egui::Color32::WHITE, a));
    p.rect_stroke(
        img_rect,
        7.0,
        egui::Stroke::new(
            1.0_f32,
            fa(
                if is_dark() {
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30)
                } else {
                    egui::Color32::from_rgba_unmultiplied(30, 42, 80, 40)
                },
                a,
            ),
        ),
    );

    // 名字（显示名 = 别名 > 匹配名 > 原始名；按像素宽截断 + 省略号，全名靠 hover tooltip）
    let title = display_name(g).to_string();
    let tx = img_rect.right() + 12.0;
    // 运行中的话右上角要留出绿点的位置，名字可用宽度相应收窄
    let dot_w = if running { 22.0 } else { 0.0 };
    let max_w = rect.right() - 12.0 - dot_w - tx;
    let name_c = text_main();
    let name_gal = fit_galley(&p, &title, 15.0, name_c, max_w);
    p.galley(pos2(tx, rect.top() + 11.0), name_gal, fa(name_c, a));

    // 运行中标记：一颗绿点（具体玩了多久在详情页的「时长」行里）
    if running {
        let rc = if is_dark() {
            egui::Color32::from_rgb(96, 214, 156)
        } else {
            egui::Color32::from_rgb(20, 156, 104)
        };
        let c = pos2(rect.right() - 19.0, rect.top() + 16.0);
        p.circle_filled(
            c,
            6.5,
            fa(egui::Color32::from_rgba_unmultiplied(rc.r(), rc.g(), rc.b(), 62), a),
        );
        p.circle_filled(c, 3.6, fa(rc, a));
    }

    // 进度 + 评分
    let mut meta = String::new();
    if g.progress > 0 {
        meta.push_str(&format!("　{}%", g.progress));
    }
    if g.rating > 0 {
        meta.push_str(&format!("　★{}", g.rating));
    }
    let meta_c = text_sub();
    let meta_gal = fit_galley(&p, &meta, 12.5, meta_c, max_w);
    p.galley(pos2(tx, rect.top() + 33.0), meta_gal, fa(meta_c, a));

    // 细进度条（玻璃感：白色轨道 + 主色填充）；入场时填充条从左「长」出来
    if g.progress > 0 {
        let bar_y = rect.bottom() - 11.0;
        let track = egui::Rect::from_min_max(pos2(tx, bar_y), pos2(rect.right() - 12.0, bar_y + 3.0));
        let track_c = if is_dark() {
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 24)
        } else {
            egui::Color32::from_rgba_unmultiplied(20, 30, 60, 20)
        };
        p.rect_filled(track, 1.5, fa(track_c, a));
        let w = track.width() * g.progress as f32 / 100.0 * ease(a);
        if w > 1.0 {
            let fill_r = egui::Rect::from_min_max(track.min, pos2(track.min.x + w, track.max.y));
            p.rect_filled(fill_r, 1.5, fa(accent_fill(), a));
        }
    }
    resp
}

/// Windows 亚克力模糊（真·系统级透明，透出桌面）：直接走 user32 的
/// SetWindowCompositionAttribute（undocumented 但稳定，Tauri 的 window-vibrancy 同款实现）
#[cfg(windows)]
fn apply_windows_acrylic(dark: bool) {
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }
    #[repr(C)]
    struct CompAttrData {
        attribute: u32,
        data: *mut AccentPolicy,
        size_of_data: usize,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> isize;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryA(name: *const u8) -> isize;
        fn GetProcAddress(module: isize, name: *const u8) -> isize;
    }

    const WCA_ACCENT_POLICY: u32 = 19;
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
    const ACCENT_FLAG_DRAW_ALL_LAYERS: u32 = 2;

    let title: Vec<u16> = std::ffi::OsStr::new("游戏柜 · game-cabinet")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd == 0 {
            eprintln!("未找到主窗口句柄，跳过亚克力效果");
            return;
        }
        // gradient_color 为 ABGR：主题色底 + 中低 alpha，让桌面的虚化透上来但不至于把画面压死
        // 夜色=深蓝黑；日间=低不透明度冷白（纯白高 alpha 会把模糊层压成死白，玻璃感全无）
        let tint = if dark {
            (112u32 << 24) | (26 << 16) | (22 << 8) | 14
        } else {
            (48u32 << 24) | (255 << 16) | (246 << 8) | 240
        };
        let mut policy = AccentPolicy {
            accent_state: ACCENT_ENABLE_ACRYLICBLURBEHIND,
            accent_flags: ACCENT_FLAG_DRAW_ALL_LAYERS,
            gradient_color: tint,
            animation_id: 0,
        };
        let data = CompAttrData {
            attribute: WCA_ACCENT_POLICY,
            data: &mut policy,
            size_of_data: std::mem::size_of::<AccentPolicy>(),
        };
        // SetWindowCompositionAttribute 是非公开 API，user32.lib 无导出符号，运行时动态加载
        type SetAttr = unsafe extern "system" fn(isize, *const CompAttrData) -> i32;
        let name = b"SetWindowCompositionAttribute\0";
        let user32 = LoadLibraryA(b"user32.dll\0".as_ptr());
        let proc = GetProcAddress(user32, name.as_ptr());
        if proc == 0 {
            eprintln!("当前系统不支持 SetWindowCompositionAttribute");
            return;
        }
        let set_attr: SetAttr = std::mem::transmute(proc);
        let ok = set_attr(hwnd, &data);
        if ok == 0 {
            eprintln!("SetWindowCompositionAttribute 失败（Win10/11 应支持亚克力）");
        }
    }
}

/// Win11 圆角窗口：系统圆角是画在原生边框上的，无边框窗口不再自动获得，
/// 用 DWM 手动加回来（Win10 无此属性，调用失败静默忽略，窗口保持直角）。
fn apply_window_rounding() {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> isize;
    }
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attr: u32, value: *const u32, size: u32) -> i32;
    }

    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: u32 = 2;

    let title: Vec<u16> = std::ffi::OsStr::new("游戏柜 · game-cabinet")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd == 0 {
            return;
        }
        let pref: u32 = DWMWCP_ROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref, 4);
    }
}

impl eframe::App for CabinetApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 变了东西之后主动补几帧。egui 是反应式的（实测空闲 6 秒只渲染 6 帧），
        // 而部分控件要「第二帧」才对：egui::Grid 首帧故意不可见（grid.rs 里
        // `ui.set_visible(prev_state.is_some())`，避免首帧抖动），交叉淡入也需要连续帧。
        // 不补这几帧的话，界面就会停在半成品状态等用户下一次动鼠标 —— 也就是「慢一拍」。
        if self.settle > 0 {
            self.settle -= 1;
            ctx.request_repaint();
        }

        // ---- 动效计时器推进 ----
        // 全是「一次性入场」进度，跑满就停（停掉后不再 request_repaint，不白烧 CPU）。
        // egui 是反应式的：动画期间必须自己叫重绘，否则会停在半路。
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        if self.selected != self.detail_key {
            // 换了游戏：详情内容从头入场（与背景交叉淡入同步）
            self.detail_key = self.selected;
            self.detail_t = 0.0;
        }
        if self.detail_t < 1.0 {
            self.detail_t = (self.detail_t + dt / 0.26).min(1.0);
            ctx.request_repaint();
        }
        if self.list_t < 1.6 {
            self.list_t = (self.list_t + dt).min(1.6);
            ctx.request_repaint();
        }
        if self.intro_t < 1.0 {
            self.intro_t = (self.intro_t + dt / 0.30).min(1.0);
            ctx.request_repaint();
        }
        if self.scan_done_t > 0.0 {
            self.scan_done_t = (self.scan_done_t - dt / 1.2).max(0.0);
            ctx.request_repaint();
        }

        // ---- 运行中会话：推进计时 / 结算已退出的游戏 ----
        self.poll_running(ctx);

        // 首帧：窗口已存在，应用系统亚克力模糊
        if !self.vibrancy_done {
            self.vibrancy_done = true;
            #[cfg(windows)]
            {
                apply_windows_acrylic(self.dark);
                apply_window_rounding();
            }
        }

        // 英雄图下载完成（或确认没有）：写回条目 → 重建背景 → 落盘
        if let Some(rx) = self.hero_rx.take() {
            match rx.try_recv() {
                Ok(Some(m)) => {
                    let is_sel = self.selected == Some(m.idx);
                    if let Some(g) = self.games.get_mut(m.idx) {
                        g.hero_local = Some(m.rel);
                    }
                    // 只有「当前正在看的那款」换了图才需要立刻重建背景；
                    // 预取回来的其它游戏的图，等真点到它时 backdrop_src 自然给出新路径。
                    // 否则每抓完一张都会触发一次无意义的重建 + 交叉淡入，看着像在抽动。
                    if is_sel {
                        self.bg_src = None; // 换背景源，下一帧重建
                        self.settle = self.settle.max(6);
                    }
                    let lib = crate::library::Library { games: self.games.clone() };
                    if let Err(e) = lib.save(&self.library_path) {
                        eprintln!("英雄图路径保存失败: {e}");
                    }
                    ctx.request_repaint();
                }
                Ok(None) => {}
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.hero_rx = Some(rx); // 还没好，放回继续等
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
            }
        }
        self.maybe_fetch_hero(ctx);

        // 整窗口虚化背景。
        // 注意：egui 的 SidePanel/TopBottomPanel/CentralPanel 用的就是 LayerId::background()
        // （见 egui containers/panel.rs），即面板和这张背景图落在【同一个 PaintList】里，
        // 合成顺序 = 添加顺序。所以必须画在所有面板之前，否则背景会盖住整个界面。
        self.paint_window_backdrop(ctx);

        // 接收后台扫描结果
        if let Some(rx) = self.scan_rx.take() {
            match rx.try_recv() {
                Ok(msg) => {
                    self.games = msg.library.games;
                    self.library_dir = msg.dir.clone();
                    self.library_path = msg.dir.join("library.json");
                    self.textures.clear();
                    self.selected = None;
                    self.editing = false;
                    self.status = format!("扫描完成，共 {} 款", self.games.len());
                    // 动效：列表重新错峰入场 + 顶部亮一下「扫描完成」对勾
                    self.list_t = 0.0;
                    self.scan_done_t = 1.0;
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.scan_rx = Some(rx); // 还没好，放回继续等
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
            }
        }

        // 接收「按名称重新匹配」结果
        if let Some(rx) = self.rematch_rx.take() {
            match rx.try_recv() {
                Ok(m) => {
                    let ok_msg = m.error.clone();
                    if let Some(g) = self.games.get_mut(m.idx) {
                        if m.error.is_none() {
                            // 匹配成功：Steam 返回的正名直接作为游戏名
                            let display =
                                m.matched.clone().unwrap_or_else(|| m.new_name.clone());
                            if !display.is_empty() {
                                g.name = display;
                            }
                        }
                        if m.appid.is_some() {
                            g.steam_appid = m.appid;
                        }
                        if m.matched.is_some() {
                            g.matched_name = m.matched.clone();
                        }
                        if m.cover_url.is_some() {
                            g.cover_url = m.cover_url;
                        }
                        if m.header_url.is_some() {
                            g.header_url = m.header_url;
                        }
                        if m.cover_local.is_some() {
                            g.cover_local = m.cover_local;
                        }
                    }
                    self.textures.clear(); // 封面变了，全部重载
                    self.bg_src = None; // 强制重建背景（可能拿到了新的英雄图）
                    self.hero_tried.clear();
                    self.status = match ok_msg {
                        Some(e) => format!("匹配失败：{e}"),
                        None => format!("已匹配：{}", {
                            // matched 已写回，从游戏条目取回显示
                            self.games
                                .get(m.idx)
                                .and_then(|g| g.matched_name.clone())
                                .unwrap_or_default()
                        }),
                    };
                    let lib = crate::library::Library { games: self.games.clone() };
                    if let Err(e) = lib.save(&self.library_path) {
                        self.status = format!("保存失败: {e}");
                    }
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.rematch_rx = Some(rx); // 还没好，放回继续等
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
            }
        }

        // 自绘玻璃顶栏 = 标题栏 + 工具栏合一（按 apple-design 准则重做）：
        // · 材质分层级：顶栏是结构层 → 较重的半透明材质；按钮是交互层 → 轻/透明，不叠亮面
        // · 层级靠「字重 + 字号 + 分组留白」表达，去掉分隔线这类硬边
        // · 受光亮边：顶边一条更亮的细线模拟玻璃上缘反光；底缘用渐变淡出代替硬线
        egui::TopBottomPanel::top("top")
            .exact_height(34.0) // 收窄：内容全走 child_ui，主 ui 量不到高度，必须显式定高
            .frame(
                egui::Frame::none()
                    .fill(if is_dark() {
                        egui::Color32::from_rgba_unmultiplied(13, 15, 24, 150)
                    } else {
                        egui::Color32::from_rgba_unmultiplied(252, 252, 255, 116)
                    })
                    .inner_margin(egui::Margin::symmetric(14.0, 2.0)),
            )
            .show(ctx, |ui| {
                let full = ui.available_rect_before_wrap();
                // 窗口控制按钮与窗口边缘保持呼吸间距（右 6px、上下 2px），不再贴死角落
                let ctrl_w = 3.0 * 34.0 + 8.0; // 三个控制按钮 + 按钮间距
                let ctrl_rect = egui::Rect::from_min_max(
                    egui::pos2(full.right() - ctrl_w - 6.0, full.top() + 2.0),
                    egui::pos2(full.right() - 6.0, full.bottom() - 2.0),
                );
                let left_rect = egui::Rect::from_min_max(
                    full.min,
                    egui::pos2(full.right() - ctrl_w, full.bottom()),
                );

                // 整条顶栏空白区都是拖拽区：先铺一层不可见的拖拽/双击面，
                // 之后 child_ui 里的按钮等控件画在其上层，点击命中仍优先给按钮。
                let bar_resp = ui.allocate_rect(full, egui::Sense::click_and_drag());
                if bar_resp.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if bar_resp.double_clicked() {
                    self.maximized = !self.maximized;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(self.maximized));
                }

                let mut left_ui =
                    ui.child_ui(left_rect, egui::Layout::left_to_right(egui::Align::Center));

                // ── 标题组：应用图标（拖拽移动窗口，双击切换最大化）──
                let icon_resp = {
                    let tex = self.icon_tex.get_or_insert_with(|| {
                        let img = image::load_from_memory(APP_ICON_PNG)
                            .expect("内置应用图标解码失败")
                            .into_rgba8();
                        let size = [img.width() as usize, img.height() as usize];
                        left_ui.ctx().load_texture(
                            "app-icon",
                            egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
                            egui::TextureOptions::LINEAR,
                        )
                    });
                    let (ir, resp) =
                        left_ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::drag());
                    left_ui.painter().image(
                        tex.id(),
                        ir,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    resp
                };
                if icon_resp.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if icon_resp.double_clicked() {
                    self.maximized = !self.maximized;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(self.maximized));
                }

                // ── 操作组：单个「库」按钮，点击弹出下拉三项 ──
                // 不用 egui 原生 menu_button —— 它是瞬弹的，做不了补间展开动画。
                // 这里自绘按钮 + 自管理弹层（Area），入场做「淡入 + 自上方下滑 6px」。
                left_ui.add_space(22.0);
                {
                    let (brect, bresp) =
                        left_ui.allocate_exact_size(egui::vec2(56.0, 22.0), egui::Sense::click());
                    // 按钮悬停补间（幽灵样式 → 浅色圆角底），展开期间保持高亮
                    let bh = ctx.animate_value_with_time(
                        egui::Id::new("lib_btn_hov"),
                        if bresp.hovered() || self.lib_menu { 1.0 } else { 0.0 },
                        0.12,
                    );
                    if bresp.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let bp = left_ui.painter();
                    let bg = if self.lib_menu {
                        if is_dark() {
                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 46)
                        } else {
                            egui::Color32::from_rgba_unmultiplied(20, 30, 60, 30)
                        }
                    } else {
                        hover_veil()
                    };
                    if bh > 0.002 {
                        bp.rect_filled(brect, 7.0, fa(bg, bh));
                    }
                    let btxt = mixc(text_sub(), text_main(), bh);
                    let ir = egui::Rect::from_min_size(
                        brect.min + egui::vec2(8.0, 3.0),
                        egui::vec2(16.0, 16.0),
                    );
                    draw_lib_grid(bp, ir, btxt);
                    bp.text(
                        egui::pos2(ir.right() + 6.0, brect.center().y),
                        egui::Align2::LEFT_CENTER,
                        "库",
                        egui::FontId::proportional(14.0),
                        btxt,
                    );
                    if bresp.clicked() {
                        self.lib_menu = !self.lib_menu;
                        ctx.request_repaint();
                    }

                    // ---- 自管理弹层：t_open 驱动淡入 + 下滑 ----
                    let t_open = ctx.animate_value_with_time(
                        egui::Id::new("lib_menu_t"),
                        if self.lib_menu { 1.0 } else { 0.0 },
                        0.18,
                    );
                    if t_open > 0.01 {
                        let e = ease(t_open);
                        let mut act: Option<u8> = None;
                        let pos = egui::pos2(brect.left(), brect.bottom() + 8.0 - 8.0 * e);
                        let area = egui::Area::new(egui::Id::new("lib_menu"))
                            .order(egui::Order::Foreground)
                            .fixed_pos(pos)
                            .show(ctx, |ui| {
                                let panel_fill = if is_dark() {
                                    egui::Color32::from_rgba_unmultiplied(28, 32, 44, 244)
                                } else {
                                    egui::Color32::from_rgba_unmultiplied(250, 251, 255, 246)
                                };
                                egui::Frame::none()
                                    .fill(fa(panel_fill, e))
                                    .rounding(egui::Rounding::same(12.0))
                                    .stroke(egui::Stroke::new(1.0_f32, fa(glass_stroke(), e)))
                                    // 弹层留白：上下 8、左右 8（原来只有 6，文字贴边显局促）
                                    .inner_margin(egui::Margin::symmetric(8.0, 8.0))
                                    .show(ui, |ui| {
                                        // 自绘行，不依赖系统按钮：行高、左右内边距、行间距完全可控，
                                        // 悬停底色也与弹层等宽（系统按钮会缩到文字宽，看着短一截）
                                        const W: f32 = 200.0;
                                        const ROW_H: f32 = 30.0;
                                        const ROW_GAP: f32 = 4.0;
                                        // 先固定整体宽度，避免逐行 allocate 时宽度抖动；
                                        // item_spacing 收到 0（行间距由 ROW_GAP + 行高内边距控制，
                                        // 默认的 10px 会叠上去让行距虚胖）
                                        ui.set_min_width(W);
                                        ui.spacing_mut().item_spacing = egui::vec2(0.0, ROW_GAP);
                                        let items: [&str; 3] =
                                            ["选择游戏文件夹…", "打开库文件…", "重新扫描"];
                                        for (i, label) in items.iter().enumerate() {
                                            // 逐条错峰落下：第 n 行晚 0.18t 起步（菜单有「层层落下」的节奏）
                                            let rt = ((t_open - i as f32 * 0.18) / 0.82)
                                                .clamp(0.0, 1.0);
                                            let re = ease(rt);
                                            let (rrect, rresp) = ui.allocate_exact_size(
                                                egui::vec2(W, ROW_H),
                                                egui::Sense::click(),
                                            );
                                            // 悬停底色补间（每行独立 id）
                                            let hv = ui.ctx().animate_value_with_time(
                                                egui::Id::new(("menu_row_hov", i)),
                                                if rresp.hovered() { 1.0 } else { 0.0 },
                                                0.10,
                                            );
                                            if hv > 0.002 {
                                                ui.painter().rect_filled(
                                                    rrect,
                                                    8.0,
                                                    fa(hover_veil(), hv * re),
                                                );
                                            }
                                            if rresp.hovered() {
                                                ui.ctx().set_cursor_icon(
                                                    egui::CursorIcon::PointingHand,
                                                );
                                            }
                                            // 文字：左对齐、距弹层边 12px（比原来宽松）
                                            ui.painter().text(
                                                egui::pos2(rrect.left() + 12.0, rrect.center().y),
                                                egui::Align2::LEFT_CENTER,
                                                *label,
                                                egui::FontId::proportional(13.5),
                                                fa(text_main(), re),
                                            );
                                            if rresp.clicked() {
                                                act = Some(i as u8);
                                            }
                                        }
                                    });
                            });
                        // 点空白处收起（点按钮本身由上面的 toggle 处理）
                        if self.lib_menu && !area.response.hovered() && !bresp.hovered() {
                            let clicked = ctx.input(|i| i.pointer.any_click());
                            if clicked {
                                self.lib_menu = false;
                                ctx.request_repaint();
                            }
                        }
                        if let Some(k) = act {
                            self.lib_menu = false;
                            ctx.request_repaint();
                            match k {
                                0 => {
                                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                        self.start_scan(dir);
                                    }
                                }
                                1 => {
                                    if let Some(p) = rfd::FileDialog::new()
                                        .add_filter("JSON", &["json"])
                                        .pick_file()
                                    {
                                        self.open_library(p);
                                    }
                                }
                                _ => {
                                    if !self.library_dir.as_os_str().is_empty() {
                                        self.start_scan(self.library_dir.clone());
                                    }
                                }
                            }
                        }
                    }
                }

                // ── 主题胶囊：日/夜切换 ──
                left_ui.add_space(16.0);
                if let Some(nd) = theme_capsule(&mut left_ui, self.dark) {
                    if nd != self.dark {
                        self.dark = nd;
                        THEME_DARK.store(nd, Ordering::Relaxed);
                        apply_theme(ctx);
                        // 亚克力 tint 也跟着换：夜色深蓝黑 / 日间亮白
                        #[cfg(windows)]
                        apply_windows_acrylic(nd);
                        ctx.request_repaint();
                    }
                }

                // ── 信息组：状态圆点 + 文字（库路径不再展示，没有信息量）──
                left_ui.add_space(18.0);
                let st_color = if self.scan_rx.is_some() {
                    egui::Color32::from_rgb(230, 190, 90)
                } else {
                    egui::Color32::from_rgb(120, 200, 140)
                };
                let (dot_rect, _) =
                    left_ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                left_ui
                    .painter()
                    .circle_filled(dot_rect.center(), 3.0, st_color);
                left_ui.add_space(5.0);
                left_ui.label(
                    egui::RichText::new(&self.status).color(text_sub()).size(12.0),
                );

                // ── 右侧窗口控制 ──
                let mut ctrl_ui =
                    ui.child_ui(ctrl_rect, egui::Layout::left_to_right(egui::Align::Center));
                ctrl_ui.add_space(2.0);
                window_control_button(&mut ctrl_ui, ctx, 'm', self.maximized);
                ctrl_ui.add_space(2.0);
                window_control_button(&mut ctrl_ui, ctx, 'x', self.maximized);
                ctrl_ui.add_space(2.0);
                window_control_button(&mut ctrl_ui, ctx, 'c', self.maximized);

                // ── 玻璃受光细节：顶边亮线 + 底缘渐变淡出（替代硬分隔线）──
                let screen = ctx.screen_rect();
                let wide = ui.painter().with_clip_rect(screen);
                let (fade_c, edge_c) = if is_dark() {
                    (
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 66),
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 70),
                    )
                } else {
                    (
                        egui::Color32::from_rgba_unmultiplied(20, 30, 60, 30),
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 190),
                    )
                };
                wide.line_segment(
                    [
                        egui::pos2(screen.left(), screen.top() + 0.5),
                        egui::pos2(screen.right(), screen.top() + 0.5),
                    ],
                    egui::Stroke::new(1.0_f32, edge_c),
                );
                let bar_bottom = full.bottom() + 2.0; // 补回 inner_margin 的下半
                vgradient(
                    &wide,
                    egui::Rect::from_min_max(
                        egui::pos2(screen.left(), bar_bottom),
                        egui::pos2(screen.right(), bar_bottom + 12.0),
                    ),
                    fade_c,
                    egui::Color32::TRANSPARENT,
                );
            });

        // 左侧游戏列表：透明面板 + 左右内边距（卡片不贴窗口边缘）
        egui::SidePanel::left("list")
            .resizable(true)
            .default_width(310.0)
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(12.0_f32, 8.0_f32)),
            )
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.heading(egui::RichText::new("游戏列表").color(text_main()));
                // 扫描中显示 spinner（快转 spinner 让加载显得更快）；完成后对勾淡出
                if self.scan_rx.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("扫描并抓取封面中…").color(text_sub()));
                    });
                } else if self.scan_done_t > 0.0 {
                    // 完成提示：对勾 + 文字，随剩余时长整体淡出（1.2s 后自然消失）
                    let k = ease(self.scan_done_t.min(1.0));
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(
                            egui::vec2(14.0, 14.0),
                            egui::Sense::hover(),
                        );
                        let p = ui.painter();
                        let ok = egui::Color32::from_rgb(94, 200, 140);
                        // 手绘对勾（Lucide check），两段圆头线
                        let pt = |x: f32, y: f32| {
                            pos2(r.left() + x / 14.0 * r.width(), r.top() + y / 14.0 * r.height())
                        };
                        let st = egui::Stroke::new(1.8_f32, fa(ok, k));
                        p.line_segment([pt(2.0, 7.6), pt(5.6, 11.0)], st);
                        p.line_segment([pt(5.6, 11.0), pt(12.0, 3.4)], st);
                        ui.label(
                            egui::RichText::new("扫描完成")
                                .color(fa(text_sub(), k))
                                .size(12.5),
                        );
                    });
                }
                ui.add_space(2.0);
                let dir = self.library_dir.clone();
                let n = self.games.len();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(2.0);
                    // 正在计时的游戏名（按显示名比对，下标可能因扫描而变）
                    let running_names: std::collections::HashSet<String> =
                        self.running.iter().map(|r| r.title.clone()).collect();
                    for i in 0..n {
                        let tex = ensure_cover(
                            ctx,
                            &dir,
                            &mut self.textures,
                            &mut self.avg,
                            &self.games[i],
                        )
                        .unwrap_or_else(|| placeholder_tex(ctx, &mut self.placeholder));
                        let is_sel = self.selected == Some(i);
                        let tint = self
                            .avg
                            .get(&cover_key(&self.games[i]))
                            .copied()
                            .unwrap_or_else(|| {
                                if is_dark() {
                                    egui::Color32::from_rgb(58, 64, 82)
                                } else {
                                    egui::Color32::from_rgb(206, 213, 230)
                                }
                            });
                        // 错峰入场：按序号给一点延迟（上限 0.5s，游戏多了也不会等太久）
                        let delay = (i as f32 * 0.02).min(0.5);
                        let appear = ease(((self.list_t - delay) / 0.25).clamp(0.0, 1.0));
                        let is_running = running_names.contains(display_name(&self.games[i]));
                        let resp =
                            game_card(ui, &tex, &self.games[i], i, is_sel, tint, appear, is_running);
                        let clicked = resp.clicked();
                        // tooltip：显示名 + 原始名（不同时才展示，方便认出是哪个文件夹）
                        let shown = display_name(&self.games[i]).to_string();
                        let tip = match self.games[i].name == shown {
                            true => shown,
                            false => format!("{}\n原始：{}", shown, self.games[i].name),
                        };
                        resp.clone().on_hover_text(tip);
                        // 右键菜单：改名重匹配 / 设置别名
                        resp.context_menu(|ui| {
                            if ui.button("✏ 重命名并重新匹配…").clicked() {
                                self.rename_buf = self.games[i].name.clone();
                                self.rename_idx = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("🏷 设置别名…").clicked() {
                                self.alias_buf =
                                    self.games[i].alias.clone().unwrap_or_default();
                                self.alias_idx = Some(i);
                                ui.close_menu();
                            }
                        });
                        if clicked {
                            self.selected = Some(i);
                            self.editing = false;
                            self.load_edit_buffer(i);
                            self.settle = 6; // 补几帧：Grid 首帧不可见、背景要换图
                        }
                        ui.add_space(6.0);
                    }
                });
                if n == 0 {
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new("还没有游戏").color(text_main()));
                    ui.label(
                        egui::RichText::new("点上方「选择游戏文件夹」开始扫描你的单机游戏。")
                            .color(text_sub())
                            .size(13.0),
                    );
                }
            });

        // 右侧详情：透明面板 + 内边距（卡片与分隔线/右缘保持间距）
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(14.0_f32, 10.0_f32)),
            )
            .show(ctx, |ui| {
                if let Some(i) = self.selected {
                    // 详情入场：整块内容按 detail_t 淡入（令牌函数自动吃全局透明度），
                    // 内容同时自下而上落定 8px。用完必须还原成 1.0，
                    // 否则同一帧后面画的弹层/状态栏会一起变淡。
                    let de = ease(self.detail_t);
                    set_ui_alpha(de);
                    self.detail_ui(ctx, ui, i, de);
                    set_ui_alpha(1.0);
                } else {
                    // 空状态入场：欢迎卡淡入 + 从下方 10px 落定
                    let ie = ease(self.intro_t);
                    ui.add_space(20.0 + (1.0 - ie) * 10.0);
                    set_ui_alpha(ie);
                    // 空状态：居中玻璃欢迎卡
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                        ui.set_max_width(460.0_f32.min(ui.available_width()));
                        glass_frame().show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new("游戏柜").heading().color(text_main()));
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(
                                        "从左侧选择一个游戏查看 / 编辑，\n或点「选择游戏文件夹」开始扫描你的单机游戏。",
                                    )
                                    .color(text_sub())
                                    .size(14.0),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new(
                                        "扫描会自动按游戏名从 Steam 抓取封面，存进每个游戏文件夹下的 .cabinet。",
                                    )
                                    .color(text_sub())
                                    .size(12.5),
                                );
                                ui.label(
                                    egui::RichText::new(
                                        "支持：状态(在玩/通关/弃坑…)、进度%、评分、时长、备注，全部存进 library.json。",
                                    )
                                    .color(text_sub())
                                    .size(12.5),
                                );
                                ui.add_space(8.0);
                            });
                        });
                    });
                    set_ui_alpha(1.0); // 还原全局透明度（弹层/状态栏不能被一起淡掉）
                }
            });

        // ---- 右键弹窗：重命名并重新匹配 ----
        if let Some(i) = self.rename_idx {
            let mut open = true;
            egui::Window::new("重命名并重新匹配")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .frame(dialog_frame())
                .anchor(egui::Align2::CENTER_CENTER, [0.0, -60.0])
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new("输入游戏名，将从 Steam 搜索并抓取封面。")
                            .size(13.0),
                    );
                    ui.add_space(4.0);
                    let enter = {
                        let r = ui.add_sized(
                            [340.0_f32, 30.0_f32],
                            egui::TextEdit::singleline(&mut self.rename_buf),
                        );
                        r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    };
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, "搜索并匹配").clicked() || enter {
                            let term = self.rename_buf.trim().to_string();
                            if term.is_empty() {
                                self.status = "名称为空".into();
                            } else {
                                if let Some(g) = self.games.get_mut(i) {
                                    g.name = term.clone();
                                }
                                self.rename_idx = None;
                                self.start_rematch(i, term);
                            }
                        }
                        if ui.button("取消").clicked() {
                            self.rename_idx = None;
                        }
                    });
                });
            if !open {
                self.rename_idx = None;
            }
        }

        // ---- 右键弹窗：设置别名 ----
        if let Some(i) = self.alias_idx {
            let mut open = true;
            egui::Window::new("设置别名")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .frame(dialog_frame())
                .anchor(egui::Align2::CENTER_CENTER, [0.0, -60.0])
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "别名会优先显示在列表中；留空保存 = 清除别名，回退显示匹配名。",
                        )
                        .size(13.0),
                    );
                    ui.add_space(4.0);
                    let enter = {
                        let r = ui.add_sized(
                            [340.0_f32, 30.0_f32],
                            egui::TextEdit::singleline(&mut self.alias_buf),
                        );
                        r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    };
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, "保存别名").clicked() || enter {
                            let a = self.alias_buf.trim().to_string();
                            if let Some(g) = self.games.get_mut(i) {
                                g.alias = if a.is_empty() { None } else { Some(a) };
                            }
                            self.alias_idx = None;
                            let lib = crate::library::Library { games: self.games.clone() };
                            match lib.save(&self.library_path) {
                                Ok(()) => self.status = "已保存".into(),
                                Err(e) => self.status = format!("保存失败: {e}"),
                            }
                        }
                        if ui.button("取消").clicked() {
                            self.alias_idx = None;
                        }
                    });
                });
            if !open {
                self.alias_idx = None;
            }
        }
    }
    // 窗口清屏为全透明：让系统亚克力模糊透出桌面
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

/// 确保封面纹理已加载（用 image crate 解码本地封面文件），返回纹理句柄
fn ensure_cover(
    ctx: &egui::Context,
    library_dir: &Path,
    textures: &mut std::collections::HashMap<String, egui::TextureHandle>,
    avg: &mut std::collections::HashMap<String, egui::Color32>,
    game: &GameEntry,
) -> Option<egui::TextureHandle> {
    let key = cover_key(game);
    if let Some(t) = textures.get(&key) {
        return Some(t.clone());
    }
    let local = game.cover_local.as_ref()?;
    let path = library_dir.join(local);
    let img = image::open(&path).ok()?;
    let rgba = img.to_rgba8();
    // 平均色（列表卡片的毛玻璃底用）：缩到 1x1 等价于求整图均值，几乎零成本
    if !avg.contains_key(&key) {
        let one = image::imageops::resize(&rgba, 1, 1, image::imageops::FilterType::Triangle);
        let px = one.get_pixel(0, 0);
        avg.insert(key.clone(), egui::Color32::from_rgb(px[0], px[1], px[2]));
    }
    let size = [rgba.width() as usize, rgba.height() as usize];
    let ci = egui::ColorImage::from_rgba_unmultiplied(size, &rgba.into_raw());
    let tex = ctx.load_texture(&key, ci, egui::TextureOptions::LINEAR);
    textures.insert(key, tex.clone());
    Some(tex)
}

fn cover_key(g: &GameEntry) -> String {
    g.cover_local
        .clone()
        .unwrap_or_else(|| g.name.clone())
}
