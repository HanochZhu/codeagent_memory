use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use anyhow::{bail, Result};
use rusqlite::OptionalExtension;
use rust_stemmers::{Algorithm, Stemmer};
use serde::Serialize;
use unicode_script::{Script, UnicodeScript};
use uuid::Uuid;

use super::ebbinghaus::INITIAL_STABILITY_DAYS;
use super::embed::{encode_f32, Embedder};
use crate::db;
use crate::project::Project;

#[derive(Debug, Serialize)]
pub struct AddResult {
    pub id: String,
    pub parent_id: Option<String>,
    pub supersedes_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    pub summary: String,
}

/// One memory to store. `parent`, `supersedes`, and `links` take an existing
/// memory id, or `#0`, `#1`, ... for another item in the same `add_memories` call.
#[derive(Debug, Clone)]
pub struct NewMemory {
    pub summary: String,
    pub body: String,
    pub parent: Option<String>,
    pub supersedes: Option<String>,
    pub links: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AddBatchResult {
    pub memories: Vec<AddResult>,
}

pub fn add_solution(
    project: &Project,
    embedder: &dyn Embedder,
    summary: &str,
    body: &str,
    parent_id: Option<&str>,
    supersedes_id: Option<&str>,
) -> Result<AddResult> {
    add_solution_with_links(
        project,
        embedder,
        summary,
        body,
        parent_id,
        supersedes_id,
        &[],
    )
}

pub fn add_solution_with_links(
    project: &Project,
    embedder: &dyn Embedder,
    summary: &str,
    body: &str,
    parent_id: Option<&str>,
    supersedes_id: Option<&str>,
    links: &[String],
) -> Result<AddResult> {
    let batch = insert_memories(
        project,
        embedder,
        &[NewMemory {
            summary: summary.to_string(),
            body: body.to_string(),
            parent: parent_id.map(str::to_string),
            supersedes: supersedes_id.map(str::to_string),
            links: links.to_vec(),
        }],
        ErrorStyle::Single,
    )?;
    Ok(batch
        .memories
        .into_iter()
        .next()
        .expect("single memory insert returns one row"))
}

/// Store every memory in one transaction. A `#N` reference points at the item
/// at that index in `memories`. Nothing is written if any item fails.
pub fn add_memories(
    project: &Project,
    embedder: &dyn Embedder,
    memories: &[NewMemory],
) -> Result<AddBatchResult> {
    if memories.is_empty() {
        bail!("memories must contain at least one memory");
    }
    insert_memories(project, embedder, memories, ErrorStyle::Batch)
}

enum ErrorStyle {
    Single,
    Batch,
}

enum MemoryRef {
    Index(usize),
    Id(String),
}

struct Draft {
    id: String,
    summary: String,
    body: String,
    parent: Option<MemoryRef>,
    supersedes: Option<MemoryRef>,
    links: Vec<MemoryRef>,
}

fn insert_memories(
    project: &Project,
    embedder: &dyn Embedder,
    memories: &[NewMemory],
    style: ErrorStyle,
) -> Result<AddBatchResult> {
    let drafts = prepare_drafts(memories, &style)?;
    project.ensure_initialized()?;
    let conn = db::open_db(&project.db_path())?;
    let prepared = resolve_drafts(&conn, &drafts, &style)?;
    let embeddings = embed_drafts(embedder, &prepared)?;
    let model = embedder.model_id();
    let now = chrono::Utc::now().timestamp();

    let tx = conn.unchecked_transaction()?;
    for (draft, embedding) in prepared.iter().zip(&embeddings) {
        let fts_text = tokenize_for_fts(&draft.summary, &draft.body);
        tx.execute(
            "INSERT INTO solutions(id, parent_id, supersedes_id, summary, body, created_at, updated_at, recalled_at, stability, embedding, embedding_model, fts_text)
             VALUES (?1, NULL, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                draft.id,
                draft.summary,
                draft.body,
                now,
                now,
                now,
                INITIAL_STABILITY_DAYS,
                embedding,
                model,
                fts_text
            ],
        )?;
    }
    for draft in &prepared {
        if draft.parent_id.is_some() || draft.supersedes_id.is_some() {
            tx.execute(
                "UPDATE solutions SET parent_id = ?1, supersedes_id = ?2 WHERE id = ?3",
                rusqlite::params![draft.parent_id, draft.supersedes_id, draft.id],
            )?;
        }
    }
    for draft in &prepared {
        for target_id in &draft.links {
            tx.execute(
                "INSERT OR IGNORE INTO solution_links(source_id, target_id) VALUES (?1, ?2)",
                rusqlite::params![draft.id, target_id],
            )?;
        }
    }
    tx.commit()?;

    Ok(AddBatchResult {
        memories: prepared
            .into_iter()
            .map(|draft| AddResult {
                id: draft.id,
                parent_id: draft.parent_id,
                supersedes_id: draft.supersedes_id,
                links: draft.links,
                summary: draft.summary,
            })
            .collect(),
    })
}

struct ResolvedDraft {
    id: String,
    summary: String,
    body: String,
    parent_id: Option<String>,
    supersedes_id: Option<String>,
    links: Vec<String>,
}

fn prepare_drafts(memories: &[NewMemory], style: &ErrorStyle) -> Result<Vec<Draft>> {
    let count = memories.len();
    let mut drafts = Vec::with_capacity(count);
    for (index, memory) in memories.iter().enumerate() {
        let summary = memory.summary.trim();
        if summary.is_empty() {
            bail!("{}", missing_summary(style, index));
        }
        if memory.body.trim().is_empty() {
            bail!("{}", missing_body(style, index));
        }
        require_english(&summary_field(style, index), summary)?;
        let at = |field: &str| field_at(style, index, field);
        drafts.push(Draft {
            id: new_id(),
            summary: summary.to_string(),
            body: memory.body.clone(),
            parent: optional_ref(memory.parent.as_deref(), count, &at("parent"))?,
            supersedes: optional_ref(memory.supersedes.as_deref(), count, &at("supersedes"))?,
            links: memory
                .links
                .iter()
                .filter(|link| !link.trim().is_empty())
                .map(|link| parse_ref(link, count, &at("links")))
                .collect::<Result<_>>()?,
        });
    }
    Ok(drafts)
}

fn resolve_drafts(
    conn: &rusqlite::Connection,
    drafts: &[Draft],
    style: &ErrorStyle,
) -> Result<Vec<ResolvedDraft>> {
    let mut explicit_parent = Vec::with_capacity(drafts.len());
    let mut supersedes_of = Vec::with_capacity(drafts.len());
    let mut existing_parent: HashMap<String, Option<String>> = HashMap::new();

    for (index, draft) in drafts.iter().enumerate() {
        explicit_parent.push(match &draft.parent {
            Some(target) => Some(resolve_target(
                drafts, draft, target, index, "parent", style,
            )?),
            None => None,
        });
        if let Some(target) = &draft.parent {
            if let MemoryRef::Id(id) = target {
                require_memory(conn, "parent", id)?;
            }
        }
        supersedes_of.push(match &draft.supersedes {
            Some(target) => {
                let id = resolve_target(drafts, draft, target, index, "supersedes", style)?;
                if let MemoryRef::Id(existing) = target {
                    let parent = require_memory(conn, "superseded", existing)?;
                    let successor: Option<String> = conn
                        .query_row(
                            "SELECT id FROM solutions WHERE supersedes_id = ?1",
                            [existing],
                            |row| row.get(0),
                        )
                        .optional()?;
                    if let Some(successor) = successor {
                        bail!("memory {existing} is already superseded by {successor}");
                    }
                    existing_parent.insert(existing.clone(), parent);
                }
                Some(id)
            }
            None => None,
        });
    }

    ensure_no_supersedes_cycle(drafts)?;

    let mut seen_supersedes = HashSet::new();
    for (index, target) in supersedes_of.iter().enumerate() {
        if let Some(target) = target {
            if !seen_supersedes.insert(target.clone()) {
                bail!(
                    "{} supersedes {target}, which is already superseded in this call",
                    field_at(style, index, "supersedes")
                );
            }
        }
    }

    let mut parents = Vec::with_capacity(drafts.len());
    for index in 0..drafts.len() {
        parents.push(inherited_parent(
            index,
            drafts,
            &explicit_parent,
            &existing_parent,
            &mut Vec::new(),
        )?);
    }

    let mut resolved = Vec::with_capacity(drafts.len());
    for (index, draft) in drafts.iter().enumerate() {
        let mut links = Vec::new();
        let mut seen = HashSet::new();
        for target in &draft.links {
            let id = resolve_target(drafts, draft, target, index, "links", style)?;
            if let MemoryRef::Id(existing) = target {
                require_memory(conn, "linked", existing)?;
            }
            if seen.insert(id.clone()) {
                links.push(id);
            }
        }
        resolved.push(ResolvedDraft {
            id: draft.id.clone(),
            summary: draft.summary.clone(),
            body: draft.body.clone(),
            parent_id: parents[index].clone(),
            supersedes_id: supersedes_of[index].clone(),
            links,
        });
    }
    Ok(resolved)
}

fn ensure_no_supersedes_cycle(drafts: &[Draft]) -> Result<()> {
    for start in 0..drafts.len() {
        let mut seen = HashSet::new();
        let mut current = start;
        loop {
            if !seen.insert(current) {
                bail!("memories[{start}] supersedes a memory that supersedes it");
            }
            match &drafts[current].supersedes {
                Some(MemoryRef::Index(next)) => current = *next,
                _ => break,
            }
        }
    }
    Ok(())
}

fn inherited_parent(
    index: usize,
    drafts: &[Draft],
    explicit_parent: &[Option<String>],
    existing_parent: &HashMap<String, Option<String>>,
    stack: &mut Vec<usize>,
) -> Result<Option<String>> {
    if let Some(parent) = &explicit_parent[index] {
        return Ok(Some(parent.clone()));
    }
    let Some(supersedes) = &drafts[index].supersedes else {
        return Ok(None);
    };
    if stack.contains(&index) {
        bail!("memories[{index}] supersedes a memory whose parent depends on itself");
    }
    stack.push(index);
    let parent = match supersedes {
        MemoryRef::Id(id) => existing_parent.get(id).cloned().flatten(),
        MemoryRef::Index(other) => {
            inherited_parent(*other, drafts, explicit_parent, existing_parent, stack)?
        }
    };
    stack.pop();
    Ok(parent)
}

fn resolve_target(
    drafts: &[Draft],
    draft: &Draft,
    target: &MemoryRef,
    index: usize,
    field: &str,
    style: &ErrorStyle,
) -> Result<String> {
    let id = match target {
        MemoryRef::Index(other) => drafts[*other].id.clone(),
        MemoryRef::Id(id) => id.clone(),
    };
    if id == draft.id {
        bail!("{} cannot refer to itself", field_at(style, index, field));
    }
    Ok(id)
}

fn embed_drafts(embedder: &dyn Embedder, drafts: &[ResolvedDraft]) -> Result<Vec<Vec<u8>>> {
    drafts
        .iter()
        .map(|draft| {
            Ok(encode_f32(
                &embedder.embed(&format!("{}\n{}", draft.summary, draft.body))?,
            ))
        })
        .collect()
}

fn optional_ref(raw: Option<&str>, count: usize, at: &str) -> Result<Option<MemoryRef>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Ok(Some(parse_ref(value, count, at)?)),
        None => Ok(None),
    }
}

