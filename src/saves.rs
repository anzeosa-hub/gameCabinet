//! 游戏存档位置探测。
//!
//! 设计前提（本项目定位）：全部操作**只在本机文件系统内完成**，不联网、不查任何在线清单。
//! 存档是玩家在本地玩过之后由游戏自己写出来的文件，因此只能通过「读本地目录」来定位。
//!
//! 三条定位路径，确定性从高到低：
//!   1. 引擎声明 —— Unity 的 `_Data\app.info` 直接写明公司名与产品名，拼出来的路径是精确的；
//!      Unreal 的 exe 名自带项目名。这类是不用猜的。
//!   2. 引擎默认 —— Godot / GameMaker / Ren'Py / LÖVE / RPG Maker 等框架各有固定的写入目录，
//!      只需从游戏文件夹里认出框架指纹 + 取出游戏名。
//!   3. 名字兜底 —— 把游戏名洗一遍，到几个存档富集区（Saved Games / 文档 / AppData / 用户根目录 …）
//!      做包含匹配，列出候选交给人确认。
//!
//! 本模块**只读**，不做任何写操作（移动 / 联接 / 备份在后续阶段）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 存档根目录的**机器无关记号**。
///
/// 为什么不直接存 `C:\Users\zelun\AppData\LocalLow\...` 这种绝对路径：它把当前用户名焊死在里面，
/// 换机器 / 换用户名就失效。存成「根目录记号 + 相对路径」后，备份时按本机展开去读，
/// 还原时按新机器重新展开去写 —— 用户名不一致的问题顺带就消失了。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SaveRoot {
    /// `%LOCALAPPDATA%Low`（Unity 的默认位置）
    LocalLow,
    /// `%LOCALAPPDATA%`（Unreal / GameMaker / RPG Maker MV 的默认位置）
    Local,
    /// `%APPDATA%`（Godot / Ren'Py / LÖVE 的默认位置）
    Roaming,
    /// 「文档」（Steamworks 的 `WinMyDocuments`）
    Documents,
    /// `%USERPROFILE%\Saved Games`（Windows 的特殊目录，Steamworks 的 `WinSavedGames`）
    SavedGames,
    /// 用户根目录 `%USERPROFILE%`（Java 程序常用 `user.home`）
    Home,
    /// `%ProgramData%`
    ProgramData,
    /// 游戏安装目录本身（绿色版 / portable，重装不丢）
    Install,
}

