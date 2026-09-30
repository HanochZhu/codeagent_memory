use tree_sitter::Node;

use super::{collect_child_kind_heritage, ImplBlock};

pub const DEFS: &str = r#"
(class_declaration name: (identifier) @name) @def
(interface_declaration name: (identifier) @name) @def
(struct_declaration name: (identifier) @name) @def
(enum_declaration name: (identifier) @name) @def
(record_declaration name: (identifier) @name) @def
(method_declaration name: (identifier) @name) @def
(constructor_declaration name: (identifier) @name) @def
"#;

pub const CALLS: &str = r#"
(invocation_expression function: (identifier) @call)
(invocation_expression function: (member_access_expression name: (identifier) @call))
"#;

pub const REFS: &str = r#"
(variable_declaration type: (identifier) @ref)
(parameter type: (identifier) @ref)
(object_creation_expression type: (identifier) @ref)
"#;

pub fn classify_kind(node: Node) -> &'static str {
    match node.kind() {
        "method_declaration" | "constructor_declaration" => "method",
        "interface_declaration" => "trait",
        "enum_declaration" => "enum",
        "struct_declaration" => "struct",
        _ => "class",
    }
}

pub fn collect_impl(node: Node, source: &str, out: &mut Vec<ImplBlock>) {
    collect_child_kind_heritage(
        node,
        source,
        &[
            "class_declaration",
            "interface_declaration",
            "struct_declaration",
            "record_declaration",
        ],
        "base_list",
        out,
    );
}
