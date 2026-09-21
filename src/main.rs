use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;

use anyhow::{bail, Result};
use cam::code::{
    clamp_debounce_ms, index_project, ls, read, refs, sync_project, watch_project, RefDir,
    DEFAULT_DEBOUNCE_MS,
};
use cam::memory::{add_solution, format_tree, show_solution, solution_tree, Fusion};
use cam::ops::{load_embedder, resolve_project};
use cam::output::{emit_error_json, emit_json, emit_text};
use cam::project::Project;
use cam::Config;
use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use serde::Serialize;

#[derive(Parser)]
#[command(name = "cam", version, about = "CodeAgent memory: code graph + memory (CLI + MCP)")]
struct Cli {
    /// Project root (otherwise CAM_PROJECT, then .cam / .git walk-up, else cwd)
    #[arg(long, global = true, value_name = "DIR")]
    project: Option<PathBuf>,
    /// Print compact JSON instead of text
    #[arg(long, global = true)]
    json: bool,
    /// Pretty-print JSON (implies --json)
    #[arg(long, global = true)]
    pretty: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse the project with tree-sitter into SQLite
    Index,
    /// Incrementally update the graph for files that changed
    Sync,
    /// Watch the project and update the graph when files change
    Watch {
        /// Quiet window in ms after the last relevant event (default 2000)
        #[arg(long, default_value_t = DEFAULT_DEBOUNCE_MS)]
        debounce_ms: u64,
    },
    /// List the indexed graph as a virtual filesystem
    Ls { virt_path: Option<String> },
    /// Read a file outline or a symbol body
    Read {
        virt_path: String,
        #[arg(long)]
        full: bool,
    },
    /// One-hop callers (in) or callees (out)
    Ref {
        symbol: String,
        /// Direction: in = callers, out = callees
        #[arg(long, value_enum)]
        dir: Option<RefDir>,
        /// Shorthand for --dir in
        #[arg(long, conflicts_with_all = ["dir", "callees"])]
        callers: bool,
        /// Shorthand for --dir out
        #[arg(long, conflicts_with_all = ["dir", "callers"])]
        callees: bool,
    },
    /// Hybrid recall: vector + BM25, fused with min-max sum or RRF
    Recall {
        query: String,
        #[arg(long, default_value_t = 3)]
        limit: usize,
        /// Score fusion: RRF (default) or min-max sum
        #[arg(long, value_enum, default_value_t = Fusion::Rrf)]
        fusion: Fusion,
        /// Skip model2vec and use the test hash embedder
        #[arg(long, hide = true)]
        hash_embed: bool,
    },
    /// Add a memory (body from --body, --file, or stdin)
    Add {
        #[arg(long)]
        summary: String,
        #[arg(long)]
        parent: Option<String>,
        /// Read the body from a file
        #[arg(long, conflicts_with = "body")]
        file: Option<PathBuf>,
        /// Pass the body inline
        #[arg(long, conflicts_with = "file")]
        body: Option<String>,
        #[arg(long, hide = true)]
        hash_embed: bool,
    },
    /// Browse the memory tree
    Mem {
        #[command(subcommand)]
        cmd: MemCmd,
    },
    /// Show the resolved project, database, and config
    Status,
    /// Read or update the global config
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Start an MCP stdio server for the main coding agent
    Mcp,
}

#[derive(Subcommand)]
enum MemCmd {
    Tree,
    Show { id: String },
}

#[derive(Subcommand)]
enum ConfigCmd {
    Get,
    Set { key: String, value: String },
}

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    let wants_json = raw.iter().any(|a| a == "--json" || a == "--pretty");
    let wants_pretty = raw.iter().any(|a| a == "--pretty");

    let cli = match Cli::try_parse_from(&raw) {
        Ok(cli) => cli,
        Err(err) => {
            let help = matches!(
                err.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            );
            if wants_json && !help {
                let message = err
                    .to_string()
                    .lines()
                    .next()
                    .unwrap_or("invalid arguments")
                    .trim_start_matches("error: ")
                    .to_string();
                let _ = emit_error_json("usage", &message, wants_pretty);
                std::process::exit(2);
            }
            let _ = err.print();
            std::process::exit(err.exit_code());
        }
    };

    let json = cli.json || cli.pretty;
    let pretty = cli.pretty;
    if let Err(err) = run(cli, json, pretty) {
        if json {
            let code = classify_error(&err);
            let _ = emit_error_json(code, &format!("{err:#}"), pretty);
        } else {
            eprintln!("error: {err:#}");
        }
        std::process::exit(1);
    }
}

