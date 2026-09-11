//! csc-app CLI 入口——本地交互游玩（UX 设计稿到位前的可玩实现）。
//!
//! ```text
//! csc-app <assets_dir> [--seed 42] [--name Player] [--auto]
//! ```

use std::io::{BufRead, Write};

use csc_app::StdinDecisionSource;
use csc_core::engine::Engine;
use csc_decision::source::AutoDecisionSource;
use csc_domain::tier::Tier;
use csc_time::clock::SimClock;
use csc_util::rng::Xoshiro256StarStar;

struct Args {
    assets_dir: String,
    seed: u64,
    name: String,
    auto: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        assets_dir: "assets".into(),
        seed: Engine::DEFAULT_SEED,
        name: "Player".into(),
        auto: false,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--seed" => {
                i += 1;
                args.seed = argv
                    .get(i)
                    .ok_or("--seed 缺值")?
                    .parse()
                    .map_err(|_| "--seed 必须为数字")?;
            }
            "--name" => {
                i += 1;
                args.name = argv.get(i).ok_or("--name 缺值")?.clone();
            }
            "--auto" => args.auto = true,
            other if !other.starts_with("--") => args.assets_dir = other.to_string(),
            other => return Err(format!("未知参数：{other}")),
        }
        i += 1;
    }
    Ok(args)
}

fn load_engine(args: &Args) -> Result<(Engine, Option<String>), String> {
    // 资产读取由 CLI 负责（核心零 IO）
    let mut standings = Vec::new();
    let mut baseline = None;
    let mut ratings = None;
    let mut rating_profile = None;
    let mut text = None;
    for entry in std::fs::read_dir(&args.assets_dir)
        .map_err(|e| format!("读取资产目录 {} 失败：{e}", args.assets_dir))?
    {
        let path = entry.map_err(|e| format!("遍历资产目录失败：{e}"))?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.starts_with("standings_") && name.ends_with(".json") {
            let content =
                std::fs::read_to_string(&path).map_err(|e| format!("读取 {name} 失败：{e}"))?;
            standings.push((name, content));
        } else if name == "roles_baseline.json" {
            baseline =
                Some(std::fs::read_to_string(&path).map_err(|e| format!("读取 {name} 失败：{e}"))?);
        } else if name == "player_ratings.json" {
            ratings =
                Some(std::fs::read_to_string(&path).map_err(|e| format!("读取 {name} 失败：{e}"))?);
        } else if name == "rating_profile.json" {
            rating_profile =
                Some(std::fs::read_to_string(&path).map_err(|e| format!("读取 {name} 失败：{e}"))?);
        }
    }
    // 叙事文案覆盖包（assets/text/zh-CN.json；缺失 = 编译期嵌入默认文案）
    let text_dir = std::path::Path::new(&args.assets_dir).join("text");
    let text_file = text_dir.join("zh-CN.json");
    if text_file.is_file() {
        text =
            Some(std::fs::read_to_string(&text_file).map_err(|e| format!("读取文案包失败：{e}"))?);
    }
    if standings.is_empty() {
        return Err(format!(
            "资产目录 {} 中没有 standings_*.json",
            args.assets_dir
        ));
    }
    let mut engine = Engine::load_from_standings(
        &standings,
        &csc_core::CalibrationAssets {
            baseline: baseline.as_deref(),
            ratings: ratings.as_deref(),
            rating_profile: rating_profile.as_deref(),
            text: text.as_deref(),
        },
        args.seed,
        SimClock::of(2026, 1, 1),
    )
    .map_err(|e| e.to_string())?;
    // 主角：从垫底队伍起步（青训新秀叙事）
    let bottom = csc_util::id::TeamId(engine.world().all_teams().len().saturating_sub(1) as u32);
    engine
        .create_protagonist(
            &args.name,
            Tier::Tier4,
            bottom,
            &mut Xoshiro256StarStar::seed(args.seed ^ 0x5EED_C0DE),
            None,
        )
        .map_err(|e| e.to_string())?;
    Ok((engine, text))
}

