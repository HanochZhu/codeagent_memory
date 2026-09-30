use tree_sitter::Node;

use super::{collect_ident_leaves, has_ancestor, named_type, push_heritage_base, ImplBlock};

pub const DEFS: &str = r#"
(function_definition name: (identifier) @name) @def
(class_definition name: (identifier) @name) @def
"#;

pub const CALLS: &str = r#"
(call function: (identifier) @call)
(call function: (attribute attribute: (identifier) @call))
"#;

pub fn classify_kind(node: Node) -> &'static str {
    match node.kind() {
        "function_definition" if has_ancestor(node, "class_definition") => "method",
        "function_definition" => "function",
        _ => "class",
    }
}

pub fn collect_impl(node: Node, source: &str, out: &mut Vec<ImplBlock>) {
    if node.kind() != "class_definition" {
        return;
    }
    let Some(type_name) = named_type(node, source) else {
        return;
    };
    let Some(supers) = node.child_by_field_name("superclasses") else {
        return;
    };
    collect_ident_leaves(supers, source, &mut |base| {
        push_heritage_base(out, node, &type_name, base);
    });
}
