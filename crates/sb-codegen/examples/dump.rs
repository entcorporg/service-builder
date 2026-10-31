//! Writes the generated workspace of the showcase project: `cargo run -p sb-codegen --example dump -- <dir>`
//!
//! The showcase is the sample with every feature of `sb_model::sample` added (single-use token, scheduled job,
//! signed webhook, encryption, field rules, composite keys), so that the CI compiles the code of all of them.
fn main() {
    let dir = std::env::args().nth(1).expect("usage: dump <dir>");
    let files = sb_codegen::generate(&sb_model::sample::showcase()).expect("the showcase is valid");
    sb_codegen::write_to_dir(&files, std::path::Path::new(&dir)).expect("write");
    println!("wrote {} files to {dir}", files.len());
}
