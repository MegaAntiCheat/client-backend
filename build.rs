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
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        match std::fs::File::open("assets/icon.png") {
            Ok(file) => {
                println!("This is windows, creating icon.");
                // Create a new, empty icon collection
                let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);
                // Add PNG file to the collection
                let image = ico::IconImage::read_png(file).expect("Could not read PNG file");
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
                // Compile and embed icon.rc
                embed_resource::compile("assets/icon.rc", embed_resource::NONE);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!("Could not find assets/icon.png, building without icon.");
            }
            Err(e) => {
                panic!("Could not open icon file: {e}");
            }
        }
    } else {
        println!("This is not windows, skipping icon creation.");
    }
}
