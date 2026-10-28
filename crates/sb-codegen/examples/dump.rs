//! Writes the generated workspace of the sample project: `cargo run -p sb-codegen --example dump -- <dir>`
fn main() {
    let dir = std::env::args().nth(1).expect("usage: dump <dir>");
    let files = sb_codegen::generate(&sb_model::sample::orders()).expect("sample is valid");
    sb_codegen::write_to_dir(&files, std::path::Path::new(&dir)).expect("write");
    println!("wrote {} files to {dir}", files.len());
}
