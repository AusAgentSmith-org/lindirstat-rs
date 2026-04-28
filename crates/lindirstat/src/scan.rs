use anyhow::{Context, Result};
use lindirstat_wire::{read_frame, Entry, Frame, Summary};
use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

pub enum Msg {
    Status(String),
    Log(String),
    Header { root: String },
    Batch(Vec<Entry>),
    Done(Summary),
    Error(String),
}

pub struct ScanHandle {
    pub rx: mpsc::Receiver<Msg>,
}

const BATCH_SIZE: usize = 512;
const LOCAL_BUILD: &str = env!("CARGO_PKG_VERSION");

#[cfg(embed_scanner)]
static SCANNER_X86_64: &[u8] = include_bytes!("../assets/scanner-x86_64");
#[cfg(embed_scanner)]
static SCANNER_AARCH64: &[u8] = include_bytes!("../assets/scanner-aarch64");

pub struct PasswordAuth<'a> {
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub password: &'a str,
}

pub fn spawn_ssh(
    ctx: egui::Context,
    host: &str,
    remote_path: &str,
    scanner_path: &str,
    sudo: bool,
    one_filesystem: bool,
) -> ScanHandle {
    let host = host.to_owned();
    let remote_path = remote_path.to_owned();
    let scanner_path = scanner_path.to_owned();

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Err(e) = ssh_key_thread(
            &ctx,
            &tx,
            &host,
            &remote_path,
            &scanner_path,
            sudo,
            one_filesystem,
        ) {
            let _ = tx.send(Msg::Error(e.to_string()));
            ctx.request_repaint();
        }
    });
    ScanHandle { rx }
}

fn ssh_key_thread(
    ctx: &egui::Context,
    tx: &mpsc::Sender<Msg>,
    host: &str,
    remote_path: &str,
    scanner_path: &str,
    sudo: bool,
    one_filesystem: bool,
) -> Result<()> {
    send_status(ctx, tx, "connecting…");
    maybe_upload_ssh_key(ctx, tx, host, scanner_path)?;

    let cmd = build_cmd(scanner_path, remote_path, sudo, one_filesystem);
    send_status(ctx, tx, "starting scan…");
    send_log(ctx, tx, &format!("exec: ssh {host} {cmd}"));
    let mut child = Command::new("ssh")
        .arg(host)
        .arg(&cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("spawn ssh")?;
    send_log(ctx, tx, "ssh spawned — reading frames…");
    let stdout = child.stdout.take().unwrap();
    pump_frames(ctx, tx, stdout);
    Ok(())
}

fn maybe_upload_ssh_key(
    ctx: &egui::Context,
    tx: &mpsc::Sender<Msg>,
    host: &str,
    scanner_path: &str,
) -> Result<()> {
    send_status(ctx, tx, "checking remote scanner…");

    let check_cmd = format!(
        "uname -m; {0} --wire-version 2>/dev/null || echo 'build=missing'",
        shell_escape(scanner_path),
    );
    let out = Command::new("ssh")
        .arg(host)
        .arg(&check_cmd)
        .output()
        .context("ssh version check")?;
    let output = String::from_utf8_lossy(&out.stdout);

    let mut arch_found = "?";
    let mut build_found = "?";
    for line in output.lines() {
        let line = line.trim();
        if line == "x86_64" || line == "aarch64" {
            arch_found = line;
        }
        if let Some(v) = line.split_whitespace().find_map(|t| t.strip_prefix("build=")) {
            build_found = v;
        }
    }
    send_log(
        ctx,
        tx,
        &format!("remote: arch={arch_found} build={build_found} local={LOCAL_BUILD}"),
    );

    let Some(bytes) = pick_bytes_if_needed(&output)? else {
        send_log(ctx, tx, "scanner up to date, skipping upload");
        return Ok(());
    };

    send_status(ctx, tx, "uploading scanner…");
    send_log(ctx, tx, &format!("uploading {} bytes…", bytes.len()));
    let upload_cmd = format!(
        "mkdir -p $(dirname {0}) && cat > {0} && chmod +x {0}",
        shell_escape(scanner_path),
    );
    let mut child = Command::new("ssh")
        .arg(host)
        .arg(&upload_cmd)
        .stdin(Stdio::piped())
        .spawn()
        .context("spawn ssh for upload")?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(bytes)
        .context("write scanner bytes")?;
    let status = child.wait().context("wait for upload")?;
    anyhow::ensure!(
        status.success(),
        "scanner upload failed (ssh exited {status})"
    );

    send_status(ctx, tx, "scanner uploaded");
    Ok(())
}

pub fn spawn_password(
    ctx: egui::Context,
    auth: PasswordAuth<'_>,
    remote_path: &str,
    scanner_path: &str,
    sudo: bool,
    one_filesystem: bool,
) -> ScanHandle {
    let host = auth.host.to_owned();
    let port = auth.port;
    let username = auth.username.to_owned();
    let password = auth.password.to_owned();
    let remote_path = remote_path.to_owned();
    let scanner_path = scanner_path.to_owned();

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Err(e) = password_thread(
            &ctx,
            &tx,
            &host,
            port,
            &username,
            &password,
            &remote_path,
            &scanner_path,
            sudo,
            one_filesystem,
        ) {
            let _ = tx.send(Msg::Error(e.to_string()));
            ctx.request_repaint();
        }
    });
    ScanHandle { rx }
}

