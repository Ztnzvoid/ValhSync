//! Take a server's offer of a newer launcher, all the way to the file.
//!
//!     cargo run -p valhsync --example selfupdate-probe -- <server-url>
//!
//! The whole channel against a real server: fetch the offer, check its
//! signature with the key that server publishes, decide whether it is newer
//! than this build, download it, verify its hash, and install it. Everything
//! the window does when somebody presses the button, minus the window -- so
//! that "the update works" can be something observed rather than something
//! asserted.

fn main() -> anyhow::Result<()> {
    let url = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("give me a server URL"))?;

    let client = valhsync::http::Client::new()?;
    let key = client.fetch_key(&url)?;
    println!("server key         {}", key.fingerprint());

    let offer = client
        .fetch_update_offer(&url, &key)?
        .ok_or_else(|| anyhow::anyhow!("that server offers no launcher"))?;
    let running = valhsync::selfupdate::current_version();
    println!("offered            {} for {}", offer.version, offer.target);
    println!("running            {running}");
    println!("newer than us?     {}", offer.is_newer_than(running));
    println!(
        "runs on this box?  {}",
        offer.runs_on(valhsync::selfupdate::current_target())
    );
    println!("size               {} bytes", offer.size);

    if !offer.is_newer_than(running) {
        println!("\nnothing to install: the offer is not newer than this build");
        return Ok(());
    }

    let staged = valhsync::selfupdate::staging_path()?;
    let mut last = 0u64;
    client.download_update(&url, &offer, &staged, &mut |done| {
        if done - last > 4_000_000 {
            last = done;
            println!("  {done} / {} bytes", offer.size);
        }
    })?;
    println!("downloaded and verified against the signed hash");

    let installed = valhsync::selfupdate::install(&staged)?;
    println!("installed          {}", installed.display());
    Ok(())
}