impl SaveRoot {
    /// JSON 里写的记号名
    pub fn token(&self) -> &'static str {
        match self {
            SaveRoot::LocalLow => "locallow",
            SaveRoot::Local => "local",
            SaveRoot::Roaming => "roaming",
            SaveRoot::Documents => "documents",
            SaveRoot::SavedGames => "savedgames",
            SaveRoot::Home => "home",
            SaveRoot::ProgramData => "programdata",
            SaveRoot::Install => "install",
        }
    }

    /// 从记号名解析回来（手填的 api / 未来读老数据时用）
    pub fn from_token(s: &str) -> Option<SaveRoot> {
        Some(match s.trim().to_lowercase().as_str() {
            "locallow" | "local_low" | "winappdatalocallow" => SaveRoot::LocalLow,
            "local" | "localappdata" | "winappdatalocal" => SaveRoot::Local,
            "roaming" | "appdata" | "winappdataroaming" => SaveRoot::Roaming,
            "documents" | "mydocuments" | "winmydocuments" => SaveRoot::Documents,
            "savedgames" | "winsavedgames" | "saved games" => SaveRoot::SavedGames,
            "home" | "userprofile" => SaveRoot::Home,
            "programdata" => SaveRoot::ProgramData,
            "install" | "installdir" | "winappinstall" => SaveRoot::Install,
            _ => return None,
        })
    }

    /// 在本机把这个记号展开成真实目录。`install_dir` 仅 `Install` 用得上（游戏文件夹）。
    ///
    /// 注：Windows 上「文档」「保存的游戏」的真实位置存在注册表里（可能被重定向到 OneDrive），
    /// 走 `SHGetKnownFolderPath` 才严谨。这里不为了它引入 winapi 依赖，
    /// 用 `USERPROFILE` 拼 + OneDrive 兜底的近似做法，已能覆盖绝大多数人的机器。
    pub fn base(&self, install_dir: Option<&Path>) -> Option<PathBuf> {
        let home = |k: &str| std::env::var(k).ok().map(PathBuf::from);
        match self {
            SaveRoot::Local => home("LOCALAPPDATA"),
            SaveRoot::Roaming => home("APPDATA"),
            // LOCALAPPDATA = ...\AppData\Local，它的同级兄弟就是 LocalLow
            SaveRoot::LocalLow => home("LOCALAPPDATA")
                .and_then(|l| Path::new(&l).parent().map(|p| p.join("LocalLow")))
                .or_else(|| home("USERPROFILE").map(|h| h.join("AppData").join("LocalLow"))),
            SaveRoot::Documents => home("USERPROFILE").map(|h| documents_of(&h)),
            SaveRoot::SavedGames => home("USERPROFILE").map(|h| h.join("Saved Games")),
            SaveRoot::Home => home("USERPROFILE"),
            SaveRoot::ProgramData => home("ProgramData")
                .or_else(|| home("ALLUSERSPROFILE"))
                .or_else(|| home("USERPROFILE").map(|h| h.join("AppData").join("Local"))),
            SaveRoot::Install => install_dir.map(|p| p.to_path_buf()),
        }
    }
}

/// 「文档」目录：优先常规位置，被 OneDrive 接管时回落到 OneDrive 下那份
fn documents_of(home: &Path) -> PathBuf {
    let d = home.join("Documents");
    if d.exists() {
        return d;
    }
    let od = home.join("OneDrive").join("Documents");
    if od.exists() {
        return od;
    }
    d
}

/// 一条存档位置规格：`根目录记号 + 相对该根的路径`。
///
/// 序列化到 `library.json` 里的形态：`{ "root": "locallow", "rel": "Dumativa\\Enigma" }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveSpec {
    pub root: SaveRoot,
    pub rel: String,
}

impl SaveSpec {
    /// 在本机展开成真实路径
    pub fn resolve(&self, install_dir: Option<&Path>) -> Option<PathBuf> {
        let b = self.root.base(install_dir)?;
        if self.rel.trim().is_empty() {
            Some(b)
        } else {
            Some(b.join(&self.rel))
        }
    }
}

/// 把一个本机绝对路径**反向**拆成 `SaveSpec`（能拆就拆，拆不动原样保留）。
///
/// 用于「用户手动浏览选择了某个文件夹」的场景：记下来时顺便换成记号形式，
/// 免得把 `C:\Users\zelun\...` 这种带用户名的路径焊进库里。
pub fn spec_from_path(p: &Path, install_dir: Option<&Path>) -> SaveSpec {
    let all = [
        SaveRoot::Install,
        SaveRoot::LocalLow,
        SaveRoot::Local,
        SaveRoot::Roaming,
        SaveRoot::SavedGames,
        SaveRoot::Documents,
        SaveRoot::ProgramData,
        SaveRoot::Home,
    ];
    for r in all {
        if let Some(b) = r.base(install_dir) {
            if let Ok(rel) = p.strip_prefix(&b) {
                return SaveSpec {
                    root: r,
                    rel: rel.to_string_lossy().into_owned(),
                };
            }
        }
    }
    // 落在已知目录之外：整条路径原样记着（换成当前用户名下的绝对路径）
    SaveSpec {
        root: SaveRoot::Home,
        rel: p.to_string_lossy().into_owned(),
    }
}

