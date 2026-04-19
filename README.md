# lindirstat-rs

A Windows GUI disk-usage visualiser for **remote Linux hosts**. Connect over SSH, scan a directory tree on the server, and explore the results as an interactive treemap — like WinDirStat or qdirstat, but the filesystem being analysed is remote.

---

## Features

- **Live streaming treemap** — entries render incrementally as the scanner streams them back; no waiting for the full scan to finish
- **Squarified treemap layout** — proportional area representation with click-to-zoom navigation
- **SSH key auth** — uses your existing `~/.ssh/config`, agent, jump hosts, and keys; no extra configuration
- **Password auth** — built-in username/password login via libssh2 for hosts where key auth isn't set up
- **Sudo scanning** — toggle to run the scanner as root for full filesystem visibility
- **Hover tooltips** — full path and human-readable size for any block under the cursor
- **Zero remote dependencies** — the scanner is a statically-linked musl binary; it runs on any Linux kernel ≥ 3.2 with no libc requirement

---

## How it works

```
Windows client  ──SSH──▶  Linux server
   (egui GUI)                (scanner agent)
                               │
                    jwalk parallel walk
                               │
                    wire stream (stdout)
                               │
   treemap ◀── parse ◀── SSH pipe ◀──┘
```

1. The client connects to the remote host over SSH
2. It uploads or verifies a small static scanner binary at `~/.cache/lindirstat/scanner` on the server
3. The scanner walks the requested path using parallel directory traversal and streams entries back over stdout in a compact binary protocol
4. The client parses the stream incrementally, building the tree in memory and re-rendering the treemap every 100ms as data arrives
5. The final `Summary` frame closes the stream with entry count, total size, error count, and elapsed time

---

## Getting started

### Prerequisites

- Rust toolchain (stable, 1.75+)
- A Linux server accessible via SSH

### Build (Windows)

```powershell
cargo build --release
```

The output binary is `target/release/lindirstat.exe`.

### Build (Linux / macOS dev)

```bash
cargo build --release
```

Produces `target/release/lindirstat`. The GUI targets are the same cross-platform — eframe supports Windows, Linux, and macOS.

### Cross-compile Windows release from Linux

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu -p lindirstat
```

Output: `target/x86_64-pc-windows-gnu/release/lindirstat.exe`

---

## Usage

### GUI

Launch `lindirstat.exe`. The toolbar at the top contains all connection options:

| Field | Description |
|-------|-------------|
| **SSH Key** / **Password** | Authentication method |
| **host** | SSH Key mode: `user@hostname` — Password mode: hostname or IP |
| **port** | (Password mode) SSH port, defaults to 22 if blank |
| **user / pass** | (Password mode) Credentials |
| **path** | Absolute path on the remote server to scan |
| **sudo** | Run the scanner under `sudo` for root-level access |

Click **Scan** to start. The treemap fills in as data arrives.

**Navigation:**
- **Hover** — shows full path and size in a tooltip
- **Click** — zooms into the clicked directory
- **⬆ Up** — zooms back out one level

### CLI

A headless CLI tool is also included for scripting or quick checks:

```bash
# Print the top 20 largest directories under /var on a remote host
lindirstat-cli user@host:/var

# Top 50, run as root
lindirstat-cli --top 50 --sudo user@host:/

# Read a pre-captured wire stream from stdin
lindirstat-cli -
```

Output:

```
      SIZE  PATH
   45.2 GB  /var
   38.1 GB  /var/lib
   22.7 GB  /var/lib/docker
    9.4 GB  /var/log
   ...
```

---

## Scanner agent

The scanner is a separate binary that runs on the remote Linux host. It is placed at `~/.cache/lindirstat/scanner` by the bootstrap flow (M3, in progress). For now you can copy it manually:

```bash
# Build for x86_64 Linux (static musl)
cargo build --release --target x86_64-unknown-linux-musl -p lindirstat-scanner

