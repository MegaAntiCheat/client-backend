use std::{env, path::Path};

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

    // Embed icon on Windows
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        match std::fs::File::open("assets/icon.png") {
            Ok(file) => {
                // Create a new, empty icon collection and add the PNG to it
                let image = ico::IconImage::read_png(file).expect("Could not read PNG file");
                let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);
                icon_dir.add_entry(
                    ico::IconDirEntry::encode(&image)
                        .expect("Could not add PNG file to icon collection"),
                );

                // Write the ICO file to disk
                let file = std::fs::File::create("assets/icon.ico")
                    .expect("Could not create assets/icon.ico");
                icon_dir
                    .write(file)
                    .expect("Could not write assets/icon.ico to disk.");

                embed_resource::compile("assets/icon.rc", embed_resource::NONE);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // No icon found, don't worry about it
            }
            Err(e) => {
                // Found icon but there was a problem
                panic!("Could not open icon file: {e}");
            }
        }
    }
}