#[allow(clippy::too_many_arguments)]
fn password_thread(
    ctx: &egui::Context,
    tx: &mpsc::Sender<Msg>,
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    remote_path: &str,
    scanner_path: &str,
    sudo: bool,
    one_filesystem: bool,
) -> Result<()> {
    send_status(ctx, tx, "connecting…");

    let tcp = std::net::TcpStream::connect((host, port)).context("tcp connect")?;
    send_log(ctx, tx, &format!("tcp connected to {host}:{port}"));
    let mut sess = ssh2::Session::new().context("create session")?;
    sess.set_tcp_stream(tcp);
    sess.handshake().context("ssh handshake")?;
    send_log(ctx, tx, "ssh handshake ok");
    sess.userauth_password(username, password)
        .context("password auth")?;
    send_log(ctx, tx, &format!("authenticated as {username}"));

    maybe_upload_password(ctx, tx, &sess, scanner_path)?;

    send_status(ctx, tx, "starting scan…");
    let cmd = build_cmd(scanner_path, remote_path, sudo, one_filesystem);
    send_log(ctx, tx, &format!("exec: {cmd}"));
    let mut channel = sess.channel_session().context("open channel")?;
    channel.exec(&cmd).context("exec")?;
    send_log(ctx, tx, "exec ok — reading frames…");
    pump_frames(ctx, tx, channel);
    Ok(())
}

fn maybe_upload_password(
    ctx: &egui::Context,
    tx: &mpsc::Sender<Msg>,
    sess: &ssh2::Session,
    scanner_path: &str,
) -> Result<()> {
    send_status(ctx, tx, "checking remote scanner…");

    let check_cmd = format!(
        "uname -m; {0} --wire-version 2>/dev/null || echo 'build=missing'",
        shell_escape(scanner_path),
    );
    let mut ch = sess.channel_session().context("version check channel")?;
    ch.exec(&check_cmd).context("version check exec")?;
    let mut output = String::new();
    ch.read_to_string(&mut output)
        .context("version check read")?;
    ch.wait_close().ok();

    let mut arch_found = "?";
    let mut build_found = "?";
    for line in output.lines() {
        let line = line.trim();
        if line == "x86_64" || line == "aarch64" {
            arch_found = line;
        }
        if let Some(v) = line.split_whitespace().find_map(|t| t.strip_prefix("build=")) {
            build_found = v;
        }
    }
    send_log(
        ctx,
        tx,
        &format!("remote: arch={arch_found} build={build_found} local={LOCAL_BUILD}"),
    );

    let Some(bytes) = pick_bytes_if_needed(&output)? else {
        send_log(ctx, tx, "scanner up to date, skipping upload");
        return Ok(());
    };

    send_status(ctx, tx, "uploading scanner…");
    send_log(ctx, tx, &format!("uploading {} bytes…", bytes.len()));
    let remote = std::path::Path::new(scanner_path);
    if let Some(parent) = remote.parent() {
        let mkdir = format!("mkdir -p {}", shell_escape(parent.to_str().unwrap_or("")));
        let mut ch = sess.channel_session().context("mkdir channel")?;
        ch.exec(&mkdir).ok();
        ch.wait_close().ok();
    }
    let mut remote_file = sess
        .scp_send(remote, 0o755, bytes.len() as u64, None)
        .context("scp_send")?;
    remote_file.write_all(bytes).context("scp write")?;
    remote_file.send_eof().context("scp eof")?;
    remote_file.wait_eof().context("scp wait eof")?;
    remote_file.close().context("scp close")?;
    remote_file.wait_close().context("scp wait close")?;

    send_status(ctx, tx, "scanner uploaded");
    Ok(())
}

