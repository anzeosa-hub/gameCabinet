#![windows_subsystem = "windows"]
mod gui;
mod library;
mod saves;
mod scanner;
mod steam;

use clap::{Parser, Subcommand};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "game-cabinet", about = "本地单机游戏库管理 + Steam 封面抓取")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 扫描游戏目录并抓取封面
    Scan {
        /// 游戏根目录
        folder: PathBuf,
        /// 封面输出目录（默认 <folder>/covers）
        #[arg(short, long)]
        covers: Option<PathBuf>,
        /// 并发抓取数（非交互模式，默认 4）
        #[arg(short = 'j', long, default_value_t = 4)]
        jobs: usize,
        /// 交互式匹配：逐个列出候选让你选（或手动指定 appid）
        #[arg(short, long)]
        interactive: bool,
    },
    /// 对库中未匹配封面的游戏做交互式重新匹配
    Match {
        /// 游戏库文件
        #[arg(short, long, default_value = "library.json")]
        library: PathBuf,
        /// 扫描根目录（用于解析封面相对路径 & 输出位置）
        #[arg(short, long)]
        root: Option<PathBuf>,
    },
    /// 编辑某个游戏的进度 / 状态 / 评分 / 备注，或补封面
    Edit {
        /// 游戏名关键词（取第一个包含它的游戏）
        name: String,
        /// 游戏库文件
        #[arg(short, long, default_value = "library.json")]
        library: PathBuf,
        /// 扫描根目录（封面输出位置）
        #[arg(short, long)]
        root: Option<PathBuf>,
        /// 进度 0-100
        #[arg(long)]
        progress: Option<u8>,
        /// 游玩时长（小时）
        #[arg(long)]
        playtime: Option<f32>,
        /// 评分 1-10
        #[arg(long)]
        rating: Option<u8>,
        /// 备注
        #[arg(long)]
        note: Option<String>,
        /// 直接指定 Steam appid（覆盖匹配，并下载其封面）
        #[arg(long)]
        appid: Option<u32>,
        /// 补封面：本地图片路径，或图片 URL（http/https）
        #[arg(long)]
        cover: Option<String>,
    },
    /// 探测游戏的存档位置（只读：不移动、不修改任何文件）
    Probe {
        /// 游戏库文件
        #[arg(short, long, default_value = "library.json")]
        library: PathBuf,
        /// 只看名字含该关键词的游戏
        filter: Option<String>,
        /// 列出本机还不存在的路径（游戏没玩过、目录还没生成）
        #[arg(long)]
        all: bool,
        /// 交互式确认：把选中的路径写进 library.json
        #[arg(short, long)]
        pick: bool,
    },
    /// 根据 library.json 生成 HTML 海报墙（含进度 / 状态 / 评分）
    Wall {
        #[arg(short, long, default_value = "library.json")]
        library: PathBuf,
        #[arg(short, long, default_value = "wall.html")]
        out: PathBuf,
    },
    /// 启动桌面图形界面（双击 exe 默认进入）
    Gui,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let key = std::env::var("STEAMGRIDDB_API_KEY").unwrap_or_default();
    match cli.command {
        Some(Commands::Scan {
            folder,
            covers,
            jobs,
            interactive,
        }) => run_scan(folder, covers, jobs, interactive, &key).await?,
        Some(Commands::Match { library, root }) => run_match(library, root, &key).await?,
        Some(Commands::Edit {
            name,
            library,
            root,
            progress,
            playtime,
            rating,
            note,
            appid,
            cover,
        }) => {
            run_edit(name, library, root, progress, playtime, rating, note, appid, cover, &key).await?
        }
        Some(Commands::Probe {
            library,
            filter,
            all,
            pick,
        }) => run_probe(library.as_path(), filter.as_deref(), all, pick)?,
        Some(Commands::Wall { library, out }) => run_wall(library.as_path(), out.as_path())?,
        Some(Commands::Gui) | None => gui::run()?,
    }
    Ok(())
}

fn rel(p: &Path, root: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().into_owned()
}

fn cover_base(c: &steam::Candidate) -> String {
    match c.source {
        "sgdb" => format!("sgdb_{}", c.id),
        _ => c.id.to_string(),
    }
}

