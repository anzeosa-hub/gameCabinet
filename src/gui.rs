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

const STATUSES: [&str; 5] = ["未开始", "在玩", "已通关", "弃坑", "想玩"];

// ---- 玻璃设计令牌（Apple 风：SF 蓝 + 冷灰夜色） ----
// from_rgba_unmultiplied 非 const，这里用函数而非常量
fn glass_fill() -> egui::Color32 {
    // 卡片底：低不透明度深色（透出亚克力模糊层，玻璃感优先；文字可读性由系统暗色 tint 兜底）
    egui::Color32::from_rgba_unmultiplied(20, 24, 34, 70)
}
fn glass_fill_hover() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(42, 48, 62, 150)
}
fn glass_stroke() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 22)
}
fn text_sub() -> egui::Color32 {
    // 次要文字：保持较高不透明度，透在亮壁纸上也要可读（对比度规范）
    egui::Color32::from_rgba_unmultiplied(226, 232, 246, 210)
}
const TEXT_MAIN: egui::Color32 = egui::Color32::from_rgb(240, 244, 252);

// 控件底色：必须自带足够不透明度的深色，否则亮壁纸透过来会把浅色字吞掉
fn button_fill() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(34, 38, 50, 185)
}
fn button_fill_hover() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(54, 60, 78, 205)
}
fn button_fill_active() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(22, 26, 36, 225)
}
fn accent_fill() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(10, 132, 255, 240)
}

/// 状态 → 颜色
fn status_color(s: &str) -> egui::Color32 {
    match s {
        "在玩" => egui::Color32::from_rgb(80, 200, 120),
        "已通关" => egui::Color32::from_rgb(90, 160, 240),
        "弃坑" => egui::Color32::from_rgb(230, 110, 110),
        "想玩" => egui::Color32::from_rgb(230, 190, 90),
        _ => egui::Color32::from_gray(160),
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

/// 弹窗玻璃框：近不透明深色 + 大圆角 + 细描边 + 悬浮阴影
/// （默认 Window 框太浅，标题栏和正文对比度都不够，统一换成这张）
fn dialog_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(egui::Color32::from_rgba_unmultiplied(20, 22, 32, 242))
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

/// 状态胶囊：状态色淡底 + 同色描边 + 同色字（详情页只读展示用）
fn status_pill(ui: &mut egui::Ui, s: &str) {
    let c = status_color(s);
    egui::Frame::none()
        .fill(egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 40))
        .stroke(egui::Stroke::new(
            1.0_f32,
            egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 140),
        ))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(9.0, 2.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(s).size(12.5).color(c));
        });
}

/// 细进度条（只读展示：白轨道 + 状态色填充），宽度吃满所在容器
fn thin_progress(ui: &mut egui::Ui, progress: u8, color: egui::Color32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 6.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26));
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
    open_dir: bool,
}

/// 详情操作按钮组。
/// 注意：调用方若是 right_to_left 布局，先添加的按钮显示在更右侧。
fn detail_action_buttons(
    ui: &mut egui::Ui,
    editing: bool,
    has_exe: bool,
    has_dir: bool,
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

    // 编辑态下「保存」已经是主色按钮，启动就不再抢主色，避免两个蓝按钮打架
    let mut launch_btn = if editing {
        ui.button("▶ 启动游戏")
    } else {
        primary_button(ui, "▶ 启动游戏")
    };
    if !has_exe {
        launch_btn = launch_btn.on_hover_text("该条目未记录可执行文件（可右键左侧卡片「重命名并重新匹配」）");
    }
    if launch_btn.clicked() {
        a.launch = true;
    }

    a
}

