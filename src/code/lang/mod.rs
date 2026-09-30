mod c;
mod cpp;
mod csharp;
mod go;
mod java;
mod javascript;
mod python;
mod rust;

use std::path::Path;

use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
}

#[derive(Debug, Clone)]
pub(crate) struct Def {
    pub kind: String,
    pub name: String,
    pub start_line: i64,
    pub end_line: i64,
    pub start_byte: usize,
    pub end_byte: usize,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelKind {
    Calls,
    References,
}

#[derive(Debug, Clone)]
pub(crate) struct Rel {
    pub kind: RelKind,
    pub name: String,
    pub qualifier: Option<String>,
    pub line: i64,
    pub byte: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ImplBlock {
    pub type_name: String,
    pub trait_name: Option<String>,
    pub start_byte: usize,
    pub end_byte: usize,
    pub line: i64,
}

pub(crate) struct Parsed {
    pub defs: Vec<Def>,
    pub rels: Vec<Rel>,
    pub impls: Vec<ImplBlock>,
}

struct Syntax {
    language: Language,
    defs: &'static str,
    calls: &'static str,
    refs: Option<&'static str>,
    classify: fn(tree_sitter::Node) -> &'static str,
    collect_impl: fn(tree_sitter::Node, &str, &mut Vec<ImplBlock>),
}

const SKIP_REF_NAMES: &[&str] = &[
    "Self", "self", "super", "crate", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16",
    "u32", "u64", "u128", "usize", "f32", "f64", "bool", "str", "char", "never", "void", "int",
    "long", "short", "float", "double", "unsigned", "signed", "const", "auto", "size_t", "boolean",
    "byte", "var", "dynamic", "object", "string", "nullptr", "NULL", "this", "base",
];

impl Lang {
    pub(crate) fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Some(Self::Rust),
            "py" => Some(Self::Python),
            "js" | "mjs" | "cjs" | "jsx" => Some(Self::JavaScript),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "c" => Some(Self::C),
            "h" | "hh" | "hpp" | "hxx" | "cc" | "cpp" | "cxx" | "c++" => Some(Self::Cpp),
            "cs" => Some(Self::CSharp),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript | Self::Tsx => "typescript",
            Self::Go => "go",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "csharp",
        }
    }

    fn syntax(self) -> Syntax {
        match self {
            Self::Rust => Syntax {
                language: tree_sitter_rust::LANGUAGE.into(),
                defs: rust::DEFS,
                calls: rust::CALLS,
                refs: Some(rust::REFS),
                classify: rust::classify_kind,
                collect_impl: rust::collect_impl,
            },
            Self::Python => Syntax {
                language: tree_sitter_python::LANGUAGE.into(),
                defs: python::DEFS,
                calls: python::CALLS,
                refs: None,
                classify: python::classify_kind,
                collect_impl: python::collect_impl,
            },
            Self::JavaScript => Syntax {
                language: tree_sitter_javascript::LANGUAGE.into(),
                defs: javascript::JS_DEFS,
                calls: javascript::CALLS,
                refs: None,
                classify: javascript::classify_kind,
                collect_impl: javascript::collect_impl,
            },
            Self::TypeScript => Syntax {
                language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                defs: javascript::TS_DEFS,
                calls: javascript::CALLS,
                refs: Some(javascript::TS_REFS),
                classify: javascript::classify_kind,
                collect_impl: javascript::collect_impl,
            },
            Self::Tsx => Syntax {
                language: tree_sitter_typescript::LANGUAGE_TSX.into(),
                defs: javascript::TS_DEFS,
                calls: javascript::CALLS,
                refs: Some(javascript::TS_REFS),
                classify: javascript::classify_kind,
                collect_impl: javascript::collect_impl,
            },
            Self::Go => Syntax {
                language: tree_sitter_go::LANGUAGE.into(),
                defs: go::DEFS,
                calls: go::CALLS,
                refs: None,
                classify: go::classify_kind,
                collect_impl: skip_impl,
            },
            Self::Java => Syntax {
                language: tree_sitter_java::LANGUAGE.into(),
                defs: java::DEFS,
                calls: java::CALLS,
                refs: Some(java::REFS),
                classify: java::classify_kind,
                collect_impl: java::collect_impl,
            },
            Self::C => Syntax {
                language: tree_sitter_c::LANGUAGE.into(),
                defs: c::DEFS,
                calls: c::CALLS,
                refs: Some(c::REFS),
                classify: c::classify_kind,
                collect_impl: skip_impl,
            },
            Self::Cpp => Syntax {
                language: tree_sitter_cpp::LANGUAGE.into(),
                defs: cpp::DEFS,
                calls: cpp::CALLS,
                refs: Some(cpp::REFS),
                classify: cpp::classify_kind,
                collect_impl: cpp::collect_impl,
            },
            Self::CSharp => Syntax {
                language: tree_sitter_c_sharp::LANGUAGE.into(),
                defs: csharp::DEFS,
                calls: csharp::CALLS,
                refs: Some(csharp::REFS),
                classify: csharp::classify_kind,
                collect_impl: csharp::collect_impl,
            },
        }
    }
}

