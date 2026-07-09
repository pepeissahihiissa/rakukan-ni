//! rakukan エンジン CLI
//!
//! v0.3.0 時点の動作確認ツール。対話的にローマ字を入力して変換結果を確認できる。
//!
//! # 使い方
//! ```
//! cargo run -p rakukan-engine-cli                     # 対話モード
//! cargo run -p rakukan-engine-cli -- --once nihongo   # 非対話モード
//! cargo run -p rakukan-engine-cli -- --list-models    # モデル一覧
//! cargo run -p rakukan-engine-cli -- --model jinen-v1-small-q5
//!
//! # 評価モード（読みを直接モデルに渡す）
//! cargo run -p rakukan-engine-cli -- --eval --reading パソコンパソコンパソコン
//! cargo run -p rakukan-engine-cli -- --eval --reading パソコンパソコン --context "前に変換した文章だ"
//! cargo run -p rakukan-engine-cli -- --eval --reading パソコン --instr "長さを保って変換:" --raw
//! ```

use anyhow::Result;
use clap::Parser;
use rakukan_engine::{EngineConfig, RakunEngine};
use std::io::{self, BufRead, Write};

#[derive(Parser, Debug)]
#[command(
    name = "rakukan-cli",
    about = "rakukan 変換エンジン CLI（v0.3.0 動作確認）",
    long_about = None
)]
struct Args {
    /// モデル variant ID（省略: レジストリのデフォルト = jinen-v1-xsmall-q5）
    #[arg(short, long)]
    model: Option<String>,

    /// 変換候補の最大数
    #[arg(short = 'n', long, default_value = "5")]
    num_candidates: usize,

    /// llama.cpp スレッド数（0 = 自動）
    #[arg(short = 't', long, default_value = "0")]
    threads: u32,

    /// 非対話モード: 指定ローマ字を変換して終了
    #[arg(long)]
    once: Option<String>,

    /// 利用可能なモデル一覧を表示して終了
    #[arg(long)]
    list_models: bool,

    /// ログレベル
    #[arg(long, default_value = "warn")]
    log: String,

    // ── 評価モード ─────────────────────────────────────
    /// 評価モード: 読み（ひらがな/カタカナ）を直接モデルに渡す
    #[arg(long)]
    eval: bool,

    /// 評価モード用 読み（ひらがな/カタカナ）
    #[arg(long)]
    reading: Option<String>,

    /// 評価モード用 文脈テキスト
    #[arg(long, default_value = "")]
    context: String,

    /// 評価モード用 プロンプト指示（context と input の間に挿入）
    #[arg(long, default_value = "")]
    instr: String,

    /// 生のbeam候補を表示（RUST_LOG=debug 相当）
    #[arg(long)]
    raw: bool,

    /// マージ後候補も表示（LLM単体 と 辞書+学習+LLMマージ を比較）
    #[arg(long)]
    show_merge: bool,

    /// マージ候補上限数（--show-merge と併用）
    #[arg(long, default_value = "40")]
    merge_limit: usize,

    /// trim_output_repetition と長さガードを無効にする（診断用）
    #[arg(long)]
    no_trim: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let log_level = if args.raw { "debug" } else { &args.log };
    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .with_writer(io::stderr)
        .init();

    if args.list_models {
        print_model_list();
        return Ok(());
    }

    let config = EngineConfig {
        model_variant: args.model.clone(),
        num_candidates: args.num_candidates,
        n_threads: args.threads,
        no_trim: args.no_trim,
        ..EngineConfig::default()
    };

    if args.eval {
        return run_eval(&config, &args);
    }

    eprintln!("rakukan エンジン CLI  ─  v0.3.0");
    eprintln!("モデルを初期化中...");
    eprintln!("（初回は HuggingFace からダウンロードします）");

    let mut engine = RakunEngine::new(config);
    engine.init_kanji()?;

    eprintln!("✓ 準備完了\n");

    if let Some(input) = args.once {
        run_once(&mut engine, &input, args.num_candidates)?;
    } else {
        run_interactive(&mut engine, args.num_candidates)?;
    }

    Ok(())
}

// ── 評価モード ──────────────────────────────────────────