fn parse_ref(raw: &str, count: usize, at: &str) -> Result<MemoryRef> {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('#') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            let index: usize = match rest.parse() {
                Ok(index) => index,
                Err(_) => bail!("{at} index #{rest} is too large"),
            };
            if index >= count {
                bail!("{at} index #{index} is out of range; this call has {count} memories");
            }
            return Ok(MemoryRef::Index(index));
        }
    }
    if raw.is_empty() {
        bail!("{at} memory id is empty");
    }
    Ok(MemoryRef::Id(raw.to_string()))
}

fn new_id() -> String {
    Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn missing_summary(style: &ErrorStyle, index: usize) -> String {
    match style {
        ErrorStyle::Single => "summary is required".to_string(),
        ErrorStyle::Batch => format!("memories[{index}].summary is required"),
    }
}

fn missing_body(style: &ErrorStyle, index: usize) -> String {
    match style {
        ErrorStyle::Single => "body is required (stdin or --file)".to_string(),
        ErrorStyle::Batch => format!("memories[{index}].body is required"),
    }
}

fn summary_field(style: &ErrorStyle, index: usize) -> String {
    match style {
        ErrorStyle::Single => "summary".to_string(),
        ErrorStyle::Batch => format!("memories[{index}].summary"),
    }
}

fn field_at(style: &ErrorStyle, index: usize, field: &str) -> String {
    match style {
        ErrorStyle::Single => field.to_string(),
        ErrorStyle::Batch => format!("memories[{index}].{field}"),
    }
}

fn require_memory(conn: &rusqlite::Connection, relation: &str, id: &str) -> Result<Option<String>> {
    match conn.query_row(
        "SELECT parent_id FROM solutions WHERE id = ?1",
        [id],
        |row| row.get(0),
    ) {
        Ok(parent) => Ok(parent),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            bail!("{relation} memory not found: {id}")
        }
        Err(err) => Err(err.into()),
    }
}