fn classify_error(err: &anyhow::Error) -> &'static str {
    if err
        .chain()
        .any(|cause| cause.downcast_ref::<std::io::Error>().is_some())
    {
        return "io";
    }
    let message = err.to_string();
    if message.contains("not indexed") {
        "not_indexed"
    } else if message.contains("ambiguous") {
        "ambiguous"
    } else if message.contains("not found") || message.contains("no project found") {
        "not_found"
    } else if message.contains("required")
        || message.contains("specify")
        || message.contains("must be")
        || message.contains("does not")
        || message.contains("unsupported")
    {
        "usage"
    } else if message.contains("config") {
        "config"
    } else {
        "error"
    }
}

fn run(cli: Cli, json: bool, pretty: bool) -> Result<()> {
    let project_arg = cli.project.as_deref();
    match cli.command {
        Command::Mcp => {
            cam::mcp::serve_stdio(project_arg)?;
        }
        Command::Index => {
            let project = resolve_project(project_arg)?;
            let report = index_project(&project)?;
            if json {
                emit_json(&report, pretty)?;
            } else {
                emit_text(format!(
                    "indexed {} files, {} nodes, {} edges ({} skipped)",
                    report.files, report.nodes, report.edges, report.skipped
                ));
            }
        }
        Command::Sync => {
            let project = resolve_project(project_arg)?;
            let report = sync_project(&project)?;
            if json {
                emit_json(&report, pretty)?;
            } else {
                emit_text(format_sync(&report));
            }
        }
        Command::Watch { debounce_ms } => {
            let project = resolve_project(project_arg)?;
            let debounce_ms = clamp_debounce_ms(debounce_ms);
            if !json {
                emit_text(format!(
                    "watching {} (debounce {debounce_ms}ms)",
                    project.root.display()
                ));
            }
            let mut first = true;
            watch_project(
                &project,
                std::time::Duration::from_millis(debounce_ms),
                None,
                |report| {
                    let changed =
                        report.files_added + report.files_modified + report.files_removed;
                    if !first && changed == 0 {
                        return;
                    }
                    first = false;
                    if json {
                        let _ = emit_json(report, pretty);
                    } else {
                        emit_text(format_sync(report));
                    }
                },
            )?;
        }
        Command::Ls { virt_path } => {
            let project = resolve_project(project_arg)?;
            let entries = ls(&project, virt_path.as_deref())?;
            if json {
                emit_json(&entries, pretty)?;
            } else {
                for e in entries {
                    let extra = match (e.start_line, e.end_line) {
                        (Some(s), Some(t)) => format!("  {s}-{t}"),
                        _ => String::new(),
                    };
                    println!("{:<8} {}{}", e.kind, e.path.unwrap_or(e.name), extra);
                }
            }
        }
        Command::Read { virt_path, full } => {
            let project = resolve_project(project_arg)?;
            let result = read(&project, &virt_path, full)?;
            if json {
                emit_json(&result, pretty)?;
            } else {
                if result.kind != "outline" {
                    if let (Some(s), Some(t)) = (result.start_line, result.end_line) {
                        eprintln!("{}  {}-{}", result.path, s, t);
                    }
                }
                emit_text(result.source);
            }
        }
        Command::Ref {
            symbol,
            dir,
            callers,
            callees,
        } => {
            let dir = resolve_ref_dir(dir, callers, callees)?;
            let project = resolve_project(project_arg)?;
            let result = refs(&project, &symbol, dir)?;
            if json {
                emit_json(&result, pretty)?;
            } else if result.refs.is_empty() {
                emit_text("no refs");
            } else {
                for r in result.refs {
                    println!("{}  {}:{}", r.name, r.file_path, r.start_line);
                }
            }
        }
        Command::Recall {
            query,
            limit,
            fusion,
            hash_embed,
        } => {
            let project = resolve_project(project_arg)?;
            let embedder = load_embedder(hash_embed)?;
            let hits = cam::memory::recall(&project, embedder.as_ref(), &query, limit, fusion)?;
            if json {
                emit_json(&hits, pretty)?;
            } else if hits.is_empty() {
                emit_text("no memories");
            } else {
                for h in hits {
                    let mut flags = Vec::new();
                    if h.stale {
                        flags.push("stale");
                    }
                    if h.needs_update {
                        flags.push("needs_update");
                    }
                    if !h.latest {
                        flags.push("older");
                    }
                    let flags = if flags.is_empty() {
                        String::new()
                    } else {
                        format!("  {}", flags.join(","))
                    };
                    println!(
                        "{}  {:.3}  R={:.2}  {}d{}  {}",
                        h.id,
                        h.score,
                        h.retention,
                        h.age_days,
                        flags,
                        h.path.join(" > ")
                    );
                    println!("{}", trim_body(&h.body, 400));
                    println!();
                }
            }
        }
        Command::Add {
            summary,
            parent,
            file,
            body,
            hash_embed,
        } => {
            let project = resolve_project(project_arg)?;
            let body = read_body(file.as_ref(), body)?;
            let embedder = load_embedder(hash_embed)?;
            let added = add_solution(
                &project,
                embedder.as_ref(),
                &summary,
                &body,
                parent.as_deref(),
            )?;
            if json {
                emit_json(&added, pretty)?;
            } else {
                emit_text(format!("added {}", added.id));
            }
        }
        Command::Mem { cmd } => {
            let project = resolve_project(project_arg)?;
            match cmd {
                MemCmd::Tree => {
                    let tree = solution_tree(&project)?;
                    if json {
                        emit_json(&tree, pretty)?;
                    } else {
                        let text = format_tree(&tree, 0);
                        if text.is_empty() {
                            emit_text("no memories");
                        } else {
                            print!("{text}");
                        }
                    }
                }
                MemCmd::Show { id } => {
                    let view = show_solution(&project, &id)?;
                    if json {
                        emit_json(&view, pretty)?;
                    } else {
                        println!(
                            "{}  R={:.2}  {}d  {}{}",
                            view.id,
                            view.retention,
                            view.age_days,
                            if view.stale { "stale " } else { "" },
                            if view.needs_update {
                                "needs_update"
                            } else {
                                "ok"
                            }
                        );
                        println!("{}", view.summary);
                        println!("{}", view.body);
                    }
                }
            }
        }
        Command::Status => {
            let project = resolve_project(project_arg)?;
            let out = collect_status(&project)?;
            if json {
                emit_json(&out, pretty)?;
            } else {
                println!("root         {}", out.root);
                println!(
                    "db           {}{}",
                    out.db,
                    if out.db_exists { "" } else { " (missing)" }
                );
                println!("files        {}", out.files);
                println!("nodes        {}", out.nodes);
                println!("edges        {}", out.edges);
                println!("solutions    {}", out.solutions);
                println!("stale_days   {}", out.stale_days);
                println!("hash_embed   {}", out.hash_embed);
                println!("require_model2vec {}", out.require_model2vec);
            }
        }
        Command::Config { cmd } => match cmd {
            ConfigCmd::Get => {
                let cfg = Config::load()?;
                if json {
                    emit_json(&cfg, pretty)?;
                } else {
                    emit_text(format!("stale_days = {}", cfg.stale_days));
                }
            }
            ConfigCmd::Set { key, value } => {
                if key != "stale_days" {
                    bail!("unsupported config key `{key}`; only `stale_days` is supported");
                }
                let stale_days: u32 = value
                    .trim()
                    .parse()
                    .map_err(|_| anyhow::anyhow!("stale_days must be a positive integer"))?;
                let mut cfg = Config::load()?;
                cfg.stale_days = stale_days;
                cfg.save()?;
                if json {
                    emit_json(&cfg, pretty)?;
                } else {
                    emit_text(format!("stale_days = {}", cfg.stale_days));
                }
            }
        },
    }
    Ok(())
}

