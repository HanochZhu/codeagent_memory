use tree_sitter::Node;

use super::{has_ancestor, keep_ref_name, trailing_type_name, ImplBlock};

pub const DEFS: &str = r#"
(function_item name: (identifier) @name) @def
(struct_item name: (type_identifier) @name) @def
(enum_item name: (type_identifier) @name) @def
(enum_variant name: (identifier) @name) @def
(trait_item name: (type_identifier) @name) @def
(type_item name: (type_identifier) @name) @def
"#;

pub const CALLS: &str = r#"
(call_expression function: (identifier) @call)
(call_expression function: (field_expression field: (field_identifier) @call))
(call_expression function: (scoped_identifier name: (identifier) @call))
(call_expression function: (generic_function function: (identifier) @call))
(call_expression function: (generic_function function: (field_expression field: (field_identifier) @call)))
(call_expression function: (generic_function function: (scoped_identifier name: (identifier) @call)))
"#;

pub const REFS: &str = r#"
(type_identifier) @ref
(scoped_type_identifier name: (type_identifier) @ref)
(scoped_identifier path: (identifier) @ref)
(scoped_identifier path: (scoped_identifier name: (identifier) @ref))
"#;

pub fn classify_kind(node: Node) -> &'static str {
    match node.kind() {
        "function_item" if has_ancestor(node, "impl_item") => "method",
        "function_item" => "function",
        "trait_item" => "trait",
        "enum_item" => "enum",
        "type_item" => "type_alias",
        _ => "struct",
    }
}

pub fn collect_impl(node: Node, source: &str, out: &mut Vec<ImplBlock>) {
    if node.kind() != "impl_item" {
        return;
    }
    let trait_name = node
        .child_by_field_name("trait")
        .and_then(|n| trailing_type_name(n, source));
    let Some(type_name) = node
        .child_by_field_name("type")
        .and_then(|n| trailing_type_name(n, source))
    else {
        return;
    };
    if keep_ref_name(&type_name) {
        out.push(super::impl_from(
            node,
            type_name,
            trait_name.filter(|t| keep_ref_name(t)),
        ));
    }
}