fn run_eval(config: &EngineConfig, args: &Args) -> Result<()> {
    let reading = args.reading.as_deref().unwrap_or("");
    if reading.is_empty() {
        anyhow::bail!("--eval requires --reading <TEXT>");
    }

    eprintln!("rakukan 評価モード");
    eprintln!("モデルを初期化中...");
    let mut engine = RakunEngine::new(config.clone());
    engine.init_kanji()?;
    engine.init_dict();
    eprintln!("✓ 準備完了\n");

    let llm_candidates = engine.convert_reading(reading, &args.context, args.num_candidates)?;

    println!("読み:     {}", reading);
    println!("文脈:     {}", args.context);
    if !args.instr.is_empty() {
        println!("指示:     {}", args.instr);
    }
    println!();

    println!("=== LLM単体 ===");
    print_candidates(&llm_candidates);

    if args.show_merge {
        let merged = engine.merge_candidates_for_reading(reading, llm_candidates.clone(), args.merge_limit);
        println!();
        println!("=== マージ後 (学習/辞書/LLM) ===");
        print_candidates(&merged);

        // 差分表示: LLMにない候補
        let added: Vec<&String> = merged.iter().filter(|c| !llm_candidates.contains(c)).collect();
        if !added.is_empty() {
            println!();
            println!("--- LLMにはなかった候補（学習/辞書由来） ---");
            for c in added {
                println!("  + {} ({}chars)", c, c.chars().count());
            }
        }
    }

    Ok(())
}

// ── 非対話モード ─────────────────────────────────────────

fn run_once(engine: &mut RakunEngine, romaji: &str, n: usize) -> Result<()> {
    for c in romaji.chars() {
        engine.push_char(c);
    }

    let reading = engine.current_preedit().hiragana;
    eprintln!("入力:     {}", romaji);
    eprintln!("ひらがな: {}", reading);

    let candidates = engine.convert(n)?;
    eprintln!("変換候補:");
    for (i, c) in candidates.iter().enumerate() {
        println!("{}: {}", i + 1, c);
    }
    Ok(())
}

// ── 対話モード ───────────────────────────────────────────

fn run_interactive(engine: &mut RakunEngine, n: usize) -> Result<()> {
    eprintln!("ローマ字を入力 → Enter で変換");
    eprintln!("コマンド: reset (リセット) / quit (終了)");
    eprintln!();

    let stdin = io::stdin();
    let stdout = io::stdout();

    loop {
        print!("ローマ字> ");
        stdout.lock().flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();

        match line {
            "" => {
                eprintln!("（入力なし）");
            }
            "quit" | "exit" | "q" => {
                eprintln!("終了します。");
                break;
            }
            "reset" | "r" => {
                engine.reset_preedit();
                eprintln!("リセットしました。");
            }
            input => {
                for c in input.chars() {
                    engine.push_char(c);
                }
                let reading = engine.current_preedit().hiragana;
                eprintln!("ひらがな: {}  →  変換中...", reading);

                let candidates = engine.convert(n)?;
                if candidates.is_empty() {
                    eprintln!("  （候補なし）\n");
                    continue;
                }

                for (i, c) in candidates.iter().enumerate() {
                    let mark = if i == 0 { "▶" } else { " " };
                    println!("{} {}: {}", mark, i + 1, c);
                }

                // 候補選択
                print!("\n番号を選択 [1]: ");
                stdout.lock().flush()?;
                let mut choice = String::new();
                stdin.lock().read_line(&mut choice)?;
                let choice = choice.trim();

                let committed = if let Ok(idx) = choice.parse::<usize>() {
                    candidates
                        .get(idx.saturating_sub(1))
                        .cloned()
                        .unwrap_or_else(|| candidates[0].clone())
                } else {
                    candidates[0].clone()
                };

                engine.commit(&committed);
                eprintln!("確定: 「{}」", committed);
                eprintln!("文脈:  「{}」\n", engine.committed_text());
            }
        }
    }
    Ok(())
}

// ── 表示ヘルパー ──────────────────────────────────────────

fn print_candidates(candidates: &[String]) {
    if candidates.is_empty() {
        println!("  （候補なし）");
    } else {
        for (i, c) in candidates.iter().enumerate() {
            let mark = if i == 0 { "▶" } else { " " };
            println!("{} {}: {} ({}chars)", mark, i + 1, c, c.chars().count());
        }
    }
}

// ── モデル一覧 ────────────────────────────────────────────

fn print_model_list() {
    let models = RakunEngine::available_models();
    println!("利用可能なモデル:");
    println!("{:<40} {}", "ID", "表示名");
    println!("{}", "─".repeat(70));
    for m in &models {
        let mark = if m.is_default {
            " ← デフォルト"
        } else {
            ""
        };
        println!("{:<40} {}{}", m.id, m.display_name, mark);
    }
    println!("\n使い方: rakukan-cli --model <ID>");
}
