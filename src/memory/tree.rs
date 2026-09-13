use anyhow::{bail, Result};
use serde::Serialize;

use crate::config::Config;
use crate::db;
use crate::memory::ebbinghaus::{c0, needs_update, retention};
use crate::project::Project;

#[derive(Debug, Serialize)]
pub struct TreeNode {
    pub id: String,
    pub summary: String,
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Serialize)]
pub struct SolutionView {
    pub id: String,
    pub parent_id: Option<String>,
    pub summary: String,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub stale: bool,
    pub needs_update: bool,
    pub retention: f64,
    pub age_days: i64,
}

pub fn solution_tree(project: &Project) -> Result<Vec<TreeNode>> {
    let conn = db::open_db(&project.db_path())?;
    let mut stmt =
        conn.prepare("SELECT id, parent_id, summary FROM solutions ORDER BY created_at")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    fn build(parent: Option<&str>, rows: &[(String, Option<String>, String)]) -> Vec<TreeNode> {
        rows.iter()
            .filter(|(_, p, _)| p.as_deref() == parent)
            .map(|(id, _, summary)| TreeNode {
                id: id.clone(),
                summary: summary.clone(),
                children: build(Some(id), rows),
            })
            .collect()
    }
    Ok(build(None, &rows))
}

pub fn show_solution(project: &Project, id: &str) -> Result<SolutionView> {
    let conn = db::open_db(&project.db_path())?;
    let cfg = Config::load()?;
    let found = conn.query_row(
        "SELECT id, parent_id, summary, body, created_at, updated_at, recalled_at, stability
         FROM solutions WHERE id = ?1",
        [id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, f64>(7)?,
            ))
        },
    );
    let (id, parent_id, summary, body, created_at, updated_at, recalled_at, stability) = match found
    {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => bail!("solution not found: {id}"),
        Err(e) => return Err(e.into()),
    };
    let now = chrono::Utc::now().timestamp();
    let r = retention(now, c0(recalled_at, created_at), stability);
    Ok(SolutionView {
        id,
        parent_id,
        summary,
        body,
        created_at,
        updated_at,
        stale: cfg.is_stale(updated_at),
        needs_update: needs_update(r),
        retention: r,
        age_days: Config::age_days(updated_at),
    })
}

pub fn format_tree(nodes: &[TreeNode], indent: usize) -> String {
    let mut out = String::new();
    for node in nodes {
        out.push_str(&format!(
            "{}{}  {}\n",
            "  ".repeat(indent),
            node.id,
            node.summary
        ));
        out.push_str(&format_tree(&node.children, indent + 1));
    }
    out
}