/// Returns `Some(bytes)` when upload is needed (version mismatch or missing).
/// Returns `None` when the remote already matches `LOCAL_BUILD`.
fn pick_bytes_if_needed(output: &str) -> Result<Option<&'static [u8]>> {
    let mut remote_build = "";
    #[cfg(embed_scanner)]
    let mut arch = "";

    for line in output.lines() {
        let line = line.trim();
        #[cfg(embed_scanner)]
        if line == "x86_64" || line == "aarch64" {
            arch = line;
        }
        if let Some(v) = line.split_whitespace().find_map(|t| t.strip_prefix("build=")) {
            remote_build = v;
        }
    }

    if remote_build == LOCAL_BUILD {
        return Ok(None);
    }

    #[cfg(not(embed_scanner))]
    {
        Ok(None)
    }

    #[cfg(embed_scanner)]
    {
        let bytes: &'static [u8] = match arch {
            "x86_64" => SCANNER_X86_64,
            "aarch64" => SCANNER_AARCH64,
            _ => anyhow::bail!("unsupported remote arch {arch:?} (expected x86_64 or aarch64)"),
        };
        Ok(Some(bytes))
    }
}

fn send_status(ctx: &egui::Context, tx: &mpsc::Sender<Msg>, s: &str) {
    let _ = tx.send(Msg::Status(s.to_owned()));
    ctx.request_repaint();
}

fn send_log(ctx: &egui::Context, tx: &mpsc::Sender<Msg>, s: &str) {
    let _ = tx.send(Msg::Log(s.to_owned()));
    ctx.request_repaint();
}

fn pump_frames(ctx: &egui::Context, tx: &mpsc::Sender<Msg>, reader: impl Read) {
    let mut r = BufReader::new(reader);
    let mut batch: Vec<Entry> = Vec::with_capacity(BATCH_SIZE);
    loop {
        match read_frame(&mut r) {
            Ok(Some(Frame::Header(h))) => {
                let _ = tx.send(Msg::Header { root: h.root });
                ctx.request_repaint();
            }
            Ok(Some(Frame::Entry(e))) => {
                batch.push(e);
                if batch.len() >= BATCH_SIZE {
                    let _ = tx.send(Msg::Batch(std::mem::take(&mut batch)));
                    batch = Vec::with_capacity(BATCH_SIZE);
                    ctx.request_repaint();
                }
            }
            Ok(Some(Frame::Summary(s))) => {
                if !batch.is_empty() {
                    let _ = tx.send(Msg::Batch(std::mem::take(&mut batch)));
                }
                let _ = tx.send(Msg::Done(s));
                ctx.request_repaint();
                break;
            }
            Ok(None) => break,
            Err(e) => {
                let _ = tx.send(Msg::Error(e.to_string()));
                ctx.request_repaint();
                break;
            }
        }
    }
}

fn build_cmd(scanner_path: &str, remote_path: &str, sudo: bool, one_filesystem: bool) -> String {
    let one_fs_flag = if one_filesystem {
        " --one-filesystem"
    } else {
        ""
    };
    if sudo {
        format!(
            "sudo {} {}{}",
            shell_escape(scanner_path),
            shell_escape(remote_path),
            one_fs_flag
        )
    } else {
        format!(
            "{} {}{}",
            shell_escape(scanner_path),
            shell_escape(remote_path),
            one_fs_flag
        )
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