fn print_summary(engine: &Engine) {
    let q = engine.query();
    let s = q.world_summary();
    println!(
        "\n📅 {} | 第 {} 个月 | 队伍 {} 支 | 选手 {} 名 | 事件 {} 条 | 决策 {} 次",
        s.date,
        s.month,
        s.team_count,
        s.player_count,
        s.journal_cursor + 1,
        s.decisions_logged
    );
    if let Some(profile) = engine
        .player()
        .map(|pid| {
            engine
                .world()
                .player(pid)
                .map(|p| p.name.clone())
                .unwrap_or_default()
        })
        .and_then(|name| q.player_profile(&name))
    {
        println!(
            "👤 {}（{} 岁，{:?}，实力 {:.0}）| 队伍：{} | 年薪 {} | 合同剩 {} 年 | 现金 {} | 声誉 {}",
            profile.name,
            profile.age,
            profile.role,
            profile.power,
            profile.team.as_deref().unwrap_or("自由身"),
            profile.salary,
            profile.contract_years,
            profile.cash,
            profile.reputation
        );
    } else if let Some((name, ending)) = q.career_ending() {
        // 主角已退役（36 岁谢幕）：summary 保留主角存在感 + 结局定级
        // （2026 试玩修复：此前退役后主角行完全消失，玩家打完 20 年看不到结局）
        println!(
            "👤 {}（已退役）| 生涯结局：{}（GOAT 评分 {:.0}）",
            name,
            ending.tier.label(),
            ending.goat_score
        );
    }
}

fn print_journal(engine: &Engine, n: usize) {
    let all = engine.journal().all();
    let start = all.len().saturating_sub(n);
    println!("── 最近事件 ──");
    for e in &all[start..] {
        println!("  [{:>4}] {} | {:?}", e.seq(), e.date(), e.kind());
    }
}

fn run_auto(engine: &mut Engine, months: u32) {
    let mut auto = AutoDecisionSource;
    let start = std::time::Instant::now();
    engine
        .run_season(months as i32, &mut auto)
        .expect("auto 推进失败：内部错误");
    println!("⏩ 自动推进 {months} 个月完成（{:.2?}）", start.elapsed());
    print_journal(engine, 8);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let (mut engine, text) = load_engine(&args)?;
    println!(
        "CSC 生涯模拟器 CLI（种子 {}，主角 {}）",
        args.seed, args.name
    );
    print_summary(&engine);

    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    loop {
        print!("\n> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let cmd: Vec<&str> = line.split_whitespace().collect();
        if cmd.is_empty() {
            // 回车 = 推进 1 个月
            if args.auto {
                run_auto(&mut engine, 1);
            } else {
                let mut source = StdinDecisionSource {
                    text: csc_text::TextBundle::parse(text.as_deref())
                        .map_err(|e| e.to_string())?,
                };
                if let Err(e) = engine.advance_month(&mut source) {
                    eprintln!("⚠️ 推进失败：{e}");
                }
                print_journal(&engine, 8);
            }
            print_summary(&engine);
        } else {
            match cmd[0] {
                "auto" => {
                    let n: u32 = cmd
                        .get(1)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1)
                        .clamp(1, 240);
                    run_auto(&mut engine, n);
                    print_summary(&engine);
                }
                "summary" => print_summary(&engine),
                "ending" => {
                    // 生涯结局复盘（2026 试玩新增：玩家通关后可查看完整定论）
                    match engine.query().career_ending() {
                        Some((name, ending)) => {
                            println!(
                                "\n🏁 {} 的生涯结局：{}（GOAT 评分 {:.0}）",
                                name,
                                ending.tier.label(),
                                ending.goat_score
                            );
                            println!("   {}\n   {}", ending.verdict, ending.review);
                        }
                        None => println!("生涯尚未结束（主角 36 岁退役后可查看结局）"),
                    }
                }
                "journal" => {
                    let n: usize = cmd.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
                    print_journal(&engine, n);
                }
                "save" => {
                    let path = cmd.get(1).ok_or("save 缺文件名")?;
                    let json = serde_json::to_string_pretty(&engine.snapshot())?;
                    std::fs::write(path, json)?;
                    println!("💾 已存档 → {path}");
                }
                "load" => {
                    let path = cmd.get(1).ok_or("load 缺文件名")?;
                    let json = std::fs::read_to_string(path)?;
                    let state = csc_core::state::GameState::migrate(&json)
                        .map_err(|e| std::io::Error::other(e.to_string()))?;
                    engine.restore(state);
                    println!("📂 已读档 ← {path}");
                    print_summary(&engine);
                }
                "quit" | "exit" => break,
                other => println!(
                    "未知命令：{other}（回车推进 / auto N / save / load / summary / journal / ending / quit）"
                ),
            }
        }
    }
    println!("再见，传奇。");
    Ok(())
}
