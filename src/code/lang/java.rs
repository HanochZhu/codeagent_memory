use tree_sitter::Node;

use super::{collect_field_heritage, ImplBlock};

pub const DEFS: &str = r#"
(class_declaration name: (identifier) @name) @def
(interface_declaration name: (identifier) @name) @def
(enum_declaration name: (identifier) @name) @def
(record_declaration name: (identifier) @name) @def
(method_declaration name: (identifier) @name) @def
(constructor_declaration name: (identifier) @name) @def
"#;

pub const CALLS: &str = r#"
(method_invocation name: (identifier) @call)
"#;

pub const REFS: &str = r#"
(type_identifier) @ref
"#;

pub fn classify_kind(node: Node) -> &'static str {
    match node.kind() {
        "method_declaration" | "constructor_declaration" => "method",
        "interface_declaration" => "trait",
        "enum_declaration" => "enum",
        _ => "class",
    }
}

pub fn collect_impl(node: Node, source: &str, out: &mut Vec<ImplBlock>) {
    collect_field_heritage(
        node,
        source,
        &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "record_declaration",
        ],
        &["superclass", "interfaces"],
        out,
    );
}
