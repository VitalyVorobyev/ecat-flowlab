//! With the `embed-ui` feature, checks that the built UI exists before `rust-embed` reads it,
//! and rebuilds the crate when it changes.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_EMBED_UI").is_none() {
        return;
    }
    let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    assert!(
        dist.join("index.html").is_file(),
        "feature `embed-ui` needs the built UI in {}: run `bun install && bun run build` in ui/ first",
        dist.display()
    );
}