pub(crate) fn parse_source(lang: Lang, source: &str) -> Result<Parsed> {
    let syntax = lang.syntax();
    let mut parser = Parser::new();
    parser.set_language(&syntax.language)?;
    let tree = parser
        .parse(source, None)
        .context("tree-sitter parse returned none")?;
    let root = tree.root_node();

    let def_query = Query::new(&syntax.language, syntax.defs)?;
    let mut cursor = QueryCursor::new();
    let mut defs = Vec::new();
    let mut matches = cursor.matches(&def_query, root, source.as_bytes());
    while let Some(m) = matches.next() {
        let mut def_node = None;
        let mut name = None;
        for cap in m.captures {
            let cap_name = def_query.capture_names()[cap.index as usize];
            match cap_name {
                "def" => def_node = Some(cap.node),
                "name" => name = Some(cap.node.utf8_text(source.as_bytes())?.to_string()),
                _ => {}
            }
        }
        let (Some(node), Some(name)) = (def_node, name) else {
            continue;
        };
        let first_line = source
            .get(node.start_byte()..node.end_byte())
            .and_then(|s| s.lines().next())
            .map(|s| s.trim().to_string());
        defs.push(Def {
            kind: (syntax.classify)(node).to_string(),
            name,
            start_line: (node.start_position().row + 1) as i64,
            end_line: (node.end_position().row + 1) as i64,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            signature: first_line,
        });
    }

    let mut rels = collect_named(
        &syntax.language,
        root,
        source,
        syntax.calls,
        "call",
        RelKind::Calls,
    )?;
    if let Some(q) = syntax.refs {
        rels.extend(collect_named(
            &syntax.language,
            root,
            source,
            q,
            "ref",
            RelKind::References,
        )?);
        rels.retain(|r| r.kind != RelKind::References || keep_ref_name(&r.name));
    }

    let mut impls = Vec::new();
    walk_impls(syntax.collect_impl, root, source, &mut impls);

    Ok(Parsed { defs, rels, impls })
}

fn collect_named(
    language: &Language,
    root: tree_sitter::Node,
    source: &str,
    query: &str,
    capture: &str,
    kind: RelKind,
) -> Result<Vec<Rel>> {
    let q = Query::new(language, query)?;
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let mut matches = cursor.matches(&q, root, source.as_bytes());
    while let Some(m) = matches.next() {
        for cap in m.captures {
            if q.capture_names()[cap.index as usize] != capture {
                continue;
            }
            if kind == RelKind::References && is_type_def_name(cap.node) {
                continue;
            }
            let name = cap.node.utf8_text(source.as_bytes())?.to_string();
            if name.is_empty() {
                continue;
            }
            if kind == RelKind::References
                && cap.node.kind() == "identifier"
                && !looks_like_type_name(&name)
            {
                continue;
            }
            out.push(Rel {
                kind,
                name,
                qualifier: (kind == RelKind::Calls)
                    .then(|| call_qualifier(cap.node, source))
                    .flatten(),
                line: (cap.node.start_position().row + 1) as i64,
                byte: cap.node.start_byte(),
            });
        }
    }
    Ok(out)
}

fn is_type_def_name(node: tree_sitter::Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    let def_item = matches!(
        parent.kind(),
        "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "enum_variant"
            | "class_declaration"
            | "class_definition"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "struct_declaration"
            | "class_specifier"
            | "struct_specifier"
            | "enum_specifier"
            | "type_definition"
            | "alias_declaration"
            | "type_spec"
    );
    if !def_item {
        return false;
    }
    parent
        .child_by_field_name("name")
        .map(|n| n.id() == node.id())
        .unwrap_or(false)
}

fn keep_ref_name(name: &str) -> bool {
    if name.len() <= 1 {
        return false;
    }
    !SKIP_REF_NAMES.contains(&name)
}

