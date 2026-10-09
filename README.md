# gameCabinet

本地单机游戏库管理器。扫描游戏目录、自动匹配 Steam / SteamGridDB 封面，用玻璃拟态（apple-style glassmorphism）界面查看并管理你的游戏收藏。

## 特性

- 扫描本地游戏目录，自动识别游戏
- 自动匹配 Steam / SteamGridDB 封面与横版英雄图
- 玻璃拟态桌面界面：窗口真透明 + 亚克力模糊 + 虚化背景详情页
- 游戏详情：状态、进度、时长、评分、备注
- 交互式匹配，也可手动指定 appid 或补封面
- 一键启动游戏 / 打开目录
- 生成 HTML 海报墙

## 技术栈

- Rust（edition 2024）
- [eframe / egui](https://github.com/emilk/egui) 0.27（wgpu 后端，支持窗口真透明）
- reqwest + tokio 网络抓取
- image 图像处理（高斯模糊、遮罩）

## 安装

需要 Rust 工具链。

```bash
cargo build --release
```

产物位于 `target/release/game-cabinet(.exe)`。

## 使用

双击 exe 进入图形界面；也可用命令行子命令：

```bash
# 扫描目录并抓取封面
game-cabinet scan <游戏根目录> [--covers <封面目录>] [-j 4] [--interactive]

# 对未匹配封面的游戏做交互式重新匹配
game-cabinet match --library library.json --root <根目录>

# 编辑某个游戏的信息 / 补封面
game-cabinet edit <游戏名> --status 在玩 --progress 50 --rating 8 --note "好玩"
game-cabinet edit <游戏名> --appid 730          # 按 Steam appid 刷新封面
game-cabinet edit <游戏名> --cover <图片路径或URL>

# 生成 HTML 海报墙
game-cabinet wall --library library.json --out wall.html

# 启动图形界面（默认）
game-cabinet gui
```

## 数据

游戏库数据保存在扫描根目录的 `library.json`，封面保存在 `<根目录>/covers`。二者默认不纳入版本库（见 `.gitignore`）。

## 许可证

MIT
