//! Build helpers. `cargo xtask cross-build` produces musl scanner binaries
//! for x86_64 and aarch64 and copies them into the client's assets dir so
//! `include_bytes!` picks them up.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(name = "xtask")]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Cross-build scanner for musl targets and stage into client assets.
    CrossBuild,
}

const TARGETS: &[&str] = &["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];

fn main() -> Result<()> {
    match Args::parse().cmd {
        Cmd::CrossBuild => cross_build(),
    }
}

fn cross_build() -> Result<()> {
    let workspace = workspace_root()?;
    let assets = workspace.join("crates/lindirstat/assets");
    std::fs::create_dir_all(&assets)?;

    let tool = pick_cross_tool()?;
    for target in TARGETS {
        eprintln!("building scanner for {target} via {tool}");
        let status = Command::new(tool)
            .args([
                "build",
                "--release",
                "-p",
                "lindirstat-scanner",
                "--target",
                target,
            ])
            .current_dir(&workspace)
            .status()
            .with_context(|| format!("spawn {tool}"))?;
        if !status.success() {
            bail!("{tool} build failed for {target}");
        }
        let src = workspace.join(format!("target/{target}/release/scanner"));
        let dst = assets.join(format!("scanner-{}", arch_from_target(target)));
        std::fs::copy(&src, &dst)
            .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
        eprintln!("  -> {}", dst.display());
    }
    Ok(())
}

fn arch_from_target(t: &str) -> &str {
    t.split('-').next().unwrap_or(t)
}

fn pick_cross_tool() -> Result<&'static str> {
    if which("cross") {
        Ok("cross")
    } else if which("cargo-zigbuild") {
        Ok("cargo-zigbuild")
    } else {
        bail!("need either `cross` or `cargo-zigbuild` installed for cross-compilation")
    }
}

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn workspace_root() -> Result<PathBuf> {
    let out = Command::new(env!("CARGO"))
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()
        .context("cargo locate-project")?;
    let s = String::from_utf8(out.stdout).context("non-utf8 cargo output")?;
    let toml = PathBuf::from(s.trim());
    Ok(toml.parent().context("no parent")?.to_path_buf())
}