/// 详情页只读展示用的数据快照（避免在闭包里同时持有 &self.games 与 &mut self.edit_*）
struct InfoSnap {
    status: String,
    progress: u8,
    rating: u8,
    playtime: f32,
    note: String,
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

/// 应用玻璃主题
fn apply_theme(ctx: &egui::Context) {
    // 必须从纯深色主题出发，不能克隆当前主题再改 dark_mode：
    // egui 默认 Style 是浅色（各状态 weak_bg_fill 都是 230+ 的白灰），
    // dark_mode 只是个标志位，不会重算已固化的底色字段。
    let mut v = egui::Visuals::dark();
    // 全局兜底：任何没显式指定颜色的文字一律浅色，杜绝「浅底配浅字」
    v.override_text_color = Some(TEXT_MAIN);
    // 面板底：低不透明度，保持通透
    v.panel_fill = egui::Color32::from_rgba_unmultiplied(14, 16, 24, 60);
    v.window_fill = egui::Color32::from_rgba_unmultiplied(24, 26, 36, 200);
    v.extreme_bg_color = egui::Color32::from_rgba_unmultiplied(10, 12, 18, 130);
    // 控件（按钮/下拉/输入框）：深色玻璃底 + 纯白字，绝不许浅底配浅字
    v.widgets.noninteractive.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 8);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, text_sub());
    // 分隔线（面板拖拽分隔条 / ui.separator）走 bg_stroke：默认的灰白太抢眼，
    // 会把「整窗一张连贯背景」硬生生切成左右两块，改成极淡的白
    v.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26));
    v.widgets.inactive.bg_fill = button_fill();
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.4_f32, egui::Color32::WHITE);
    v.widgets.hovered.bg_fill = button_fill_hover();
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.4_f32, egui::Color32::WHITE);
    v.widgets.active.bg_fill = button_fill_active();
    v.widgets.active.fg_stroke = egui::Stroke::new(1.4_f32, egui::Color32::WHITE);
    v.widgets.open.bg_fill = button_fill();
    v.widgets.open.fg_stroke = egui::Stroke::new(1.4_f32, egui::Color32::WHITE);
    // 关键：Button / ComboBox / 弹窗标题栏的默认底色走的是 weak_bg_fill，不是 bg_fill。
    // 只改 bg_fill 的话，这些控件会保留浅色主题的白灰底（230/248），配白字直接吞掉。
    v.widgets.noninteractive.weak_bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 10);
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
            .with_transparent(true),
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
    edit_status: String,
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
            edit_status: "未开始".into(),
            edit_progress: 0,
            edit_rating: 0,
            edit_playtime: String::new(),
            edit_note: String::new(),
            editing: false,
            rename_idx: None,
            rename_buf: String::new(),
            alias_idx: None,
            alias_buf: String::new(),
        }
    }
}