fn resolve_ref_dir(dir: Option<RefDir>, callers: bool, callees: bool) -> Result<RefDir> {
    if callers {
        return Ok(RefDir::In);
    }
    if callees {
        return Ok(RefDir::Out);
    }
    dir.ok_or_else(|| anyhow::anyhow!("specify --dir in|out (or --callers / --callees)"))
}

fn read_body(file: Option<&PathBuf>, body: Option<String>) -> Result<String> {
    if let Some(body) = body {
        return Ok(body);
    }
    if let Some(path) = file {
        return Ok(fs::read_to_string(path)?);
    }
    let mut stdin = io::stdin();
    if stdin.is_terminal() {
        bail!("provide body via --body, --file, or stdin");
    }
    let mut buf = String::new();
    stdin.read_to_string(&mut buf)?;
    Ok(buf)
}

fn trim_body(body: &str, max: usize) -> String {
    if body.chars().count() <= max {
        return body.to_string();
    }
    let clipped: String = body.chars().take(max).collect();
    format!("{clipped}…")
}

fn format_sync(report: &cam::code::SyncReport) -> String {
    format!(
        "synced +{} ~{} -{}  ({} files, {} nodes, {} edges, {}ms)",
        report.files_added,
        report.files_modified,
        report.files_removed,
        report.files_checked,
        report.nodes,
        report.edges,
        report.duration_ms
    )
}