fn looks_like_type_name(name: &str) -> bool {
    keep_ref_name(name) && name.chars().next().is_some_and(|c| c.is_uppercase())
}

fn type_qualifier(node: tree_sitter::Node, source: &str) -> Option<String> {
    trailing_type_name(node, source).filter(|s| looks_like_type_name(s))
}

fn field_qualifier(parent: tree_sitter::Node, field: &str, source: &str) -> Option<String> {
    parent
        .child_by_field_name(field)
        .and_then(|n| type_qualifier(n, source))
}

fn call_qualifier(node: tree_sitter::Node, source: &str) -> Option<String> {
    let mut cur = node;
    if let Some(parent) = node.parent() {
        if parent.kind() == "generic_function" {
            cur = parent;
        }
    }
    let parent = cur.parent()?;
    match parent.kind() {
        "scoped_identifier" | "scoped_type_identifier" => field_qualifier(parent, "path", source),
        "generic_function" => parent
            .child_by_field_name("function")
            .filter(|f| matches!(f.kind(), "scoped_identifier" | "scoped_type_identifier"))
            .and_then(|scoped| field_qualifier(scoped, "path", source)),
        "qualified_identifier" => field_qualifier(parent, "scope", source),
        "field_expression" => field_qualifier(parent, "argument", source),
        "member_expression" | "member_access_expression" => parent
            .child_by_field_name("object")
            .or_else(|| parent.child_by_field_name("expression"))
            .and_then(|n| type_qualifier(n, source)),
        "method_invocation" => field_qualifier(parent, "object", source),
        _ => None,
    }
}

fn skip_impl(_node: tree_sitter::Node, _source: &str, _out: &mut Vec<ImplBlock>) {}

fn walk_impls(
    collect: fn(tree_sitter::Node, &str, &mut Vec<ImplBlock>),
    node: tree_sitter::Node,
    source: &str,
    out: &mut Vec<ImplBlock>,
) {
    collect(node, source, out);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_impls(collect, child, source, out);
    }
}

fn impl_from(node: tree_sitter::Node, type_name: String, trait_name: Option<String>) -> ImplBlock {
    ImplBlock {
        type_name,
        trait_name,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        line: (node.start_position().row + 1) as i64,
    }
}

fn named_type(node: tree_sitter::Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|n| trailing_type_name(n, source))
}

fn push_heritage_base(
    out: &mut Vec<ImplBlock>,
    node: tree_sitter::Node,
    type_name: &str,
    base: &str,
) {
    if keep_ref_name(base) && base != type_name {
        out.push(impl_from(
            node,
            type_name.to_string(),
            Some(base.to_string()),
        ));
    }
}

fn collect_child_kind_heritage(
    node: tree_sitter::Node,
    source: &str,
    type_kinds: &[&str],
    child_kind: &str,
    out: &mut Vec<ImplBlock>,
) {
    if !type_kinds.contains(&node.kind()) {
        return;
    }
    let Some(type_name) = named_type(node, source) else {
        return;
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != child_kind {
            continue;
        }
        each_type_name(child, source, &mut |base| {
            push_heritage_base(out, node, &type_name, base);
        });
    }
}

fn collect_field_heritage(
    node: tree_sitter::Node,
    source: &str,
    type_kinds: &[&str],
    fields: &[&str],
    out: &mut Vec<ImplBlock>,
) {
    if !type_kinds.contains(&node.kind()) {
        return;
    }
    let Some(type_name) = named_type(node, source) else {
        return;
    };
    for field in fields {
        if let Some(bases) = node.child_by_field_name(field) {
            each_type_name(bases, source, &mut |base| {
                push_heritage_base(out, node, &type_name, base);
            });
        }
    }
}

fn trailing_type_name(node: tree_sitter::Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier"
        | "identifier"
        | "property_identifier"
        | "field_identifier"
        | "namespace_identifier" => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string()),
        "generic_type" | "template_type" => node
            .child_by_field_name("type")
            .or_else(|| node.child_by_field_name("name"))
            .and_then(|n| trailing_type_name(n, source)),
        "generic_name" => node
            .child_by_field_name("name")
            .or_else(|| node.child(0))
            .and_then(|n| trailing_type_name(n, source)),
        "scoped_type_identifier"
        | "scoped_identifier"
        | "member_expression"
        | "qualified_identifier"
        | "nested_identifier"
        | "member_access_expression" => node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("property"))
            .and_then(|n| trailing_type_name(n, source))
            .or_else(|| {
                node.child(node.child_count().saturating_sub(1))
                    .and_then(|n| trailing_type_name(n, source))
            }),
        "reference_type" | "pointer_type" => node
            .child_by_field_name("type")
            .and_then(|n| trailing_type_name(n, source)),
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(name) = trailing_type_name(child, source) {
                    return Some(name);
                }
            }
            None
        }
    }
}