pub fn require_english(field: &str, text: &str) -> Result<()> {
    if let Some(ch) = text.chars().find(|ch| {
        ch.is_alphabetic()
            && !matches!(
                ch.script(),
                Script::Latin | Script::Common | Script::Inherited
            )
    }) {
        bail!(
            "{field} must be written in English; found non-Latin character `{ch}` (U+{:04X})",
            ch as u32
        );
    }
    Ok(())
}

fn stemmer() -> &'static Stemmer {
    static STEMMER: OnceLock<Stemmer> = OnceLock::new();
    STEMMER.get_or_init(|| Stemmer::create(Algorithm::English))
}

/// Text stored for BM25. A camelCase identifier is kept whole and also split
/// into its words, so `bundledToolSchemas` matches both itself and a query
/// that spells it `bundled tool schemas`. English words are stored in both
/// surface and stemmed forms, so inflections such as `stores` and `stored`
/// share a lexical term.
pub fn tokenize_for_fts(summary: &str, body: &str) -> String {
    let text = format!("{summary} {body}");
    english_terms(&text, false).join(" ")
}

/// Query terms for BM25. Function words are dropped: the query is an OR of
/// every term, so a word like `the` only adds documents that share nothing
/// else with the question.
pub fn tokenize_query(query: &str) -> String {
    english_terms(query, true).join(" ")
}

