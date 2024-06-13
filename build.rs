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
    match std::fs::File::open("assets/icon.png") {
        // clippy complains about 'file' being unused, still functions as intended
        Ok(_file) => {
            if env::var_os("CARGO_CFG_WINDOWS").is_some() {
                println!("This is windows, creating icon.");
                // Create a new, empty icon collection
                let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);
                // Read a PNG file from disk and add it to the collection
                let file =
                    std::fs::File::open("assets/icon.png").expect("Could not find assets/icon.png");
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
            } else {
                println!("This is not windows, skipping icon creation.");
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("Could not find assets/icon.png, building without icon.");
        }
        Err(e) => {
            panic!("Could not open icon file: {e}");
        }
    }
}