fn collect_ident_leaves(node: tree_sitter::Node, source: &str, visit: &mut impl FnMut(&str)) {
    if matches!(node.kind(), "identifier" | "type_identifier") {
        if let Ok(text) = node.utf8_text(source.as_bytes()) {
            visit(text);
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ident_leaves(child, source, visit);
    }
}

fn each_type_name(node: tree_sitter::Node, source: &str, visit: &mut impl FnMut(&str)) {
    match node.kind() {
        "type_list" | "base_list" | "base_class_clause" | "super_interfaces" | "argument_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                each_type_name(child, source, visit);
            }
        }
        _ => {
            if let Some(name) = trailing_type_name(node, source) {
                visit(&name);
            }
        }
    }
}

fn has_ancestor(mut node: tree_sitter::Node, kind: &str) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == kind {
            return true;
        }
        node = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_names(parsed: &Parsed) -> Vec<&str> {
        parsed.defs.iter().map(|d| d.name.as_str()).collect()
    }

    fn has_def(parsed: &Parsed, name: &str, kind: &str) -> bool {
        parsed.defs.iter().any(|d| d.name == name && d.kind == kind)
    }

    fn has_rel(parsed: &Parsed, kind: RelKind, name: &str) -> bool {
        parsed.rels.iter().any(|r| r.kind == kind && r.name == name)
    }

    fn has_impl(parsed: &Parsed, ty: &str, tr: &str) -> bool {
        parsed
            .impls
            .iter()
            .any(|i| i.type_name == ty && i.trait_name.as_deref() == Some(tr))
    }

    #[test]
    fn from_path_maps_supported_extensions() {
        assert_eq!(Lang::from_path(Path::new("src/lib.rs")), Some(Lang::Rust));
        assert_eq!(Lang::from_path(Path::new("App.java")), Some(Lang::Java));
        assert_eq!(Lang::from_path(Path::new("main.c")), Some(Lang::C));
        assert_eq!(Lang::from_path(Path::new("util.h")), Some(Lang::Cpp));
        assert_eq!(Lang::from_path(Path::new("foo.HPP")), Some(Lang::Cpp));
        assert_eq!(Lang::from_path(Path::new("bar.cpp")), Some(Lang::Cpp));
        assert_eq!(Lang::from_path(Path::new("Program.cs")), Some(Lang::CSharp));
        assert_eq!(Lang::from_path(Path::new("notes.md")), None);
    }

    #[test]
    fn parse_rust_functions_and_calls() {
        let src = r#"
pub fn add(a: i32, b: i32) -> i32 { a + b }
pub fn run() { let _ = add(1, 2); helper(); }
fn helper() {}
"#;
        let parsed = parse_source(Lang::Rust, src).unwrap();
        let names = def_names(&parsed);
        assert!(names.contains(&"add"));
        assert!(names.contains(&"run"));
        assert!(names.contains(&"helper"));
        assert!(has_rel(&parsed, RelKind::Calls, "add"));
        assert!(has_rel(&parsed, RelKind::Calls, "helper"));
    }

    #[test]
    fn parse_rust_enum_variants_and_aliases() {
        let src = r#"
type Result<T> = std::result::Result<T, ()>;
enum Command { DoSomething { arg: String } }
"#;
        let parsed = parse_source(Lang::Rust, src).unwrap();
        let names = def_names(&parsed);
        assert!(names.contains(&"Result"));
        assert!(names.contains(&"Command"));
        assert!(names.contains(&"DoSomething"));
        assert!(has_def(&parsed, "Command", "enum"));
        assert!(has_def(&parsed, "Result", "type_alias"));
    }

    #[test]
    fn parse_rust_impl_trait_and_type_refs() {
        let src = r#"
pub struct Foo;
pub trait Clone { fn clone(&self); }
impl Clone for Foo {
    fn clone(&self) {}
}
pub fn use_foo(x: Foo) { let _ = x.clone(); }
"#;
        let parsed = parse_source(Lang::Rust, src).unwrap();
        assert!(has_impl(&parsed, "Foo", "Clone"));
        assert!(has_rel(&parsed, RelKind::References, "Foo"));
        assert!(has_rel(&parsed, RelKind::Calls, "clone"));
    }

    #[test]
    fn parse_rust_generic_and_qualified_calls() {
        let src = r#"
pub struct Foo;
impl Foo {
    fn new() -> Self { Foo }
    fn get_one<T>(&self) {}
}
pub fn run(x: Foo) {
    let _ = Foo::new();
    x.get_one::<u8>();
}
"#;
        let parsed = parse_source(Lang::Rust, src).unwrap();
        assert!(parsed.rels.iter().any(|r| r.kind == RelKind::Calls
            && r.name == "new"
            && r.qualifier.as_deref() == Some("Foo")));
        assert!(has_rel(&parsed, RelKind::Calls, "get_one"));
        assert!(has_rel(&parsed, RelKind::References, "Foo"));
    }

    #[test]
    fn parse_js_class_heritage() {
        let src = "class Foo extends Bar { method() { this.x(); } }\nclass Bar {}\n";
        let parsed = parse_source(Lang::JavaScript, src).unwrap();
        assert!(has_impl(&parsed, "Foo", "Bar"));
    }

    #[test]
    fn parse_python_functions_and_calls() {
        let src = "def helper():\n    return 1\ndef run():\n    return helper()\n";
        let parsed = parse_source(Lang::Python, src).unwrap();
        assert_eq!(parsed.defs.len(), 2);
        assert!(has_rel(&parsed, RelKind::Calls, "helper"));
    }

    #[test]
    fn parse_go_functions_and_calls() {
        let src = r#"
package p
type Point struct { X int }
func helper() int { return 1 }
func run() int { return helper() }
"#;
        let parsed = parse_source(Lang::Go, src).unwrap();
        let names = def_names(&parsed);
        assert!(names.contains(&"Point"));
        assert!(names.contains(&"helper"));
        assert!(names.contains(&"run"));
        assert!(has_rel(&parsed, RelKind::Calls, "helper"));
    }

    #[test]
    fn parse_java_class_heritage_and_calls() {
        let src = r#"
class Bar {}
interface Runnable {}
class Foo extends Bar implements Runnable {
  void helper() {}
  void run() { helper(); }
}
"#;
        let parsed = parse_source(Lang::Java, src).unwrap();
        assert!(has_def(&parsed, "Foo", "class"));
        assert!(has_def(&parsed, "Runnable", "trait"));
        assert!(has_impl(&parsed, "Foo", "Bar"));
        assert!(has_impl(&parsed, "Foo", "Runnable"));
        assert!(has_rel(&parsed, RelKind::Calls, "helper"));
    }

    #[test]
    fn parse_c_functions_and_structs() {
        let src = r#"
struct Point { int x; };
int add(int a, int b) { return a + b; }
int run(void) { return add(1, 2); }
"#;
        let parsed = parse_source(Lang::C, src).unwrap();
        assert!(has_def(&parsed, "Point", "struct"));
        assert!(has_def(&parsed, "add", "function"));
        assert!(has_rel(&parsed, RelKind::Calls, "add"));
    }

    #[test]
    fn parse_cpp_class_heritage_and_qualified_methods() {
        let src = r#"
class Bar {};
class Foo : public Bar {
public:
  void helper();
};
void Foo::helper() {}
void run(Foo* f) { f->helper(); }
"#;
        let parsed = parse_source(Lang::Cpp, src).unwrap();
        assert!(has_def(&parsed, "Foo", "class"));
        assert!(has_def(&parsed, "helper", "method"));
        assert!(has_impl(&parsed, "Foo", "Bar"));
        assert!(has_rel(&parsed, RelKind::Calls, "helper"));
        assert!(has_rel(&parsed, RelKind::References, "Foo"));
    }

    #[test]
    fn parse_csharp_class_heritage_and_calls() {
        let src = r#"
class Bar {}
interface IRun {}
class Foo : Bar, IRun {
  void Helper() {}
  void Run() { Helper(); }
}
"#;
        let parsed = parse_source(Lang::CSharp, src).unwrap();
        assert!(has_def(&parsed, "Foo", "class"));
        assert!(has_def(&parsed, "IRun", "trait"));
        assert!(has_impl(&parsed, "Foo", "Bar"));
        assert!(has_impl(&parsed, "Foo", "IRun"));
        assert!(has_rel(&parsed, RelKind::Calls, "Helper"));
    }
}