fn english_terms(text: &str, drop_stopwords: bool) -> Vec<String> {
    let mut terms = Vec::new();
    let mut query_seen = HashSet::new();
    for token in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        if token.is_empty() {
            continue;
        }
        let mut variants = Vec::new();
        let mut variant_seen = HashSet::new();
        push_term(&mut variants, &mut variant_seen, token, drop_stopwords);
        for word in identifier_words(token) {
            push_term(&mut variants, &mut variant_seen, &word, drop_stopwords);
        }
        for term in variants {
            if !drop_stopwords || query_seen.insert(term.clone()) {
                terms.push(term);
            }
        }
    }
    terms
}

fn push_term(
    terms: &mut Vec<String>,
    seen: &mut HashSet<String>,
    term: &str,
    drop_stopwords: bool,
) {
    let normalized = term.to_lowercase();
    if normalized.is_empty() || (drop_stopwords && is_stopword(&normalized)) {
        return;
    }
    if seen.insert(normalized.clone()) {
        terms.push(normalized.clone());
    }
    if normalized.chars().all(|c| c.is_ascii_alphabetic()) {
        let stemmed = stemmer().stem(&normalized).into_owned();
        if seen.insert(stemmed.clone()) {
            terms.push(stemmed);
        }
    }
}

fn identifier_words(token: &str) -> Vec<String> {
    token.split(['_', '-']).flat_map(camel_words).collect()
}

fn camel_words(token: &str) -> Vec<String> {
    let chars: Vec<char> = token.chars().collect();
    if !chars.iter().all(|c| c.is_ascii_alphanumeric()) {
        return Vec::new();
    }
    let mut words = Vec::new();
    let mut start = 0;
    for i in 1..chars.len() {
        let (prev, cur) = (chars[i - 1], chars[i]);
        let next_lower = chars.get(i + 1).is_some_and(|c| c.is_ascii_lowercase());
        let boundary = (prev.is_ascii_lowercase() && cur.is_ascii_uppercase())
            || (prev.is_ascii_uppercase() && cur.is_ascii_uppercase() && next_lower)
            || (prev.is_ascii_alphabetic() != cur.is_ascii_alphabetic());
        if boundary {
            words.push(chars[start..i].iter().collect());
            start = i;
        }
    }
    words.push(chars[start..].iter().collect());
    words
}

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "do", "does", "did", "of", "to",
    "in", "on", "for", "by", "with", "or", "and", "it", "this", "that", "how", "what", "why",
    "which", "where", "when", "who", "whether",
];

