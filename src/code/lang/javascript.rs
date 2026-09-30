use tree_sitter::Node;

use super::{impl_from, keep_ref_name, named_type, trailing_type_name, ImplBlock};

pub const JS_DEFS: &str = r#"
(function_declaration name: (identifier) @name) @def
(generator_function_declaration name: (identifier) @name) @def
(method_definition name: (property_identifier) @name) @def
(class_declaration name: (identifier) @name) @def
"#;

pub const TS_DEFS: &str = r#"
(function_declaration name: (identifier) @name) @def
(method_definition name: (property_identifier) @name) @def
(class_declaration name: (type_identifier) @name) @def
"#;

pub const CALLS: &str = r#"
(call_expression function: (identifier) @call)
(call_expression function: (member_expression property: (property_identifier) @call))
"#;

pub const TS_REFS: &str = r#"
(type_identifier) @ref
"#;

pub fn classify_kind(node: Node) -> &'static str {
    match node.kind() {
        "method_definition" => "method",
        "function_declaration" | "generator_function_declaration" => "function",
        _ => "class",
    }
}

pub fn collect_impl(node: Node, source: &str, out: &mut Vec<ImplBlock>) {
    if node.kind() != "class_declaration" {
        return;
    }
    let Some(type_name) = named_type(node, source) else {
        return;
    };
    if let Some(base) = js_superclass(node, source) {
        if keep_ref_name(&base) {
            out.push(impl_from(node, type_name, Some(base)));
        }
    }
}

fn js_superclass(node: Node, source: &str) -> Option<String> {
    if let Some(heritage) = node.child_by_field_name("superclass") {
        return trailing_type_name(heritage, source);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_heritage" {
            return trailing_type_name(child, source);
        }
    }
    None
}
