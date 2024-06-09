use std::{env, path::Path};
use embed_resource;

fn main() {
    // If there's a file at `<project root>/ui/index.html`, enable the `include-ui` feature.
    // This results in the ui directory being bundled into the final binary.
    if Path::new(
        env::var("CARGO_MANIFEST_DIR")
            .expect("environment variable should be provided by Cargo when running")
            .as_str(),
    )
    .join("ui/index.html")
    .is_file()
    {
        println!("cargo::rustc-cfg=feature=\"include-ui\"");
    }
    // Compile and link icon.rc
    embed_resource::compile("assets/icon.rc", embed_resource::NONE);
}
