use serde::{Deserialize, Serialize};

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
    /// 游戏状态：未开始 / 在玩 / 已通关 / 弃坑 / 想玩
    #[serde(default)]
    pub status: String,
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