fn collect_status(project: &Project) -> Result<StatusOut> {
    let db = project.db_path();
    let db_exists = db.is_file();
    let cfg = Config::load()?;
    let (files, nodes, edges, solutions) = if db_exists {
        let conn = cam::open_db(&db)?;
        (
            cam::db::table_count(&conn, "files")?,
            cam::db::table_count(&conn, "nodes")?,
            cam::db::table_count(&conn, "edges")?,
            cam::db::table_count(&conn, "solutions")?,
        )
    } else {
        (0, 0, 0, 0)
    };
    Ok(StatusOut {
        root: project.root.display().to_string(),
        db: db.display().to_string(),
        db_exists,
        files,
        nodes,
        edges,
        solutions,
        stale_days: cfg.stale_days,
        hash_embed: cam::ops::env_flag("CAM_HASH_EMBED"),
        require_model2vec: std::env::var_os("CAM_REQUIRE_MODEL2VEC").is_some(),
    })
}

#[derive(Serialize)]
struct StatusOut {
    root: String,
    db: String,
    db_exists: bool,
    files: i64,
    nodes: i64,
    edges: i64,
    solutions: i64,
    stale_days: u32,
    hash_embed: bool,
    require_model2vec: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_error_maps_known_codes() {
        assert_eq!(
            classify_error(&anyhow::anyhow!("symbol not found: foo")),
            "not_found"
        );
        assert_eq!(
            classify_error(&anyhow::anyhow!("no project found")),
            "not_found"
        );
        assert_eq!(
            classify_error(&anyhow::anyhow!("ambiguous symbol `foo`")),
            "ambiguous"
        );
        assert_eq!(
            classify_error(&anyhow::anyhow!(
                "code graph not indexed for /repo; run `cam index` (MCP cam_index) once, then retry"
            )),
            "not_indexed"
        );
        assert_eq!(
            classify_error(&anyhow::anyhow!("unsupported config key `foo`")),
            "usage"
        );
        assert_eq!(
            classify_error(&anyhow::anyhow!("stale_days must be a positive integer")),
            "usage"
        );
        assert_eq!(
            classify_error(&anyhow::anyhow!("parse ~/.cam/config.toml")),
            "config"
        );
        assert_eq!(classify_error(&anyhow::anyhow!("boom")), "error");
    }

    #[test]
    fn classify_error_detects_io_source() {
        let err = anyhow::Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing",
        ));
        assert_eq!(classify_error(&err), "io");
    }

    #[test]
    fn ref_dir_flags_win_and_choice_is_required() {
        assert!(matches!(
            resolve_ref_dir(None, true, false).unwrap(),
            RefDir::In
        ));
        assert!(matches!(
            resolve_ref_dir(None, false, true).unwrap(),
            RefDir::Out
        ));
        assert!(matches!(
            resolve_ref_dir(Some(RefDir::Out), false, false).unwrap(),
            RefDir::Out
        ));
        assert!(resolve_ref_dir(None, false, false).is_err());
    }

    #[test]
    fn trim_body_clips_on_char_boundary() {
        assert_eq!(trim_body("hello", 10), "hello");
        assert_eq!(trim_body("你好世界", 2), "你好…");
    }

    #[test]
    fn read_body_prefers_inline_body() {
        assert_eq!(read_body(None, Some("inline".into())).unwrap(), "inline");
    }
}