struct ScanOutcome {
    idx: usize,
    cand: Option<steam::Candidate>,
    cover_local: Option<PathBuf>,
    cover: String,
    header: String,
}

/// 扫描目录 + 自动匹配第一个候选 + 下载封面，返回游戏库（不打印、供 GUI 复用）
pub async fn scan_and_fetch(
    folder: &Path,
    covers: Option<PathBuf>,
    jobs: usize,
    key: &str,
) -> anyhow::Result<library::Library> {
    let mut games = scanner::scan(folder);
    // 散落 exe 的兜底目录；只有真用得上（没有独立文件夹的游戏）才在任务里按需创建
    let covers_dir = covers.unwrap_or_else(|| folder.join("covers"));

    let client = reqwest::Client::builder()
        .user_agent("game-cabinet/0.1 (local game manager)")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(jobs.max(1)));
    let mut handles = Vec::new();
    for (idx, game) in games.iter().enumerate() {
        let sem = sem.clone();
        let client = client.clone();
        let name = game.name.clone();
        let key = key.to_string();
        // 每个游戏的图放哪：游戏文件夹内的 .cabinet（自包含，删游戏不留残留）；散落 exe 才回落 covers
        let art_rel = library::art_dir(folder, game);
        let in_cabinet = art_rel != Path::new("covers");
        let art_abs = if in_cabinet { folder.join(&art_rel) } else { covers_dir.clone() };
        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let cans = match steam::search_steam(&name, &client, 6).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[!] «{}» 搜索失败: {}", name, e);
                    return ScanOutcome {
                        idx,
                        cand: None,
                        cover_local: None,
                        cover: String::new(),
                        header: String::new(),
                    };
                }
            };
            if let Some(c) = cans.into_iter().next() {
                let (cover, header) = match steam::resolve_cover(c.source, c.id, &key, &client).await
                {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[!] «{}» 解析封面失败: {}", name, e);
                        return ScanOutcome {
                            idx,
                            cand: None,
                            cover_local: None,
                            cover: String::new(),
                            header: String::new(),
                        };
                    }
                };
                let _ = std::fs::create_dir_all(&art_abs);
                let (cover_path, header_path) = if in_cabinet {
                    (art_abs.join("cover.jpg"), art_abs.join("header.jpg"))
                } else {
                    let base = cover_base(&c);
                    (
                        art_abs.join(format!("{base}_cover.jpg")),
                        art_abs.join(format!("{base}_header.jpg")),
                    )
                };
                let mut ok = steam::download_cover(&client, &cover, &cover_path).await.is_ok();
                if !ok {
                    ok = steam::download_cover(&client, &header, &cover_path).await.is_ok();
                }
                let _ = steam::download_cover(&client, &header, &header_path).await;
                ScanOutcome {
                    idx,
                    cand: Some(c),
                    cover_local: if ok { Some(cover_path) } else { None },
                    cover,
                    header,
                }
            } else {
                ScanOutcome {
                    idx,
                    cand: None,
                    cover_local: None,
                    cover: String::new(),
                    header: String::new(),
                }
            }
        });
        handles.push(handle);
    }
    for h in handles {
        if let Ok(out) = h.await {
            if let Some(c) = out.cand {
                if c.source == "steam" {
                    games[out.idx].steam_appid = Some(c.id);
                }
                games[out.idx].matched_name = Some(c.name.clone());
                games[out.idx].cover_url = Some(out.cover);
                games[out.idx].header_url = Some(out.header);
                if let Some(p) = out.cover_local {
                    games[out.idx].cover_local = Some(rel(&p, folder));
                }
            }
        }
    }
    Ok(library::Library { games })
}

