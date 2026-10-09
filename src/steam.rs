use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct StoreSearchItem {
    id: u32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct StoreSearch {
    #[allow(dead_code)]
    total: u32,
    items: Vec<StoreSearchItem>,
}

/// 匹配候选（来源可能是 Steam 或 SteamGridDB）
#[derive(Debug, Clone)]
pub struct Candidate {
    pub source: &'static str, // "steam" | "sgdb"
    pub id: u32,
    pub name: String,
}

/// Steam 商店搜索，返回前 limit 个候选。
/// 注意：storesearch 是 Steam 商店的非公开接口，仅用于个人本地管理，
/// 请控制请求频率、做好缓存，勿高频刷接口。
pub async fn search_steam(
    name: &str,
    client: &reqwest::Client,
    limit: usize,
) -> anyhow::Result<Vec<Candidate>> {
    let resp = client
        .get("https://store.steampowered.com/api/storesearch/")
        .query(&[("term", name), ("cc", "cn"), ("l", "schinese")])
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(vec![]);
    }
    let data: StoreSearch = resp.json().await?;
    Ok(data
        .items
        .into_iter()
        .take(limit)
        .map(|i| Candidate {
            source: "steam",
            id: i.id,
            name: i.name,
        })
        .collect())
}

// ---------- SteamGridDB（可选，需 API key） ----------

#[derive(Debug, Deserialize)]
struct SgdbAutoItem {
    id: u32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct SgdbAuto {
    data: Vec<SgdbAutoItem>,
}

/// SteamGridDB 自动补全搜索，返回前 limit 个游戏候选（需免费 API key）
pub async fn search_sgdb(
    name: &str,
    key: &str,
    client: &reqwest::Client,
    limit: usize,
) -> anyhow::Result<Vec<Candidate>> {
    let resp = client
        .get(format!(
            "https://www.steamgriddb.com/api/v2/search/autocomplete/{}",
            name
        ))
        .query(&[("limit", limit)])
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(vec![]);
    }
    let data: SgdbAuto = resp.json().await?;
    Ok(data
        .data
        .into_iter()
        .take(limit)
        .map(|i| Candidate {
            source: "sgdb",
            id: i.id,
            name: i.name,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct SgdbGrid {
    url: String,
}

#[derive(Debug, Deserialize)]
struct SgdbGrids {
    data: Vec<SgdbGrid>,
}

/// 根据候选解析封面 URL（下载前调用）。
/// - steam：直接拼 CDN 地址（竖版 600x900 + 横版 header）
/// - sgdb：拉取该游戏 static 600x900 网格图作为封面（无 header 概念，用封面替代）
pub async fn resolve_cover(
    source: &str,
    id: u32,
    key: &str,
    client: &reqwest::Client,
) -> anyhow::Result<(String, String)> {
    if source == "sgdb" {
        let resp = client
            .get(format!("https://www.steamgriddb.com/api/v2/grids/game/{}", id))
            .query(&[("types", "static"), ("dimensions", "600x900"), ("limit", "1")])
            .header("Authorization", format!("Bearer {}", key))
            .send()
            .await?;
        let r: SgdbGrids = resp.json().await?;
        if let Some(g) = r.data.into_iter().next() {
            return Ok((g.url.clone(), g.url));
        }
        anyhow::bail!("SteamGridDB 未找到该游戏的封面")
    } else {
        let cover = format!(
            "https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/library_600x900.jpg"
        );
        let header = format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/header.jpg");
        Ok((cover, header))
    }
}

/// 下载封面字节到本地
pub async fn download_cover(client: &reqwest::Client, url: &str, dest: &Path) -> anyhow::Result<()> {
    let bytes = client.get(url).send().await?.bytes().await?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, &bytes)?;
    Ok(())
}

/// 横版「英雄图」地址 —— 详情页背景图就取它（不是竖版封面）。
/// 依次探测两张宽幅美术图，返回第一个存在的地址：
///   1. library_hero.jpg            —— 1920×620 的英雄图，构图最好
///   2. page_bg_generated_v6b.jpg   —— 商店页背景图，兜底
/// 注：这两个地址是 Steam CDN 的公开静态资源，不需要 key，也跟登录无关。
pub async fn resolve_hero(appid: u32, client: &reqwest::Client) -> Option<String> {
    for url in [
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{appid}/library_hero.jpg"),
        format!(
            "https://cdn.cloudflare.steamstatic.com/steam/apps/{appid}/page_bg_generated_v6b.jpg"
        ),
    ] {
        if let Ok(r) = client.head(&url).send().await {
            if r.status().is_success() {
                // 限制单张图下载上限，避免个别图返回 20MB+ 坏编码图时卡死
                let len = r.content_length().unwrap_or(0);
                if len == 0 || len < 10_000_000 {
                    return Some(url);
                }
            }
        }
    }
    None
}
