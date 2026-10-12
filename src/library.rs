use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 每个游戏专属的图片子目录名。
/// 放在游戏文件夹内部，与游戏自身的文件分开存放，跟着游戏一起移动 / 删除。
pub const CABINET_DIR: &str = ".cabinet";

/// 该游戏的图片输出目录（相对库根的路径）。
///
/// - 游戏是库根下的**一级子文件夹** → `<游戏文件夹>/.cabinet`（自包含，删游戏不留残留）
/// - 根目录下散落的 exe（`folder` 为空）→ 回落到 `covers`（没有独立文件夹可放）
pub fn art_dir(root: &Path, g: &GameEntry) -> PathBuf {
    if let Some(f) = g.folder.as_deref() {
        // 注意：`folder` 由 scanner 按 `root.join(子目录名)` 生成，**本身就含 root 前缀**
        //（绝对 root → 绝对路径；CLI 传相对 root → 相对 cwd 的路径）。
        // 所以这里绝不能再 join(root)，否则会叠加成 `<root>/<root>/<游戏>`。
        let fp = Path::new(f);
        // 规范化后再比父目录：兼容 `D:\Games\` 与 `D:\Games`、尾斜杠等写法差异
        let root_c = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let fp_c = std::fs::canonicalize(fp).unwrap_or_else(|_| fp.to_path_buf());
        // 必须确实是库根的直接子目录，才敢往里写（嵌套目录 / 库根本身都排除）
        if fp_c.is_dir() && fp_c.parent() == Some(root_c.as_path()) {
            if let Some(name) = fp_c.file_name() {
                return Path::new(name).join(CABINET_DIR);
            }
        }
    }
    PathBuf::from("covers")
}

/// 一个游戏的数据库条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEntry {
    /// 游戏显示名（取自文件夹名或 exe 名）
    pub name: String,
    /// 主程序 exe 路径（破解游戏常是文件夹里的某个 exe）
    pub executable: Option<String>,
    /// 游戏所在文件夹路径
    pub folder: Option<String>,
    /// 从 Steam 匹配到的 appid
    pub steam_appid: Option<u32>,
    /// 匹配到的游戏名（用于人工核对，可能来自 Steam 或 SteamGridDB）
    pub matched_name: Option<String>,
    /// 用户自定义别名（显示优先级最高：别名 > 匹配名 > 原始名）
    #[serde(default)]
    pub alias: Option<String>,
    /// 下载到本地的封面相对路径（相对扫描根目录）
    pub cover_local: Option<String>,
    /// 封面地址（Steam CDN 或 SteamGridDB）
    pub cover_url: Option<String>,
    /// 横版头图地址
    pub header_url: Option<String>,
    /// 本地横版「英雄图」相对路径（Steam library_hero.jpg）。
    /// 详情页背景取宽幅美术图，铺满才好看；竖版封面拉大只会糊成一团。
    #[serde(default)]
    pub hero_local: Option<String>,
    /// 通关进度 0-100
    #[serde(default)]
    pub progress: u8,
    /// 游玩时长（小时）
    #[serde(default)]
    pub playtime_hours: f32,
    /// 评分 1-10（0=未评）
    #[serde(default)]
    pub rating: u8,
    /// 备注
    #[serde(default)]
    pub note: String,
    /// 入库时间（RFC3339）
    pub added_at: String,
    /// 存档位置（人工确认过的）。
    ///
    /// 存的是**机器无关**的「根目录记号 + 相对路径」，不是绝对路径：
    /// 绝对路径会把当前用户名焊死在里面，换机 / 换用户名就失效。
    /// 备份时按本机展开去读、还原时按新机器重新展开去写。
    #[serde(default)]
    pub save_specs: Vec<crate::saves::SaveSpec>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Library {
    pub games: Vec<GameEntry>,
}

impl Library {
    pub fn load(path: &std::path::Path) -> Self {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(lib) = serde_json::from_str::<Library>(&text) {
                return lib;
            }
        }
        Library::default()
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }
}