/// 这条候选是怎么来的——决定置信度，也决定 UI 上怎么标注
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// 引擎写死了路径，直接读出来的（Unity app.info / Unreal 项目名）
    Declared,
    /// 框架默认 + 游戏名推断（Godot / GameMaker / …）
    Engine,
    /// 在各富集区按名字捞出来的，纯猜测
    Guess,
    /// 存档就在游戏安装目录里（绿色版）
    Portable,
}

impl Origin {
    pub fn label(&self) -> &'static str {
        match self {
            Origin::Declared => "引擎声明",
            Origin::Engine => "引擎默认",
            Origin::Guess => "名称推测",
            Origin::Portable => "就在游戏目录",
        }
    }
}

/// 一个候选存档位置
#[derive(Debug, Clone)]
pub struct Candidate {
    /// 机器无关规格（写库用）
    pub spec: SaveSpec,
    /// 本机展开后的真实路径（显示 / 复制用）
    pub path: PathBuf,
    /// 本机是否存在
    pub exists: bool,
    pub origin: Origin,
    /// 置信度 0-100
    pub score: u8,
    /// 一句话说明依据（为什么觉得是这里）
    pub why: String,
    /// 目录统计：仅在 `exists` 时才有意义
    pub files: u32,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
}

/// 一次完整探测的结果
#[derive(Debug, Clone, Default)]
pub struct ProbeResult {
    /// 认出来的引擎 / 框架（给人看的描述）
    pub engine: Option<String>,
    /// 引擎给的关键参数（Unity 的厂商 / 产品名、Unreal 的项目名等）
    pub params: Vec<(String, String)>,
    pub candidates: Vec<Candidate>,
}

// ---------------------------------------------------------------- 目录枚举

struct Entry {
    name: String,
    path: PathBuf,
    dir: bool,
}

/// 这些目录要么是 Windows 的兼容性 junction（指向 AppData/Roaming 等同父目录，会造成无限递归），
/// 要么是系统/软件噪音，永远不会是游戏存档 —— 一律跳过。
const SKIP_NAMES: &[&str] = &[
    "application data",
    "cookies",
    "local settings",
    "my documents",
    "nethood",
    "printhood",
    "recent",
    "sendto",
    "start menu",
    "templates",
    "开始菜单",
    "appdata",
    "windows",
    "microsoft",
    "packages",
    "crashdumps",
    "unreal engine",
    "unrealenginelauncher",
    "$recycle.bin",
    // 软件自己建的、看着像但绝不是存档目录的那些
    "agreements",
    "crashreports",
    "crash report client",
    "nodereviser",
    "cachestorage",
];

fn skip_name(n: &str) -> bool {
    let l = n.to_lowercase();
    if l.starts_with('.') {
        return true;
    }
    SKIP_NAMES.contains(&l.as_str())
}

/// 有界枚举：最多 `max_nodes` 个节点、`max_depth` 层。
/// 目的是给「名字兜底」提供待匹配集合，不需要把整个 AppData 翻穿 —— 存档一般就在根目录的一两层内。
fn scan_tree(root: &Path, max_depth: usize, max_nodes: usize) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= max_nodes {
            break;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            if out.len() >= max_nodes {
                break;
            }
            let ft = match e.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            // junction / 符号链接一律跳过：既是去重手段，也避免循环
            if ft.is_symlink() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let is_dir = ft.is_dir();
            if is_dir {
                if skip_name(&name) {
                    continue;
                }
                if depth + 1 < max_depth {
                    stack.push((path.clone(), depth + 1));
                }
            }
            out.push(Entry {
                name,
                path,
                dir: is_dir,
            });
        }
    }
    out
}

