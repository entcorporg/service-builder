//! Writes the generated workspace of the sample project: `cargo run -p sb-codegen --example dump -- <dir>`
//!
//! The sample is enriched with the other examples of `sb_model::sample`, so that the CI compiles the code
//! of the single-use token, the scheduled job and the signed webhook as well.
fn main() {
    let dir = std::env::args().nth(1).expect("usage: dump <dir>");
    let mut project = sb_model::sample::orders();
    sb_model::sample::add_single_use_token(&mut project);
    sb_model::sample::add_cleaner_job(&mut project);
    sb_model::sample::add_stripe_webhook(&mut project);
    let files = sb_codegen::generate(&project).expect("sample is valid");
    sb_codegen::write_to_dir(&files, std::path::Path::new(&dir)).expect("write");
    println!("wrote {} files to {dir}", files.len());
}
