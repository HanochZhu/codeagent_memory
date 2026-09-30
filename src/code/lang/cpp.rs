use tree_sitter::Node;

use super::{collect_child_kind_heritage, has_ancestor, ImplBlock};

pub const DEFS: &str = r#"
(function_declarator declarator: (identifier) @name) @def
(function_declarator declarator: (field_identifier) @name) @def
(function_declarator declarator: (qualified_identifier name: (identifier) @name)) @def
(class_specifier name: (type_identifier) @name) @def
(struct_specifier name: (type_identifier) @name) @def
(enum_specifier name: (type_identifier) @name) @def
(type_definition declarator: (type_identifier) @name) @def
(alias_declaration name: (type_identifier) @name) @def
"#;

pub const CALLS: &str = r#"
(call_expression function: (identifier) @call)
(call_expression function: (field_expression field: (field_identifier) @call))
(call_expression function: (qualified_identifier name: (identifier) @call))
"#;

pub const REFS: &str = r#"
(type_identifier) @ref
"#;

pub fn classify_kind(node: Node) -> &'static str {
    match node.kind() {
        "function_declarator" if is_cpp_method(node) => "method",
        "function_declarator" => "function",
        "class_specifier" => "class",
        "enum_specifier" => "enum",
        "type_definition" | "alias_declaration" => "type_alias",
        _ => "struct",
    }
}

pub fn collect_impl(node: Node, source: &str, out: &mut Vec<ImplBlock>) {
    collect_child_kind_heritage(
        node,
        source,
        &["class_specifier", "struct_specifier"],
        "base_class_clause",
        out,
    );
}

fn is_cpp_method(node: Node) -> bool {
    if has_ancestor(node, "class_specifier") || has_ancestor(node, "struct_specifier") {
        return true;
    }
    node.child_by_field_name("declarator")
        .is_some_and(|n| n.kind() == "qualified_identifier")
}