# Copy to the remote host
scp target/x86_64-unknown-linux-musl/release/scanner user@host:~/.cache/lindirstat/scanner
ssh user@host 'chmod +x ~/.cache/lindirstat/scanner'
```

For aarch64 (Raspberry Pi, ARM servers):

```bash
cargo build --release --target aarch64-unknown-linux-musl -p lindirstat-scanner
scp target/aarch64-unknown-linux-musl/release/scanner user@host:~/.cache/lindirstat/scanner
```

The scanner accepts:

```
scanner <path> [--one-filesystem] [--wire-version]
```

| Flag | Description |
|------|-------------|
| `<path>` | Root path to scan |
| `--one-filesystem` | Don't cross mount points (accepted, enforcement in progress) |
| `--wire-version` | Print wire protocol version and build hash, then exit |

Errors (permission denied, unreadable entries) are printed to stderr and counted in the summary; the scan continues.

---

## Wire protocol

A compact binary protocol shared between the scanner and client via the `lindirstat-wire` crate.

**Framing:** each message is a `u32 LE` byte length followed by a `postcard`-serialised payload.

**Message types (in order):**

```
Header   { magic: b"LDS1", version: u32, root: String, started_unix: u64 }
Entry    { id: u32, parent_id: u32, name: String, size: u64, mtime: i64, kind: u8 }
  ...repeated for every filesystem entry...
Summary  { entries: u64, bytes: u64, errors: u64, elapsed_ms: u64 }
```

`kind` values: `0` = file, `1` = directory, `2` = symlink, `3` = other.

`parent_id` references the `id` of the parent directory. The root entry is emitted first with `id = 0` and `parent_id = 0` (self-reference marks the root). All subsequent entries reference a previously-seen parent id, so the client can build the tree in a single forward pass.

---

## Workspace layout

```
lindirstat-rs/
├── Cargo.toml              # workspace
├── crates/
│   ├── wire/               # shared wire protocol (no_std-friendly, used by both sides)
│   ├── scanner/            # Linux agent binary (musl targets)
│   └── lindirstat/         # Windows GUI client + CLI (egui / eframe)
│       └── src/
│           ├── main.rs     # GUI entry point and treemap rendering
│           ├── cli.rs      # CLI entry point
│           ├── scan.rs     # SSH transport (key auth + password auth)
│           ├── model.rs    # in-memory tree built from the wire stream
│           ├── treemap.rs  # squarified treemap layout algorithm
│           └── lib.rs
└── xtask/                  # build helper: cross-compile scanner + stage assets
```

---

## Building the scanner cross-platform

The `xtask` automates cross-compiling the musl scanner binaries:

```bash
cargo xtask cross-build
```

Requires either [`cross`](https://github.com/cross-rs/cross) or [`cargo-zigbuild`](https://github.com/rust-cross/cargo-zigbuild) on your PATH. Produces binaries at `crates/lindirstat/assets/scanner-x86_64` and `crates/lindirstat/assets/scanner-aarch64`, which will be embedded into the client binary in a future release.

---

## Development

```bash
# Run all checks before committing
cargo fmt
cargo clippy -- -D warnings
cargo test
```

The Linux GUI build requires a display server. On headless CI, prefix with `DISPLAY=:99` or use a virtual framebuffer.

---

## Roadmap

| Milestone | Status |
|-----------|--------|
| M0 — scanner binary, wire stream to stdout | Done |
| M1 — CLI client, end-to-end transport validation | Done |
| M2 — egui treemap, live streaming, click-to-zoom | Done |
| M3 — bootstrap: embed scanner, version-check, auto-upload | Planned |
| M4 — exclude globs, error surface in UI, Windows release build | Planned |

**Out of scope for v0.1:** multiple concurrent hosts, caching scan results to disk, hardlink dedup, sparse file accounting, btrfs subvolume awareness, deleting files from the GUI.

---

## License

MIT