async fn run_scan(
    folder: PathBuf,
    covers: Option<PathBuf>,
    jobs: usize,
    interactive: bool,
    key: &str,
) -> anyhow::Result<()> {
    if interactive {
        println!("[*] 扫描目录: {}", folder.display());
        let mut games = scanner::scan(&folder);
        println!("[*] 发现 {} 个游戏", games.len());
        // 散落 exe 的兜底目录；只有真用得上才在循环里按需创建
        let covers_dir = covers.unwrap_or_else(|| folder.join("covers"));
        let client = reqwest::Client::builder()
            .user_agent("game-cabinet/0.1 (local game manager)")
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        for g in games.iter_mut() {
            println!("\n=== «{}» ===", g.name);
            // 每个游戏的图放哪：游戏文件夹内的 .cabinet；散落 exe 才回落 covers
            let art_rel = library::art_dir(&folder, g);
            let in_cabinet = art_rel != Path::new("covers");
            let art_abs = if in_cabinet { folder.join(&art_rel) } else { covers_dir.clone() };
            match interactive_pick(&g.name, &client, key).await? {
                Some(c) => {
                    let (cover, header) = steam::resolve_cover(c.source, c.id, key, &client).await?;
                    let _ = std::fs::create_dir_all(&art_abs);
                    let (cover_path, header_path) = if in_cabinet {
                        (art_abs.join("cover.jpg"), art_abs.join("header.jpg"))
                    } else {
                        let base = cover_base(&c);
                        (
                            art_abs.join(format!("{base}_cover.jpg")),
                            art_abs.join(format!("{base}_header.jpg")),
                        )
                    };
                    let mut ok = steam::download_cover(&client, &cover, &cover_path).await.is_ok();
                    if !ok {
                        ok = steam::download_cover(&client, &header, &cover_path).await.is_ok();
                    }
                    let _ = steam::download_cover(&client, &header, &header_path).await;
                    if c.source == "steam" {
                        g.steam_appid = Some(c.id);
                    }
                    g.matched_name = Some(c.name.clone());
                    g.cover_url = Some(cover);
                    g.header_url = Some(header);
                    if ok {
                        g.cover_local = Some(rel(&cover_path, &folder));
                        println!("[+] 匹配: {} ({} {})", c.name, c.source, c.id);
                    } else {
                        println!("[~] 匹配 {} 但封面下载失败", c.name);
                    }
                }
                None => println!("[ ] 跳过匹配"),
            }
        }
        let lib_path = folder.join("library.json");
        library::Library { games }.save(&lib_path)?;
        println!("[*] 已保存游戏库: {}", lib_path.display());
    } else {
        let lib = scan_and_fetch(&folder, covers, jobs, key).await?;
        let lib_path = folder.join("library.json");
        lib.save(&lib_path)?;
        println!("[*] 已保存游戏库: {}", lib_path.display());
        println!("[*] 未匹配的可用 `game-cabinet match` 交互补匹配，或 `edit --cover` 手动补封面");
    }
    Ok(())
}