impl CabinetApp {
    /// 选中游戏时，把它的字段载入编辑缓冲
    fn load_edit_buffer(&mut self, i: usize) {
        if let Some(g) = self.games.get(i) {
            self.edit_status = g.status.clone();
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
                g.status = self.edit_status.clone();
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
                    let covers_dir = dir.join("covers");
                    std::fs::create_dir_all(&covers_dir)?;
                    let base = if c.source == "sgdb" {
                        format!("sgdb_{}", c.id)
                    } else {
                        c.id.to_string()
                    };
                    let cover_path = covers_dir.join(format!("{base}_cover.jpg"));
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
                        cover_local: if ok {
                            Some(format!("covers/{base}_cover.jpg"))
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
    fn launch_exe(&mut self, i: usize, title: &str) {
        match self.games.get(i).and_then(|g| g.executable.clone()) {
            Some(exe) => {
                let mut cmd = std::process::Command::new(&exe);
                if let Some(dir) = Path::new(&exe).parent() {
                    cmd.current_dir(dir);
                }
                match cmd.spawn() {
                    Ok(_) => self.status = format!("已启动：{title}"),
                    Err(e) => self.status = format!("启动失败：{e}"),
                }
            }
            None => {
                self.status = "该条目没有可执行文件，右键左侧卡片「重命名并重新匹配」试试".into();
            }
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
                let covers = dir.join("covers");
                std::fs::create_dir_all(&covers).ok()?;
                let path = covers.join(format!("{appid}_hero.jpg"));
                crate::steam::download_cover(&client, &url, &path).await.ok()?;
                // 没有这张图时 CDN 会甩一个很小的占位/HTML 过来，按体积筛掉
                let sz = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                if sz < 15_000 {
                    let _ = std::fs::remove_file(&path);
                    return None;
                }
                Some(HeroMsg {
                    idx: i,
                    rel: format!("covers/{appid}_hero.jpg"),
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
            if let (Some(stem), Some(parent)) =
                (p.file_stem().and_then(|s| s.to_str()), p.parent())
            {
                if let Some(base) = stem.strip_suffix("_cover") {
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
        // BorderDarken：整体压暗保证面板文字对比度。比上一版轻得多 —— 上一版 alpha 152~216
        // 直接把画面压成黑灰，只剩详情区那块没被压到的图显得突兀，硬生生切出一条卡片边界
        vgradient(
            &p,
            screen,
            egui::Color32::from_rgba_unmultiplied(10, 12, 22, 92),
            egui::Color32::from_rgba_unmultiplied(6, 8, 16, 168),
        );
    }

    /// 右侧详情区：自适应内容（宽屏左右双栏 / 窄屏上下堆叠）
    fn detail_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, i: usize) {
        // 先取好只读数据，后面要 &mut self 处理按钮动作，不能跨借用
        let title = display_name(&self.games[i]).to_string();
        let raw_name = self.games[i].name.clone();
        let status = self.games[i].status.clone();
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
                ui.add_space(top_indent);
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
                    tp.galley(
                        tr_rect.min + egui::vec2(0.0, 2.0),
                        tgal.clone(),
                        egui::Color32::from_black_alpha(170),
                    );
                    tp.galley(
                        tr_rect.min + egui::vec2(0.0, 1.0),
                        tgal.clone(),
                        egui::Color32::from_black_alpha(110),
                    );
                    tp.galley(tr_rect.min, tgal, TEXT_MAIN);
                    tr_resp.on_hover_text(&title);

                    left.add_space(4.0);
                    left.horizontal(|ui| {
                        status_pill(ui, &status);
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
                        act = detail_action_buttons(ui, editing, has_exe, has_dir);
                    });
                }

                // 右列：封面（清晰、只加圆角与投影，不糊）
                {
                    let c_rect = egui::Rect::from_min_max(
                        egui::pos2(head_rect.right() - cw, head_rect.bottom() - ch),
                        head_rect.max,
                    );
                    let p = ui.painter().clone();
                    p.add(egui::epaint::RectShape::filled(
                        c_rect.translate(egui::vec2(0.0, 8.0)).expand2(egui::vec2(6.0, 6.0)),
                        egui::Rounding::same(11.0),
                        egui::Color32::from_black_alpha(90),
                    ));
                    let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    tex_round_rect(
                        &p,
                        c_rect,
                        &tex,
                        uv,
                        egui::Rounding::same(10.0),
                        egui::Color32::WHITE,
                    );
                    p.rect_stroke(
                        c_rect,
                        10.0,
                        egui::Stroke::new(
                            1.0_f32,
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40),
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
                let snap = InfoSnap {
                    status: self.games[i].status.clone(),
                    progress: self.games[i].progress,
                    rating: self.games[i].rating,
                    playtime: self.games[i].playtime_hours,
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
            ui.label(egui::RichText::new("游戏详情").size(14.0).strong().color(TEXT_MAIN));
            ui.add_space(2.0);
            ui.separator();
            ui.add_space(4.0);
            if self.editing {
                prop_row(ui, "状态", |ui| {
                    egui::ComboBox::from_id_source("edit_status")
                        .selected_text(&self.edit_status)
                        .width((w - 130.0).clamp(96.0, 160.0))
                        .show_ui(ui, |ui| {
                            for st in STATUSES {
                                ui.selectable_value(&mut self.edit_status, st.to_string(), st);
                            }
                        });
                });
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
            prop_row(ui, "状态", |ui| {
                status_pill(ui, &s.status);
            });
            prop_row(ui, "进度", |ui| {
                let (t, c) = if s.progress > 0 {
                    (format!("{} %", s.progress), TEXT_MAIN)
                } else {
                    ("—".to_string(), text_sub())
                };
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(t).color(c).size(13.5));
                    if s.progress > 0 {
                        ui.add_space(4.0);
                        thin_progress(ui, s.progress, status_color(&s.status));
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
                let (tt, tc) = if s.playtime > 0.0 {
                    (format!("{} 小时", s.playtime), TEXT_MAIN)
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
                            egui::Label::new(egui::RichText::new(name).color(TEXT_MAIN).size(13.0))
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
            ui.label(egui::RichText::new("备注").size(14.0).strong().color(TEXT_MAIN));
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
                ui.label(egui::RichText::new(&s.note).color(TEXT_MAIN).size(14.0));
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
    is_sel: bool,
    // tint = 该游戏封面的平均色：用来做卡片自带的毛玻璃底，卡片透出自身封面的虚化色
    tint: egui::Color32,
) -> egui::Response {
    let avail = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail, 78.0), egui::Sense::click());
    // 卡片底：选中 > hover > 默认
    let hovered = ui.rect_contains_pointer(rect);
    let fill = if is_sel {
        egui::Color32::from_rgba_unmultiplied(10, 132, 255, 46)
    } else if hovered {
        glass_fill_hover()
    } else {
        glass_fill()
    };
    let p = ui.painter().clone();
    p.rect_filled(rect, 12.0, fill);
    // 封面虚化色叠加：卡片自带一层自身封面的毛玻璃底（选中/hover 时更明显）
    let tint_a = if is_sel { 96 } else if hovered { 68 } else { 42 };
    p.rect_filled(
        rect,
        12.0,
        egui::Color32::from_rgba_unmultiplied(
            (tint.r() as f32 * 0.62) as u8,
            (tint.g() as f32 * 0.62) as u8,
            (tint.b() as f32 * 0.68) as u8,
            tint_a,
        ),
    );
    let stroke = if is_sel {
        egui::Stroke::new(1.2_f32, egui::Color32::from_rgb(120, 180, 255))
    } else if hovered {
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 45))
    } else {
        egui::Stroke::new(1.0_f32, glass_stroke())
    };
    p.rect_stroke(rect, 12.0, stroke);

    // 封面缩略：圆角 + 柔化投影（egui 的 RectShape 没有 blur_width，用两层递减 alpha 叠出软边）
    let pad = 10.0_f32;
    let th = 58.0_f32;
    let sz = tex.size();
    let aspect = sz[1] as f32 / sz[0] as f32;
    let tw = (th / aspect).min(62.0);
    let img_rect = egui::Rect::from_min_size(rect.min + egui::vec2(pad, pad), egui::vec2(tw, th));
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    p.add(egui::epaint::RectShape::filled(
        img_rect.translate(egui::vec2(0.0, 3.0)).expand2(egui::vec2(3.5, 3.5)),
        egui::Rounding::same(9.0),
        egui::Color32::from_black_alpha(26),
    ));
    p.add(egui::epaint::RectShape::filled(
        img_rect.translate(egui::vec2(0.0, 1.5)).expand2(egui::vec2(1.5, 1.5)),
        egui::Rounding::same(8.0),
        egui::Color32::from_black_alpha(38),
    ));
    tex_round_rect(&p, img_rect, tex, uv, egui::Rounding::same(7.0), egui::Color32::WHITE);
    p.rect_stroke(
        img_rect,
        7.0,
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30)),
    );

    // 名字（显示名 = 别名 > 匹配名 > 原始名；按像素宽截断 + 省略号，全名靠 hover tooltip）
    let title = display_name(g).to_string();
    let tx = img_rect.right() + 12.0;
    let max_w = rect.right() - 12.0 - tx;
    let name_gal = fit_galley(&p, &title, 15.0, TEXT_MAIN, max_w);
    p.galley(pos2(tx, rect.top() + 11.0), name_gal, TEXT_MAIN);

    // 状态 + 进度 + 评分
    let mut meta = g.status.clone();
    if g.progress > 0 {
        meta.push_str(&format!("　{}%", g.progress));
    }
    if g.rating > 0 {
        meta.push_str(&format!("　★{}", g.rating));
    }
    let meta_gal = fit_galley(&p, &meta, 12.5, status_color(&g.status), max_w);
    p.galley(pos2(tx, rect.top() + 33.0), meta_gal, status_color(&g.status));

    // 细进度条（玻璃感：白色轨道 + 状态色填充）
    if g.progress > 0 {
        let bar_y = rect.bottom() - 11.0;
        let track = egui::Rect::from_min_max(pos2(tx, bar_y), pos2(rect.right() - 12.0, bar_y + 3.0));
        p.rect_filled(track, 1.5, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 24));
        let w = track.width() * g.progress as f32 / 100.0;
        if w > 1.0 {
            let fill_r = egui::Rect::from_min_max(track.min, pos2(track.min.x + w, track.max.y));
            p.rect_filled(fill_r, 1.5, status_color(&g.status));
        }
    }
    resp
}

/// Windows 亚克力模糊（真·系统级透明，透出桌面）：直接走 user32 的
/// SetWindowCompositionAttribute（undocumented 但稳定，Tauri 的 window-vibrancy 同款实现）
#[cfg(windows)]
fn apply_windows_acrylic() {
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
        // gradient_color 为 ABGR：深色底 + 中低 alpha，让桌面的虚化透上来但不至于把画面压死
        let tint = (112u32 << 24) | (26 << 16) | (22 << 8) | 14;
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

        // 首帧：窗口已存在，应用系统亚克力模糊
        if !self.vibrancy_done {
            self.vibrancy_done = true;
            #[cfg(windows)]
            apply_windows_acrylic();
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

        // 顶部工具栏：半透明深色玻璃条（保证按钮/路径文字可读，同时别把背景图挡死）
        egui::TopBottomPanel::top("top")
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_rgba_unmultiplied(12, 14, 22, 172))
                    .inner_margin(egui::Margin::symmetric(14.0, 9.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if primary_button(ui, "选择游戏文件夹").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.start_scan(dir);
                        }
                    }
                    if ui.button("打开库").clicked() {
                        if let Some(p) =
                            rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file()
                        {
                            self.open_library(p);
                        }
                    }
                    if ui.button("重新扫描").clicked() {
                        if !self.library_dir.as_os_str().is_empty() {
                            self.start_scan(self.library_dir.clone());
                        }
                    }
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("库：{}", self.library_path.display()))
                            .color(text_sub())
                            .size(12.5),
                    );
                    // 状态色：扫描中偏黄、完成偏绿
                    let st_color = if self.scan_rx.is_some() {
                        egui::Color32::from_rgb(230, 190, 90)
                    } else {
                        egui::Color32::from_rgb(120, 200, 140)
                    };
                    ui.colored_label(st_color, egui::RichText::new(&self.status).size(12.5));
                });
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
                ui.heading(egui::RichText::new("游戏列表").color(TEXT_MAIN));
                // 扫描中显示 spinner（快转 spinner 让加载显得更快）
                if self.scan_rx.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("扫描并抓取封面中…").color(text_sub()));
                    });
                }
                ui.add_space(2.0);
                let dir = self.library_dir.clone();
                let n = self.games.len();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(2.0);
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
                            .unwrap_or_else(|| egui::Color32::from_rgb(58, 64, 82));
                        let resp = game_card(ui, &tex, &self.games[i], is_sel, tint);
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
                    ui.label(egui::RichText::new("还没有游戏").color(TEXT_MAIN));
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
                    self.detail_ui(ctx, ui, i);
                } else {
                    ui.add_space(20.0);
                    // 空状态：居中玻璃欢迎卡
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                        ui.set_max_width(460.0_f32.min(ui.available_width()));
                        glass_frame().show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new("游戏柜").heading().color(TEXT_MAIN));
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
                                        "扫描会自动按游戏名从 Steam 抓取封面，下载到 <游戏目录>/covers。",
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
