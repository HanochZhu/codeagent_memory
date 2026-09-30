use tree_sitter::Node;

pub const DEFS: &str = r#"
(function_declarator declarator: (identifier) @name) @def
(struct_specifier name: (type_identifier) @name) @def
(enum_specifier name: (type_identifier) @name) @def
(type_definition declarator: (type_identifier) @name) @def
(union_specifier name: (type_identifier) @name) @def
"#;

pub const CALLS: &str = r#"
(call_expression function: (identifier) @call)
(call_expression function: (field_expression field: (field_identifier) @call))
"#;

pub const REFS: &str = r#"
(type_identifier) @ref
"#;

pub fn classify_kind(node: Node) -> &'static str {
    match node.kind() {
        "function_declarator" => "function",
        "enum_specifier" => "enum",
        "type_definition" => "type_alias",
        _ => "struct",
    }
}
