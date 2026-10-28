//! Identifier rules. The generated code derives Rust, protobuf and Kafka names from
//! the names typed in the editor, so we only accept names whose conversion is
//! lossless. That keeps `input.order_id` in the editor identical to `input.order_id`
//! in the generated Rust.

use heck::{ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};

pub fn snake(s: &str) -> String {
    s.to_snake_case()
}

pub fn pascal(s: &str) -> String {
    s.to_upper_camel_case()
}

pub fn shouty(s: &str) -> String {
    s.to_shouty_snake_case()
}

fn ascii_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `order_id`: lowercase, digits and single underscores, and stable under snake-casing.
pub fn is_snake_ident(s: &str) -> bool {
    ascii_ident(s)
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && snake(s) == s
        && !is_rust_keyword(s)
}

/// `OrderCreated`: stable under UpperCamelCase conversion (so `HTTPServer` is rejected).
pub fn is_pascal_ident(s: &str) -> bool {
    ascii_ident(s) && s.starts_with(|c: char| c.is_ascii_uppercase()) && pascal(s) == s
}

/// Kafka topic names: `[a-zA-Z0-9._-]`, at most 249 characters.
pub fn is_topic_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 249
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && s.starts_with(|c: char| c.is_ascii_alphanumeric())
}

/// Name of the Rust constant holding a topic name: `orders.created` -> `ORDERS_CREATED`.
pub fn topic_const(topic: &str) -> String {
    shouty(topic)
}

pub fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "gen"
            | "macro"
            | "override"
            | "priv"
            | "try"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_idents() {
        assert!(is_snake_ident("order_id"));
        assert!(is_snake_ident("a"));
        assert!(!is_snake_ident("orderId"));
        assert!(!is_snake_ident("Order"));
        assert!(!is_snake_ident("type"));
        assert!(!is_snake_ident("_x"));
        assert!(!is_snake_ident(""));
    }

    #[test]
    fn pascal_idents() {
        assert!(is_pascal_ident("OrderCreated"));
        assert!(!is_pascal_ident("orderCreated"));
        assert!(!is_pascal_ident("HTTPServer"));
        assert!(!is_pascal_ident("Order_Created"));
    }

    #[test]
    fn topics() {
        assert!(is_topic_name("orders.created"));
        assert!(is_topic_name("payments-v2"));
        assert!(!is_topic_name("has space"));
        assert!(!is_topic_name(".."));
        assert_eq!(topic_const("orders.created"), "ORDERS_CREATED");
        assert_eq!(topic_const("payments-v2.failed"), "PAYMENTS_V2_FAILED");
    }
}
