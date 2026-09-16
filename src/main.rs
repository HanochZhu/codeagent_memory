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
use cam::output::{emit_json, emit_text};
use cam::project::Project;
use clap::{Parser, Subcommand};
use serde::Serialize;

#[derive(Parser)]
#[command(name = "cam", version, about = "CodeAgent memory: code graph + solution recall")]
struct Cli {
    /// Print JSON instead of compact text
    #[arg(long, global = true)]
    json: bool,
    /// Project root (otherwise walk up for .cam / .git)
    #[arg(long, global = true)]
    path: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create .cam/ and register the project
    Init { path: Option<PathBuf> },
    /// Parse the project with tree-sitter into SQLite
    Index { path: Option<PathBuf> },
    /// Incrementally update the graph for files that changed
    Sync { path: Option<PathBuf> },
    /// Watch the project and update the graph when files change
    Watch {
        path: Option<PathBuf>,
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
        #[arg(long, value_enum)]
        dir: RefDir,
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
    /// Add a solution (summary + body from --file or stdin)
    Add {
        #[arg(long)]
        summary: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, hide = true)]
        hash_embed: bool,
    },
    /// Browse the solution tree
    Mem {
        #[command(subcommand)]
        cmd: MemCmd,
    },
    /// Start an MCP stdio server for the main coding agent
    Mcp,
}

#[derive(Subcommand)]
enum MemCmd {
    Tree,
    Show { id: String },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { path } => {
            let project = Project::init(path.as_deref().or(cli.path.as_deref()))?;
            let _ = cam::open_db(&project.db_path())?;
            let payload = InitOut {
                root: project.root.display().to_string(),
                db: project.db_path().display().to_string(),
            };
            if cli.json {
                emit_json(&payload)?;
            } else {
                emit_text(format!("initialized {}", payload.root));
            }
        }
        Command::Mcp => {
            cam::mcp::serve_stdio(cli.path.as_deref())?;
        }
        Command::Index { path } => {
            let project = resolve_project(cli.path.as_deref().or(path.as_deref()))?;
            project.ensure_initialized()?;
            let report = index_project(&project)?;
            if cli.json {
                emit_json(&report)?;
            } else {
                emit_text(format!(
                    "indexed {} files, {} nodes, {} edges ({} skipped)",
                    report.files, report.nodes, report.edges, report.skipped
                ));
            }
        }
        Command::Sync { path } => {
            let project = resolve_project(cli.path.as_deref().or(path.as_deref()))?;
            project.ensure_initialized()?;
            let report = sync_project(&project)?;
            if cli.json {
                emit_json(&report)?;
            } else {
                emit_text(format_sync(&report));
            }
        }
        Command::Watch { path, debounce_ms } => {
            let project = resolve_project(cli.path.as_deref().or(path.as_deref()))?;
            project.ensure_initialized()?;
            let debounce_ms = clamp_debounce_ms(debounce_ms);
            if !cli.json {
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
                    if cli.json {
                        let _ = emit_json(report);
                    } else {
                        emit_text(format_sync(report));
                    }
                },
            )?;
        }
        Command::Ls { virt_path } => {
            let project = resolve_project(cli.path.as_deref())?;
            let entries = ls(&project, virt_path.as_deref())?;
            if cli.json {
                emit_json(&entries)?;
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
            let project = resolve_project(cli.path.as_deref())?;
            let result = read(&project, &virt_path, full)?;
            if cli.json {
                emit_json(&result)?;
            } else {
                if result.kind != "outline" {
                    if let (Some(s), Some(t)) = (result.start_line, result.end_line) {
                        eprintln!("{}  {}-{}", result.path, s, t);
                    }
                }
                emit_text(result.source);
            }
        }
        Command::Ref { symbol, dir } => {
            let project = resolve_project(cli.path.as_deref())?;
            let result = refs(&project, &symbol, dir)?;
            if cli.json {
                emit_json(&result)?;
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
            let project = resolve_project(cli.path.as_deref())?;
            let embedder = load_embedder(hash_embed)?;
            let hits = cam::memory::recall(&project, embedder.as_ref(), &query, limit, fusion)?;
            if cli.json {
                emit_json(&hits)?;
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
            hash_embed,
        } => {
            let project = resolve_project(cli.path.as_deref())?;
            let body = read_body(file.as_ref())?;
            let embedder = load_embedder(hash_embed)?;
            let added = add_solution(
                &project,
                embedder.as_ref(),
                &summary,
                &body,
                parent.as_deref(),
            )?;
            if cli.json {
                emit_json(&added)?;
            } else {
                emit_text(format!("added {}", added.id));
            }
        }
        Command::Mem { cmd } => {
            let project = resolve_project(cli.path.as_deref())?;
            match cmd {
                MemCmd::Tree => {
                    let tree = solution_tree(&project)?;
                    if cli.json {
                        emit_json(&tree)?;
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
                    if cli.json {
                        emit_json(&view)?;
                    } else {
                        println!("{}{}", view.id, if view.stale { "  stale" } else { "" });
                        println!("{}", view.summary);
                        println!("{}", view.body);
                    }
                }
            }
        }
    }
    Ok(())
}

fn read_body(file: Option<&PathBuf>) -> Result<String> {
    if let Some(path) = file {
        return Ok(fs::read_to_string(path)?);
    }
    let mut stdin = io::stdin();
    if stdin.is_terminal() {
        bail!("provide body via stdin or --file");
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

#[derive(Serialize)]
struct InitOut {
    root: String,
    db: String,
}
