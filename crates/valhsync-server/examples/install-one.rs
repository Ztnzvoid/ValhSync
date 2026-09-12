//! Install one dropped thing into a throwaway tree, and print what landed.
//!
//!     cargo run -p valhsync-server --example install-one -- <archive|folder|dll>
//!
//! For trying a real package from a real site against the real code path.
//! Fixtures I wrote myself only ever prove that I am consistent with myself.

fn main() -> anyhow::Result<()> {
    let Some(arg) = std::env::args().nth(1) else {
        anyhow::bail!("give me an archive, a mod folder or a .dll");
    };
    let root = std::env::temp_dir().join("valhsync-install-one");
    let plugins = root.join("BepInEx").join("plugins");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&plugins)?;

    let done = valhsync_server::install::install(&root, std::path::Path::new(&arg))?;
    println!(
        "{} {} ({} file(s))",
        if done.replaced {
            "replaced"
        } else {
            "installed"
        },
        done.name,
        done.files
    );
    for entry in walk(&plugins.join(&done.name))? {
        println!("  {entry}");
    }
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

fn walk(dir: &std::path::Path) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let shown = path.strip_prefix(dir).unwrap_or(&path);
                out.push(shown.display().to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}