/// 目录统计：文件数 / 总字节 / 最近修改时间。超大目录在 `cap` 处截断（避免整20GB回放目录慢慢数）。
fn dir_stats(dir: &Path, cap: usize) -> (u32, u64, Option<SystemTime>) {
    let mut files = 0u32;
    let mut bytes = 0u64;
    let mut newest: Option<SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            if files as usize >= cap {
                break;
            }
            let ft = match e.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                if !skip_name(&e.file_name().to_string_lossy()) {
                    stack.push(e.path());
                }
                continue;
            }
            files += 1;
            if let Ok(md) = e.metadata() {
                bytes += md.len();
                if let Ok(mt) = md.modified() {
                    if newest.map(|n| mt > n).unwrap_or(true) {
                        newest = Some(mt);
                    }
                }
            }
        }
    }
    (files, bytes, newest)
}

// ---------------------------------------------------------------- 名字清洗

/// 这些词出现在但愿这款游戏的存档目录名里也不说明什么 —— 太通用，会让「名称推测」大量误报
const STOP_WORDS: &[&str] = &[
    "game", "games", "the", "and", "data", "save", "saves", "saved", "savegame", "savegames",
    "config", "log", "logs", "temp", "tmp", "cache", "bin", "win", "win64", "x64", "x86", "ver",
    "update", "patch", "crack", "repack", "final", "full", "proper", "iso", "portable", "steam",
    "codex", "dodi", "fitgirl", "gog", "razor1911", "skidrow", "tenoke", "runrelease", "online",
    "fix", "multi", "nsfw", "dlc", "edition", "deluxe", "build", "release", "v1", "to",
    // 「Project」太通用：用它去匹配会把 XiaomiMiMoProjects、ProjectSettings 这类目录全捞上来
    "project", "settings", "version",
];

/// 洗游戏名：`ELDEN.RING.v1.09-DODI` → `elden ring`，`Project.Zomboid.Build.25414456` → `project zomboid`
fn clean_words(raw: &str) -> Vec<String> {
    // 发布组后缀常挂在 " - " 或 "-" 后面，取左侧主体部分（"Elden Ring - CODEX" → "Elden Ring"）
    let base = raw.split(" - ").next().unwrap_or(raw);
    let mut words = Vec::new();
    for tok in base.split(|c: char| c == '.' || c == '_' || c == ' ' || c == '-') {
        let t = tok.trim().to_lowercase();
        if t.is_empty() {
            continue;
        }
        // 版本号 / 构建号：出现即停。剩下的尾部全是发布信息
        let is_ver = t.chars().all(|c| c.is_ascii_digit())
            || (t.starts_with('v') && t.len() > 1 && t[1..].chars().next().is_some_and(|c| c.is_ascii_digit()));
        if is_ver || t == "build" {
            break;
        }
        if STOP_WORDS.contains(&t.as_str()) {
            continue;
        }
        // 单双字母没有辨识度
        if t.len() < 3 {
            continue;
        }
        words.push(t);
    }
    words
}

// ---------------------------------------------------------------- 引擎识别