/// 交互式挑选匹配候选
async fn interactive_pick(
    name: &str,
    client: &reqwest::Client,
    key: &str,
) -> anyhow::Result<Option<steam::Candidate>> {
    let mut cans = steam::search_steam(name, client, 6).await?;
    if !key.is_empty() {
        if let Ok(sg) = steam::search_sgdb(name, key, client, 6).await {
            cans.extend(sg);
        }
    }
    println!("候选（序号选择 / a<appid> 手动指定 Steam / s<id> 手动指定 SGDB / 0 或回车跳过）：");
    for (i, c) in cans.iter().enumerate() {
        println!("  {}. [{}] {}", i + 1, c.source, c.name);
    }
    print!("> ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let input = line.trim();
    if input.is_empty() || input == "0" {
        return Ok(None);
    }
    if let Ok(n) = input.parse::<usize>() {
        if n >= 1 && n <= cans.len() {
            return Ok(Some(cans[n - 1].clone()));
        }
        println!("[!] 序号越界，跳过");
        return Ok(None);
    }
    if let Some(rest) = input.strip_prefix('a') {
        if let Ok(id) = rest.parse::<u32>() {
            return Ok(Some(steam::Candidate {
                source: "steam",
                id,
                name: format!("手动指定 appid {id}"),
            }));
        }
    }
    if let Some(rest) = input.strip_prefix('s') {
        if let Ok(id) = rest.parse::<u32>() {
            return Ok(Some(steam::Candidate {
                source: "sgdb",
                id,
                name: format!("手动指定 SGDB {id}"),
            }));
        }
    }
    println!("[!] 无法识别，跳过");
    Ok(None)
}

async fn run_match(library_path: PathBuf, root: Option<PathBuf>, key: &str) -> anyhow::Result<()> {
    let lib_path = if library_path.exists() {
        library_path
    } else {
        root.unwrap_or_else(|| PathBuf::from(".")).join("library.json")
    };
    let mut lib = library::Library::load(&lib_path);
    let folder = lib_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    // 散落 exe 的兜底目录；按需创建
    let covers_dir = folder.join("covers");
    let client = reqwest::Client::builder()
        .user_agent("game-cabinet/0.1 (local game manager)")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let mut changed = 0;
    for g in lib.games.iter_mut() {
        if g.cover_local.is_some() {
            continue; // 已有封面
        }
        println!("\n=== «{}»（当前未匹配封面）===", g.name);
        if let Some(c) = interactive_pick(&g.name, &client, key).await? {
            let (cover, header) = steam::resolve_cover(c.source, c.id, key, &client).await?;
            let art_rel = library::art_dir(&folder, g);
            let in_cabinet = art_rel != Path::new("covers");
            let art_abs = if in_cabinet { folder.join(&art_rel) } else { covers_dir.clone() };
            let _ = std::fs::create_dir_all(&art_abs);
            let cover_path = if in_cabinet {
                art_abs.join("cover.jpg")
            } else {
                let base = cover_base(&c);
                art_abs.join(format!("{base}_cover.jpg"))
            };
            let mut ok = steam::download_cover(&client, &cover, &cover_path).await.is_ok();
            if !ok {
                ok = steam::download_cover(&client, &header, &cover_path).await.is_ok();
            }
            if c.source == "steam" {
                g.steam_appid = Some(c.id);
            }
            g.matched_name = Some(c.name.clone());
            g.cover_url = Some(cover);
            g.header_url = Some(header);
            if ok {
                g.cover_local = Some(rel(&cover_path, &folder));
                println!("[+] 匹配: {} ({} {})", c.name, c.source, c.id);
                changed += 1;
            } else {
                println!("[~] 匹配但封面下载失败");
            }
        }
    }
    lib.save(&lib_path)?;
    println!("[*] 处理完成，更新 {} 个游戏的封面", changed);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_edit(
    name: String,
    library_path: PathBuf,
    root: Option<PathBuf>,
    progress: Option<u8>,
    playtime: Option<f32>,
    rating: Option<u8>,
    note: Option<String>,
    appid: Option<u32>,
    cover: Option<String>,
    key: &str,
) -> anyhow::Result<()> {
    let lib_path = if library_path.exists() {
        library_path
    } else {
        root.unwrap_or_else(|| PathBuf::from(".")).join("library.json")
    };
    let mut lib = library::Library::load(&lib_path);
    let folder = lib_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    // 散落 exe 的兜底目录；按需创建
    let covers_dir = folder.join("covers");
    let client = reqwest::Client::builder()
        .user_agent("game-cabinet/0.1 (local game manager)")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let g = match lib.games.iter_mut().find(|g| g.name.contains(&name)) {
        Some(g) => g,
        None => anyhow::bail!("未找到包含「{}」的游戏", name),
    };
    // 这个游戏的图放哪（下面两个补封面的分支都要用）
    let art_rel = library::art_dir(&folder, g);
    let in_cabinet = art_rel != Path::new("covers");
    let art_abs = if in_cabinet { folder.join(&art_rel) } else { covers_dir.clone() };

    if let Some(v) = progress {
        g.progress = v.min(100);
    }
    if let Some(v) = playtime {
        g.playtime_hours = v;
    }
    if let Some(v) = rating {
        g.rating = v.min(10);
    }
    if let Some(v) = note {
        g.note = v;
    }

    if let Some(id) = appid {
        g.steam_appid = Some(id);
        let (cover_url, header_url) = steam::resolve_cover("steam", id, key, &client).await?;
        let _ = std::fs::create_dir_all(&art_abs);
        let cover_path = if in_cabinet {
            art_abs.join("cover.jpg")
        } else {
            art_abs.join(format!("{id}_cover.jpg"))
        };
        let _ = steam::download_cover(&client, &cover_url, &cover_path).await;
        g.cover_url = Some(cover_url);
        g.header_url = Some(header_url);
        g.cover_local = Some(rel(&cover_path, &folder));
        println!("[+] 已按 appid {} 刷新封面", id);
    }

    if let Some(c) = cover {
        let _ = std::fs::create_dir_all(&art_abs);
        let dest = if in_cabinet {
            art_abs.join("cover.jpg")
        } else {
            let base = match g.steam_appid {
                Some(id) => id.to_string(),
                None => sanitize(&g.name),
            };
            art_abs.join(format!("{base}_cover.jpg"))
        };
        let url_opt = if c.starts_with("http://") || c.starts_with("https://") {
            steam::download_cover(&client, &c, &dest).await?;
            Some(c.clone())
        } else {
            let src = Path::new(&c);
            // 源与目标为同一文件时跳过复制（避免 Windows 文件占用错误）
            if src.canonicalize().ok() != dest.canonicalize().ok() {
                std::fs::copy(src, &dest)?;
            }
            None
        };
        g.cover_local = Some(rel(&dest, &folder));
        if let Some(u) = url_opt {
            g.cover_url = Some(u);
        }
        println!("[+] 已补封面: {}", dest.display());
    }

    let updated_name = g.name.clone();
    lib.save(&lib_path)?;
    println!("[*] 已更新 «{}»", updated_name);
    Ok(())
}

/// 探测每个游戏的存档位置。
///
/// 默认是**纯只读**的：只把本机真实存在的候选路径列出来给人看。
/// 加 `--pick` 才进入交互模式，把人选中的路径写进 `library.json`。
fn run_probe(
    library_path: &Path,
    filter: Option<&str>,
    list_missing: bool,
    pick: bool,
) -> anyhow::Result<()> {
    if !library_path.exists() {
        anyhow::bail!(
            "找不到游戏库文件：{}\n用 --library 指定 library.json 的路径",
            library_path.display()
        );
    }
    let mut lib = library::Library::load(library_path);
    if lib.games.is_empty() {
        anyhow::bail!("游戏库是空的（或 JSON 解析失败），先跑一次 scan");
    }

    let mut total = 0usize;
    let mut none = 0usize;
    for g in lib.games.iter_mut() {
        if let Some(f) = filter {
            if !g.name.to_lowercase().contains(&f.to_lowercase()) {
                continue;
            }
        }
        let folder = g.folder.clone().map(PathBuf::from);
        let exe_p = g.executable.clone().map(PathBuf::from);
        let res = saves::probe_game(folder.as_deref(), exe_p.as_deref(), &g.name);
        let shown: Vec<(usize, &saves::Candidate)> = res
            .candidates
            .iter()
            .enumerate()
            // 默认只显示本机真实存在的；但「引擎声明」类的路径即使还不存在也要显示出来 ——
            // 它告诉你游戏将来会把存档写到哪（只是还没玩过、目录没生成），比空手而归有用
            .filter(|(_, c)| {
                list_missing
                    || c.exists
                    || matches!(c.origin, saves::Origin::Declared)
            })
            .collect();

        println!("\n=== «{}» ===", g.name);
        match &res.engine {
            Some(e) => {
                println!("  引擎: {e}");
                for (k, v) in &res.params {
                    println!("       {k} = {v}");
                }
            }
            None => println!("  引擎: 未识别"),
        }
        if shown.is_empty() {
            none += 1;
            println!("  本机没有任何存在的候选路径，也没有能读出路径的引擎声明。");
            println!("  要么还没玩过、引擎也没留下线索；要么得你自己翻一次文件夹再手填。");
            continue;
        }
        if shown.iter().all(|(_, c)| !c.exists) {
            none += 1;
        }
        total += shown.len();
        for (n, c) in shown.iter() {
            if c.exists {
                println!(
                    "  [{}] {} · {} · {} 文件 · 最近 {}\n      {}\n      {}",
                    n + 1,
                    c.origin.label(),
                    saves::fmt_bytes(c.bytes),
                    c.files,
                    saves::fmt_modified(c.modified),
                    c.path.display(),
                    c.why,
                );
            } else {
                println!(
                    "  [{}] {} · （本机不存在）\n      {}\n      {}",
                    n + 1,
                    c.origin.label(),
                    c.path.display(),
                    c.why,
                );
            }
        }

        if pick {
            print!("  选择（序号，逗号分隔；也可直接粘贴一个路径；回车跳过）> ");
            io::stdout().flush()?;
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line)?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // 输入里含盘符或反斜杠就当成手填路径，否则按「一组序号」解析
            let looks_path = line.contains(':') || line.contains('\\') || line.contains('/');
            let mut chosen: Vec<saves::SaveSpec> = Vec::new();
            if looks_path {
                let p = PathBuf::from(line);
                if p.is_dir() {
                    chosen.push(saves::spec_from_path(&p, folder.as_deref()));
                } else {
                    println!("  [!] 该目录不存在，未记录");
                }
            } else {
                for tok in line.split(|c: char| c == ',' || c == ' ' || c == '、') {
                    let tok = tok.trim();
                    if tok.is_empty() {
                        continue;
                    }
                    match tok.parse::<usize>() {
                        Ok(n) if n >= 1 && n <= res.candidates.len() => {
                            let spec = res.candidates[n - 1].spec.clone();
                            if !chosen.iter().any(|s| {
                                s.root == spec.root && s.rel == spec.rel
                            }) {
                                chosen.push(spec);
                            }
                        }
                        _ => println!("  [!] 忽略无法识别的输入：{tok}"),
                    }
                }
            }
            if !chosen.is_empty() {
                g.save_specs = chosen;
                println!("  [+] 已记录 {} 条路径", g.save_specs.len());
            }
        }
    }

    if pick {
        lib.save(library_path)?;
        println!("\n[*] 已写回 {}", library_path.display());
    } else {
        println!("\n[*] 共列出 {total} 条候选；{none} 个游戏没找到。加 --pick 可以把确认的路径写进库。");
    }
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_")
}

fn run_wall(library: &Path, out: &Path) -> anyhow::Result<()> {
    let lib = library::Library::load(library);
    let mut cards = String::new();
    for g in &lib.games {
        let cover = g
            .cover_local
            .clone()
            .or_else(|| g.cover_url.clone())
            .unwrap_or_default();
        let mut info = String::new();
        if g.progress > 0 {
            info.push_str(&progress_bar(g.progress));
        }
        if g.rating > 0 {
            info.push_str(&format!(" {} {}", star(g.rating), g.rating));
        }
        if g.playtime_hours > 0.0 {
            info.push_str(&format!(" ⏱{:.1}h", g.playtime_hours));
        }
        cards.push_str(&format!(
            r#"<div class="card"><img src="{cover}" alt="{name}" loading="lazy" onerror="this.style.visibility='hidden'"><div class="name">{name}</div><div class="meta">{info}</div></div>"#,
            cover = cover,
            name = escape(&g.name),
            info = info,
        ));
    }
    let html = format!(
        r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>游戏柜 · 海报墙</title>
<style>
  body {{ font-family: system-ui, "Microsoft YaHei", sans-serif; background:#14161a; color:#e8e8e8; margin:0; padding:24px; }}
  h1 {{ font-weight:600; }}
  .grid {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(180px,1fr)); gap:16px; }}
  .card {{ background:#1e2228; border-radius:10px; overflow:hidden; box-shadow:0 2px 8px rgba(0,0,0,.4); }}
  .card img {{ width:100%; aspect-ratio:2/3; object-fit:cover; display:block; background:#2a2f37; }}
  .name {{ padding:8px 10px 2px; font-size:14px; font-weight:600; }}
  .meta {{ padding:0 10px 10px; font-size:12px; color:#9aa3ad; line-height:1.6; }}
  .tag {{ display:inline-block; background:#2d6cdf; color:#fff; border-radius:4px; padding:1px 6px; font-size:11px; }}
</style></head><body>
<h1>我的游戏柜 · 共 {} 款</h1>
<div class="grid">{}</div>
</body></html>"#,
        lib.games.len(),
        cards
    );
    std::fs::write(out, html)?;
    println!("[*] 海报墙已生成: {}", out.display());
    Ok(())
}

fn progress_bar(p: u8) -> String {
    let filled = (p / 10) as usize;
    format!("{}{} {}%", "█".repeat(filled), "░".repeat(10 - filled), p)
}

fn star(r: u8) -> String {
    format!("{}{}", "★".repeat(r as usize), "☆".repeat(10 - r as usize))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
