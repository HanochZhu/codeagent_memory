use tree_sitter::Node;

pub const DEFS: &str = r#"
(function_declaration name: (identifier) @name) @def
(method_declaration name: (field_identifier) @name) @def
(type_declaration (type_spec name: (type_identifier) @name type: (struct_type))) @def
"#;

pub const CALLS: &str = r#"
(call_expression function: (identifier) @call)
(call_expression function: (selector_expression field: (field_identifier) @call))
"#;

pub fn classify_kind(node: Node) -> &'static str {
    match node.kind() {
        "method_declaration" => "method",
        "function_declaration" => "function",
        _ => "struct",
    }
}