/// 把 `Project.Zomboid.Build.24574865` 洗成 `Project Zomboid`（用于拼目录名 / 对外展示）
pub fn clean_display(raw: &str) -> String {
    let w = clean_words(raw);
    if w.is_empty() {
        raw.trim().to_string()
    } else {
        w.iter()
            .map(|s| capitalize_first(s))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

enum Engine {
    /// `_Data\app.info` 里写明厂商 + 产品名 → `%LOCALAPPDATA%Low\<厂商>\<产品名>`
    Unity { company: String, product: String },
    /// exe 名带 `-Win64-Shipping` → `%LOCALAPPDATA%\<项目>\Saved\SaveGames`
    Unreal { project: String },
    /// `%APPDATA%\Godot\app_userdata\<项目名>`
    Godot { project: String },
    /// `%LOCALAPPDATA%\<游戏名>`（开发者也可切到 AppData）
    GameMaker,
    /// `%APPDATA%\RenPy\<名字>`（常带随机数字后缀）
    RenPy,
    /// `%APPDATA%\LOVE\<identity>`
    Love,
    /// MV/MZ：`%LOCALAPPDATA%\<名字>`；VX Ace：常在安装目录内
    RpgMaker { mv: bool },
    /// `%APPDATA%\<名字>`
    NwJs,
    /// 常写 `user.home` 下自建目录
    Java,
}

/// 在游戏文件夹里找引擎指纹。只看前 3 层，够用了（`_Data` / `Engine\Binaries\Win64` 都在浅层）。
fn detect_engine(dir: &Path, exe: Option<&Path>) -> Option<Engine> {
    let entries = scan_tree(dir, 3, 1500);
    let lower_names: Vec<String> = entries.iter().map(|e| e.name.to_lowercase()).collect();
    let has_dir = |n: &str| {
        entries
            .iter()
            .any(|e| e.dir && e.name.eq_ignore_ascii_case(n))
    };
    let has_file = |n: &str| {
        entries
            .iter()
            .any(|e| !e.dir && e.name.eq_ignore_ascii_case(n))
    };
    let any_ext = |ext: &str| {
        entries
            .iter()
            .any(|e| !e.dir && e.name.to_lowercase().ends_with(ext))
    };
    let any_name_contains = |pat: &str| lower_names.iter().any(|n| n.contains(pat));

    // ---- Unity：`<X>_Data\app.info`（两行：厂商 / 产品名）----
    for e in entries.iter().filter(|e| e.dir) {
        if !e.name.to_lowercase().ends_with("_data") {
            continue;
        }
        let info = e.path.join("app.info");
        if let Ok(text) = std::fs::read_to_string(&info) {
            let mut lines = text
                .lines()
                .map(|l| l.trim_matches('\0').trim().to_string())
                .filter(|l| !l.trim().is_empty());
            let company = lines.next().unwrap_or_default();
            let product = lines.next().unwrap_or_default();
            if !company.is_empty() && !product.is_empty() {
                return Some(Engine::Unity { company, product });
            }
        }
    }

    // ---- Unreal ----
    let unreal = dir.join("Engine").join("Binaries").join("Win64").exists()
        || dir.join("Engine").exists() && any_ext(".pak")
        || any_name_contains("-win64-shipping")
        || any_name_contains("-wingdk-shipping");
    if unreal {
        let stem = exe
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| {
                entries
                    .iter()
                    .find(|e| {
                        !e.dir
                            && e.name.to_lowercase().ends_with(".exe")
                            && e.name.to_lowercase().contains("shipping")
                    })
                    .map(|e| Path::new(&e.name).file_stem().unwrap().to_string_lossy().into_owned())
            })
            .or_else(|| Some(dir.file_name()?.to_string_lossy().into_owned()))
            .unwrap_or_default();
        return Some(Engine::Unreal {
            project: unreal_project(&stem),
        });
    }

    // ---- Ren'Py：renpy / game / lib 三件套 ----
    if has_dir("renpy") && has_dir("game") && has_dir("lib") {
        return Some(Engine::RenPy);
    }

    // ---- GameMaker ----
    if has_file("data.win") {
        return Some(Engine::GameMaker);
    }

    // ---- RPG Maker MV/MZ（NW.js 打包）----
    if (has_dir("www") && any_name_contains("nw.dll"))
        || (has_file("package.json") && any_name_contains("nw.dll"))
    {
        return Some(Engine::RpgMaker { mv: true });
    }
    // VX Ace / XP：含 RGSS 运行库 → 存档一般在安装目录的 Save 目录
    if any_name_contains("rgss") || has_file("Game.ini") || has_file("Game.exe") {
        return Some(Engine::RpgMaker { mv: false });
    }

    // ---- NW.js / Electron ----
    if any_name_contains("nw.dll") || any_ext(".asar") || any_name_contains("resources/app.asar") {
        return Some(Engine::NwJs);
    }

    // ---- LÖVE ----
    if any_name_contains("love.dll") || any_ext(".love") {
        return Some(Engine::Love);
    }

    // ---- Godot：打包出的 .pck（或 pck 内嵌于 exe 时退化用 exe 名）----
    if any_ext(".pck") {
        let project = entries
            .iter()
            .find(|e| !e.dir && e.name.to_lowercase().ends_with(".pck"))
            .map(|e| Path::new(&e.name).file_stem().unwrap_or_default().to_string_lossy().into_owned())
            .or_else(|| {
                exe.and_then(|p| p.file_stem())
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        return Some(Engine::Godot { project });
    }

    // ---- Java / libGDX：自带运行时或 jar ----
    if has_dir("jre64") || has_dir("jre") || any_ext(".jar") {
        return Some(Engine::Java);
    }

    None
}

/// Unreal 的 exe 名形如 `<项目名>-Win64-Shipping` / `<项目名>-Win64-Shipping-Cmd`，去掉平台与构建配置后缀
fn unreal_project(stem: &str) -> String {
    let parts: Vec<&str> = stem.split('-').collect();
    parts.first().map(|s| s.to_string()).unwrap_or_else(|| stem.to_string())
}

// ---------------------------------------------------------------- 候选生成

fn push(
    out: &mut Vec<Candidate>,
    install: Option<&Path>,
    root: SaveRoot,
    rel: &str,
    origin: Origin,
    score: u8,
    why: String,
) {
    let spec = SaveSpec {
        root,
        rel: rel.to_string(),
    };
    let Some(path) = spec.resolve(install) else {
        return;
    };
    if out.iter().any(|c| c.path == path) {
        return;
    }
    let exists = path.exists();
    let (files, bytes, modified) = if exists {
        dir_stats(&path, 20_000)
    } else {
        (0, 0, None)
    };
    let score = if !exists && score >= 90 {
        score.saturating_sub(15)
    } else {
        score
    };
    out.push(Candidate {
        spec,
        path,
        exists,
        origin,
        score,
        why,
        files,
        bytes,
        modified,
    });
}

/// 探测一个游戏的存档位置。纯只读。
///
/// - `dir`：游戏文件夹（缺失则只做名称兜底）
/// - `exe`：主程序路径
/// - `name`：显示名作为兜底来源（清洗后去富集区匹配）
pub fn probe_game(dir: Option<&Path>, exe: Option<&Path>, name: &str) -> ProbeResult {
    let mut out = ProbeResult::default();
    let words = clean_words(name);
    // 引擎默认路径要拼「游戏名」时，用的是洗过的名字（原样往往是 Project.Zomboid.Build.24574865 这种）
    let display = clean_display(name);
    let engine = dir.and_then(|d| detect_engine(d, exe));

    match &engine {
        Some(Engine::Unity { company, product }) => {
            let rel = format!("{company}\\{product}");
            out.engine = Some("Unity".into());
            out.params = vec![
                ("厂商".into(), company.clone()),
                ("产品名".into(), product.clone()),
            ];
            push(
                &mut out.candidates,
                dir,
                SaveRoot::LocalLow,
                &rel,
                Origin::Declared,
                100,
                format!("读 <!>_Data\\app.info：厂商「{company}」+ 产品名「{product}」").replace("<!>", product),
            );
        }
        Some(Engine::Unreal { project }) => {
            out.engine = Some("Unreal Engine".into());
            out.params = vec![("项目名".into(), project.clone())];
            // 即便 SaveGames 还没生成，父级 Saved 目录只要存在，也说明这个游戏跑过、名字猜对了
            let rel = format!("{project}\\Saved\\SaveGames");
            let mut why = format!("虚幻引擎：exe 名推出项目名「{project}」");
            if let Some(base) = SaveRoot::Local.base(dir) {
                if base.join(project).exists() {
                    why.push_str("，且该游戏目录已存在");
                }
            }
            push(
                &mut out.candidates,
                dir,
                SaveRoot::Local,
                &rel,
                Origin::Declared,
                95,
                why,
            );
            // 项目名有时被发行商改过，再拿清洗后的游戏名试一次
            if let Some(w) = words.first() {
                let alt = capitalize_first(w);
                if !alt.eq_ignore_ascii_case(project) {
                    push(
                        &mut out.candidates,
                        dir,
                        SaveRoot::Local,
                        &format!("{alt}\\Saved\\SaveGames"),
                        Origin::Engine,
                        60,
                        format!("虚幻引擎备选项目名「{alt}」（取自游戏名，可能已被发行商改名）"),
                    );
                }
            }
        }
        Some(Engine::Godot { project }) => {
            out.engine = Some("Godot".into());
            out.params = vec![("项目名".into(), project.clone())];
            push(
                &mut out.candidates,
                dir,
                SaveRoot::Roaming,
                &format!("Godot\\app_userdata\\{project}"),
                Origin::Engine,
                92,
                format!("Godot：存档写在 app_userdata\\「{project}」"),
            );
        }
        Some(Engine::GameMaker) => {
            out.engine = Some("GameMaker".into());
            for (root, tag) in [(SaveRoot::Local, "%LOCALAPPDATA%"), (SaveRoot::Roaming, "%APPDATA%")] {
                push(
                    &mut out.candidates,
                    dir,
                    root,
                    &display,
                    Origin::Engine,
                    75,
                    format!("GameMaker 默认写在 {tag}\\<游戏名>（开发者可切换）"),
                );
            }
        }
        Some(Engine::RenPy) => {
            out.engine = Some("Ren'Py".into());
            push(
                &mut out.candidates,
                dir,
                SaveRoot::Roaming,
                &format!("RenPy\\{display}"),
                Origin::Engine,
                80,
                "Ren'Py 存档写在 %APPDATA%\\RenPy\\<名字>（实际目录常带随机数字后缀）".into(),
            );
            // Ren'Py 把 save_directory 设成 None 时，存档就在安装目录内的 game\saves
            if let Some(d) = dir {
                if d.join("game").join("saves").exists() {
                    push(
                        &mut out.candidates,
                        dir,
                        SaveRoot::Install,
                        "game\\saves",
                        Origin::Portable,
                        90,
                        "存档就在游戏目录内（绿色版，重装不丢）".into(),
                    );
                }
            }
        }
        Some(Engine::Love) => {
            out.engine = Some("LÖVE".into());
            push(
                &mut out.candidates,
                dir,
                SaveRoot::Roaming,
                &format!("LOVE\\{display}"),
                Origin::Engine,
                80,
                "LÖVE 存档写在 %APPDATA%\\LOVE\\<identity>".into(),
            );
        }
        Some(Engine::RpgMaker { mv }) => {
            out.engine = Some(if *mv {
                "RPG Maker MV/MZ".into()
            } else {
                "RPG Maker VX Ace/XP".into()
            });
            if *mv {
                push(
                    &mut out.candidates,
                    dir,
                    SaveRoot::Local,
                    &display,
                    Origin::Engine,
                    78,
                    "RPG Maker MV/MZ 的 .rpgsave 写在 %LOCALAPPDATA%\\<游戏名>".into(),
                );
            }
        }
        Some(Engine::NwJs) => {
            out.engine = Some("NW.js / Electron".into());
            push(
                &mut out.candidates,
                dir,
                SaveRoot::Roaming,
                &display,
                Origin::Engine,
                70,
                "NW.js/Electron 按 Chromium 约定写在 %APPDATA%\\<应用名>".into(),
            );
        }
        Some(Engine::Java) => {
            out.engine = Some("Java / libGDX".into());
            push(
                &mut out.candidates,
                dir,
                SaveRoot::Home,
                &display,
                Origin::Engine,
                65,
                "Java 程序常按 user.home 在用户根目录建自建目录".into(),
            );
        }
        None => {}
    }

    // 绿色版：安装目录里就有存档目录
    if let Some(d) = dir {
        for sub in ["saves", "Saves", "SaveData", "Save", "save"] {
            let p = d.join(sub);
            if p.is_dir() {
                push(
                    &mut out.candidates,
                    dir,
                    SaveRoot::Install,
                    sub,
                    Origin::Portable,
                    88,
                    "存档就在游戏目录内（绿色版，重装不丢）".into(),
                );
            }
        }
    }

    // 名称兜底：到几个存档富集区去找带游戏名关键词的目录
    guess_by_name(&mut out.candidates, dir, &words);

    // 排序：先按是否存在（本机真实命中的排前），再按置信度
    out.candidates.sort_by(|a, b| {
        b.exists
            .cmp(&a.exists)
            .then(b.score.cmp(&a.score))
            .then(a.path.to_string_lossy().len().cmp(&b.path.to_string_lossy().len()))
    });
    out.candidates.truncate(12);
    out
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// 名称兜底：枚举存档富集区的前两层，找出目录名含游戏关键词的
fn guess_by_name(out: &mut Vec<Candidate>, install: Option<&Path>, words: &[String]) {
    if words.is_empty() {
        return;
    }
    let roots = [
        SaveRoot::SavedGames,
        SaveRoot::Documents,
        SaveRoot::LocalLow,
        SaveRoot::Local,
        SaveRoot::Roaming,
        SaveRoot::Home,
        SaveRoot::ProgramData,
    ];
    for root in roots {
        let Some(base) = root.base(install) else {
            continue;
        };
        if !base.is_dir() {
            continue;
        }
        for e in scan_tree(&base, 3, 3000) {
            if !e.dir {
                continue;
            }
            let lname = e.name.to_lowercase();
            // 命中词数：按目录名关键词去重计数
            let hits = words.iter().filter(|w| lname.contains(w.as_str())).count();
            if hits == 0 {
                continue;
            }
            // 必须含至少一个有辨识度的词（≥5 字符），否则单靠通用词会满天飞
            if !words.iter().any(|w| w.len() >= 5 && lname.contains(w.as_str())) {
                continue;
            }
            let rel = e
                .path
                .strip_prefix(&base)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|_| e.name.clone());
            // 已命中的路径里若有一条是它的祖先，就跳过（父子同命中时保留更短的那条）
            if out.iter().any(|c| {
                c.spec.root == root
                    && (rel.starts_with(&format!("{}\\", c.spec.rel)) || c.spec.rel == rel)
            }) {
                continue;
            }
            let score = (45u8).saturating_add((hits as u8).saturating_mul(12)).min(85);
            let why = format!(
                "目录名含关键词 {}",
                words
                    .iter()
                    .filter(|w| lname.contains(w.as_str()))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" / ")
            );
            let nested = rel.contains('\\');
            let score = if nested { score.saturating_sub(5) } else { score };
            push(out, install, root.clone(), &rel, Origin::Guess, score, why);
        }
    }
}

/// 人类可读的体积
pub fn fmt_bytes(b: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if b >= GB {
        format!("{:.2} GB", b as f64 / GB as f64)
    } else if b >= MB {
        format!("{:.1} MB", b as f64 / MB as f64)
    } else if b >= KB {
        format!("{:.0} KB", b as f64 / KB as f64)
    } else {
        format!("{b} B")
    }
}

/// 最近修改时间 → 本地「年-月-日 时:分」
pub fn fmt_modified(t: Option<SystemTime>) -> String {
    use chrono::TimeZone;
    let Ok(d) = t.unwrap_or(SystemTime::UNIX_EPOCH).duration_since(SystemTime::UNIX_EPOCH) else {
        return "—".into();
    };
    match chrono::Local.timestamp_opt(d.as_secs() as i64, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => "—".into(),
    }
}
