//! CLI client: spawn `ssh host scanner <path>` (or read from stdin),
//! parse the wire stream, print the top-N biggest directories.

use anyhow::{Context, Result};
use clap::Parser;
use lindirstat::model::Tree;
use lindirstat_wire::{read_frame, Frame, KIND_DIR};
use std::io::{self, BufReader, Read};
use std::process::{Command, Stdio};

#[derive(Parser, Debug)]
#[command(name = "lindirstat-cli", version)]
struct Args {
    /// Target as `host:/path`. If `-`, read a wire stream from stdin instead.
    target: String,

    /// Remote path to the scanner binary.
    #[arg(long, default_value = "~/.cache/lindirstat/scanner")]
    scanner_path: String,

    /// Top N directories to print.
    #[arg(long, default_value_t = 20)]
    top: usize,

    /// Run the scanner under sudo on the remote.
    #[arg(long)]
    sudo: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.target == "-" {
        let stdin = io::stdin().lock();
        return run(BufReader::new(stdin), args.top);
    }

    let (host, path) = args
        .target
        .split_once(':')
        .context("target must be host:/path or -")?;

    let remote_cmd = if args.sudo {
        format!(
            "sudo {} {}",
            shell_escape(&args.scanner_path),
            shell_escape(path)
        )
    } else {
        format!(
            "{} {}",
            shell_escape(&args.scanner_path),
            shell_escape(path)
        )
    };

    let mut child = Command::new("ssh")
        .arg(host)
        .arg(remote_cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("spawn ssh")?;

    let stdout = child.stdout.take().unwrap();
    run(BufReader::new(stdout), args.top)?;
    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("ssh exited with {status}");
    }
    Ok(())
}

fn run<R: Read>(mut r: R, top: usize) -> Result<()> {
    let mut tree = Tree::default();
    let mut header_seen = false;
    loop {
        let Some(frame) = read_frame(&mut r)? else {
            break;
        };
        match frame {
            Frame::Header(h) => {
                eprintln!("scanning {} (wire v{})", h.root, h.version);
                header_seen = true;
            }
            Frame::Entry(e) => tree.push(e),
            Frame::Summary(s) => {
                eprintln!(
                    "done: {} entries, {}, {} errors, {}ms",
                    s.entries,
                    human(s.bytes),
                    s.errors,
                    s.elapsed_ms
                );
                break;
            }
        }
    }
    anyhow::ensure!(header_seen, "no header received");

    let mut dirs: Vec<(usize, u64)> = tree
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == KIND_DIR)
        .map(|(i, _)| (i, tree.subtree[i]))
        .collect();
    dirs.sort_unstable_by(|a, b| b.1.cmp(&a.1));

    println!("{:>10}  PATH", "SIZE");
    for (i, size) in dirs.into_iter().take(top) {
        println!("{:>10}  {}", human(size), tree.path_of(i));
    }
    Ok(())
}

fn human(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/_.-~=:".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}
