use clap::Parser;
use std::path::{Path, PathBuf};

#[derive(Parser,Debug)]
struct Cli {
    /// Path to a custom config file
    #[arg(short, long)]
    config: Option<PathBuf>,
}

pub fn config_path() -> PathBuf {
    let args = Cli::parse();
    if let Some(path) = args.config {
        return path;
    }

    // 2. Priority 2: List of standard system locations
    let search_paths = [
        "/etc/xrossd/xrossd.toml",
        "/usr/local/etc/xrossd/xrossd.toml",
        "./config.toml", // Useful for local development
    ];

    for path in search_paths {
        let p = Path::new(path);
        if p.exists() {
            return p.to_path_buf();
        }
    }

    // 3. Fallback: Return the most likely default even if it doesn't exist 
    // (so the error message shows the user where it expected the file)
    PathBuf::from("/etc/xrossd/xrossd.toml")
}