fn is_stopword(token: &str) -> bool {
    STOPWORDS.iter().any(|s| s.eq_ignore_ascii_case(token))
}

pub fn escape_fts_query(tokens: &str) -> String {
    tokens
        .split_whitespace()
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::HashEmbedder;
    use tempfile::tempdir;

    #[test]
    fn rejects_non_latin_memory_text() {
        let err = require_english("query", "How does 向量 recall work?").unwrap_err();
        assert!(err.to_string().contains("must be written in English"));
    }

    #[test]
    fn allows_english_code_and_punctuation() {
        require_english(
            "body",
            "Use `bundledToolSchemas()`—it returns Vec<Result<T, E>>.",
        )
        .unwrap();
    }

    #[test]
    fn english_tokens_split_identifiers_and_add_stems() {
        let terms = tokenize_for_fts(
            "bundledToolSchemas",
            "Stores cached results and stores metadata",
        );
        assert!(terms.split_whitespace().any(|term| term == "bundled"));
        assert!(terms.split_whitespace().any(|term| term == "schemas"));
        assert!(terms.split_whitespace().any(|term| term == "store"));
        assert_eq!(
            terms
                .split_whitespace()
                .filter(|term| *term == "stores")
                .count(),
            2
        );
    }

    #[test]
    fn query_tokens_drop_stopwords_and_add_stems() {
        let terms = tokenize_query("How are stored memories recalled?");
        assert_eq!(terms, "stored store memories memori recalled recal");
    }

    #[test]
    fn quotes_hyphen_and_or_tokens() {
        let q = escape_fts_query("multi-arch Docker OR");
        assert_eq!(q, "\"multi-arch\" OR \"Docker\" OR \"OR\"");
    }

    #[test]
    fn revision_inherits_structural_parent_and_stays_linear() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let topic = add_solution(
            &project,
            &embedder,
            "Retry policy topic",
            "Retry policy topic body",
            None,
            None,
        )
        .unwrap();
        let old = add_solution(
            &project,
            &embedder,
            "Fixed retry policy",
            "Use fixed retry delays.",
            Some(&topic.id),
            None,
        )
        .unwrap();
        let revision = add_solution(
            &project,
            &embedder,
            "Jittered retry policy",
            "Use jittered retry delays.",
            None,
            Some(&old.id),
        )
        .unwrap();

        assert_eq!(revision.parent_id.as_deref(), Some(topic.id.as_str()));
        assert_eq!(revision.supersedes_id.as_deref(), Some(old.id.as_str()));

        let err = add_solution(
            &project,
            &embedder,
            "Alternative retry policy",
            "Use an alternative retry delay.",
            None,
            Some(&old.id),
        )
        .unwrap_err();
        assert!(err.to_string().contains("already superseded"), "{err:#}");
    }

    #[test]
    fn allows_non_latin_body_when_adding_solution() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let res = add_solution(
            &project,
            &embedder,
            "English summary for multilingual memory",
            "记忆正文支持非拉丁字符：中文、日本語、한국어、кириллица，以及各种符号。",
            None,
            None,
        );
        assert!(res.is_ok(), "{:?}", res.err());
    }

    #[test]
    fn rejects_non_latin_summary_when_adding_solution() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let res = add_solution(
            &project,
            &embedder,
            "非英文描述摘要",
            "English body text.",
            None,
            None,
        );
        let err = res.unwrap_err();
        assert!(
            err.to_string()
                .contains("summary must be written in English"),
            "{err:#}"
        );
    }

    #[test]
    fn links_connect_related_atomic_facts_and_validate_targets() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let fact_a = add_solution(
            &project,
            &embedder,
            "SQLite stores the project graph in .cam/cam.db",
            "The project database lives at .cam/cam.db.",
            None,
            None,
        )
        .unwrap();
        let fact_b = add_solution_with_links(
            &project,
            &embedder,
            "WAL mode is enabled on every SQLite connection",
            "open_db enables PRAGMA journal_mode = WAL.",
            None,
            None,
            &[fact_a.id.clone(), fact_a.id.clone()],
        )
        .unwrap();
        assert_eq!(fact_b.links, vec![fact_a.id.clone()]);

        let view_a = crate::memory::show_solution(&project, &fact_a.id).unwrap();
        let view_b = crate::memory::show_solution(&project, &fact_b.id).unwrap();
        assert_eq!(view_a.links, vec![fact_b.id.clone()]);
        assert_eq!(view_b.links, vec![fact_a.id.clone()]);

        let missing = add_solution_with_links(
            &project,
            &embedder,
            "Orphan link test",
            "Should fail when linked memory is missing.",
            None,
            None,
            &["deadbeef1234".to_string()],
        )
        .unwrap_err();
        assert!(
            missing.to_string().contains("linked memory not found"),
            "{missing:#}"
        );
    }

    fn memory_count(project: &Project) -> i64 {
        let conn = project.connect().unwrap();
        conn.query_row("SELECT COUNT(*) FROM solutions", [], |row| row.get(0))
            .unwrap()
    }

    fn memory(summary: &str, body: &str) -> NewMemory {
        NewMemory {
            summary: summary.to_string(),
            body: body.to_string(),
            parent: None,
            supersedes: None,
            links: Vec::new(),
        }
    }

    #[test]
    fn batch_resolves_index_refs_and_inherits_parent() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let topic = add_solution(
            &project,
            &embedder,
            "Tool result size topic",
            "How tool results are sized.",
            None,
            None,
        )
        .unwrap();
        let mut shell = memory(
            "The shell tool truncates output at 1000 lines",
            "formatLargeTextForModel keeps the last 1000 lines.",
        );
        shell.parent = Some(topic.id.clone());
        let mut images = memory(
            "Images use a separate byte cap",
            "Images over 5 MB are replaced with a note.",
        );
        images.parent = Some("#0".to_string());
        images.links = vec!["#0".to_string(), "#0".to_string()];
        let mut revision = memory("The image cap is 4 MB", "The image cap is now 4 MB.");
        revision.supersedes = Some("#1".to_string());

        let batch = add_memories(&project, &embedder, &[shell, images, revision]).unwrap();
        assert_eq!(
            batch.memories[0].parent_id.as_deref(),
            Some(topic.id.as_str())
        );
        assert_eq!(
            batch.memories[1].parent_id.as_deref(),
            Some(batch.memories[0].id.as_str())
        );
        assert_eq!(batch.memories[1].links, vec![batch.memories[0].id.clone()]);
        assert_eq!(
            batch.memories[2].supersedes_id.as_deref(),
            Some(batch.memories[1].id.as_str())
        );
        assert_eq!(
            batch.memories[2].parent_id.as_deref(),
            Some(batch.memories[0].id.as_str())
        );

        let shown = crate::memory::show_solution(&project, &batch.memories[0].id).unwrap();
        assert_eq!(shown.links, vec![batch.memories[1].id.clone()]);
        let replaced = crate::memory::show_solution(&project, &batch.memories[1].id).unwrap();
        assert!(!replaced.latest);
    }

    #[test]
    fn batch_failure_writes_nothing() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let mut broken = memory(
            "Images use a separate byte cap",
            "Images over 5 MB are dropped.",
        );
        broken.summary = "图片上限".to_string();
        broken.links = vec!["#0".to_string()];
        let err = add_memories(
            &project,
            &embedder,
            &[
                memory(
                    "The shell tool truncates output at 1000 lines",
                    "formatLargeTextForModel keeps the last 1000 lines.",
                ),
                broken,
            ],
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("memories[1].summary must be written in English"),
            "{err:#}"
        );
        assert_eq!(memory_count(&project), 0);

        let mut out_of_range = memory("A second fact", "Detail.");
        out_of_range.links = vec!["#4".to_string()];
        let err = add_memories(
            &project,
            &embedder,
            &[
                memory(
                    "The shell tool truncates output at 1000 lines",
                    "Tail of 1000 lines.",
                ),
                out_of_range,
            ],
        )
        .unwrap_err();
        assert!(err.to_string().contains("out of range"), "{err:#}");
        assert_eq!(memory_count(&project), 0);
    }
}
