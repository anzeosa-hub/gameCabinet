use crate::library::GameEntry;
use std::path::{Path, PathBuf};

/// 扫描游戏根目录：
/// 1) 一级子文件夹 → 视为一个游戏（取其下主 exe）
/// 2) 根目录下散落的 exe → 视为独立游戏
pub fn scan(root: &Path) -> Vec<GameEntry> {
    let now = chrono::Local::now().to_rfc3339();
    let mut games = Vec::new();

    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = file_name_of(&path);
                if name == "covers" {
                    continue; // 封面输出目录，不当作游戏
                }
                let exe = find_main_exe(&path);
                games.push(GameEntry {
                    name,
                    executable: exe.map(|p| to_string(&p)),
                    folder: Some(to_string(&path)),
                    steam_appid: None,
                    matched_name: None,
                    alias: None,
                    cover_local: None,
                    cover_url: None,
                    header_url: None,
                    hero_local: None,
                    status: "未开始".into(),
                    progress: 0,
                    playtime_hours: 0.0,
                    rating: 0,
                    note: String::new(),
                    added_at: now.clone(),
                });
            } else if is_exe(&path) {
                games.push(GameEntry {
                    name: exe_stem_of(&path),
                    executable: Some(to_string(&path)),
                    folder: None,
                    steam_appid: None,
                    matched_name: None,
                    alias: None,
                    cover_local: None,
                    cover_url: None,
                    header_url: None,
                    hero_local: None,
                    status: "未开始".into(),
                    progress: 0,
                    playtime_hours: 0.0,
                    rating: 0,
                    note: String::new(),
                    added_at: now.clone(),
                });
            }
        }
    }
    games
}

fn to_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn file_name_of(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn exe_stem_of(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn is_exe(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false)
}

/// 在文件夹内找“主程序” exe：跳过卸载 / 配置 / 启动器类
fn find_main_exe(dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() && is_exe(&p) {
                candidates.push(p);
            }
        }
    }
    let skip = [
        "unins", "uninstall", "setup", "config", "setting", "crash", "report", "launcher",
        "patch",
    ];
    candidates.retain(|p| {
        let stem = exe_stem_of(p).to_lowercase();
        !skip.iter().any(|k| stem.contains(k))
    });
    candidates.into_iter().next()
}
