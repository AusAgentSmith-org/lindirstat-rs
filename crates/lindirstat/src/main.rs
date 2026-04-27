use eframe::egui;
use lindirstat::model::Tree;
use lindirstat::scan::{spawn_password, spawn_ssh, Msg, PasswordAuth, ScanHandle};
use lindirstat::treemap::{squarified, Cell};

#[derive(Default, PartialEq, Clone, Copy)]
enum AuthMethod {
    #[default]
    SshKey,
    Password,
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 780.0]),
        ..Default::default()
    };
    eframe::run_native(
        "lindirstat",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

#[derive(Default)]
struct App {
    auth: AuthMethod,
    host: String,
    username: String,
    password: String,
    port: String,
    remote_path: String,
    sudo: bool,
    one_filesystem: bool,

    tree: Tree,
    scan: Option<ScanHandle>,
    status: String,

    zoom: Vec<usize>,
    cells: Vec<Cell>,
    hovered: Option<usize>,
}

impl App {
    fn drain_scan(&mut self) {
        let Some(h) = &self.scan else { return };
        let mut done = false;
        loop {
            match h.rx.try_recv() {
                Ok(Msg::Status(s)) => {
                    self.status = s;
                }
                Ok(Msg::Header { root }) => {
                    self.status = format!("scanning {root}…");
                }
                Ok(Msg::Batch(b)) => {
                    self.tree.extend(b);
                    if self.zoom.is_empty() {
                        if let Some(r) = self.tree.root_idx() {
                            self.zoom.push(r);
                        }
                    }
                }
                Ok(Msg::Done(s)) => {
                    self.status = format!(
                        "done: {} entries, {}, {} errors, {}ms",
                        s.entries,
                        human(s.bytes),
                        s.errors,
                        s.elapsed_ms
                    );
                    done = true;
                }
                Ok(Msg::Error(e)) => {
                    self.status = format!("error: {e}");
                    done = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    done = true;
                    break;
                }
            }
        }
        if done {
            self.scan = None;
        }
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        self.tree = Tree::default();
        self.zoom.clear();
        self.cells.clear();
        self.hovered = None;
        let handle = match self.auth {
            AuthMethod::SshKey => spawn_ssh(
                ctx.clone(),
                &self.host,
                &self.remote_path,
                "~/.cache/lindirstat/scanner",
                self.sudo,
                self.one_filesystem,
            ),
            AuthMethod::Password => {
                let port = self.port.parse::<u16>().unwrap_or(22);
                spawn_password(
                    ctx.clone(),
                    PasswordAuth {
                        host: &self.host,
                        port,
                        username: &self.username,
                        password: &self.password,
                    },
                    &self.remote_path,
                    "~/.cache/lindirstat/scanner",
                    self.sudo,
                    self.one_filesystem,
                )
            }
        };
        self.scan = Some(handle);
        self.status = "connecting…".into();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_scan();
        if self.scan.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.auth, AuthMethod::SshKey, "SSH Key");
                ui.selectable_value(&mut self.auth, AuthMethod::Password, "Password");
                ui.separator();
                match self.auth {
                    AuthMethod::SshKey => {
                        ui.label("host:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.host)
                                .hint_text("user@hostname")
                                .desired_width(180.0),
                        );
                    }
                    AuthMethod::Password => {
                        ui.label("host:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.host)
                                .hint_text("hostname")
                                .desired_width(140.0),
                        );
                        ui.label("port:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.port)
                                .hint_text("22")
                                .desired_width(40.0),
                        );
                        ui.label("user:");
                        ui.add(egui::TextEdit::singleline(&mut self.username).desired_width(90.0));
                        ui.label("pass:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.password)
                                .password(true)
                                .desired_width(90.0),
                        );
                    }
                }
                ui.label("path:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.remote_path)
                        .hint_text("/home")
                        .desired_width(220.0),
                );
                ui.checkbox(&mut self.sudo, "sudo");
                ui.checkbox(&mut self.one_filesystem, "one FS")
                    .on_hover_text("Don't cross mount points (recommended when scanning /)");

                let can_scan = !self.host.is_empty()
                    && !self.remote_path.is_empty()
                    && self.scan.is_none()
                    && (self.auth == AuthMethod::SshKey
                        || (!self.username.is_empty() && !self.password.is_empty()));
                if ui
                    .add_enabled(can_scan, egui::Button::new("Scan"))
                    .clicked()
                {
                    self.start_scan(ctx);
                }
                ui.separator();
                ui.label(&self.status);
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.zoom.len() > 1, egui::Button::new("⬆ Up"))
                    .clicked()
                {
                    self.zoom.pop();
                }
                if let Some(&cur) = self.zoom.last() {
                    ui.label(self.tree.path_of(cur));
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(&cur) = self.zoom.last() else {
                ui.centered_and_justified(|ui| {
                    ui.label("Enter a host and path above, then click Scan");
                });
                return;
            };
            let area = ui.available_rect_before_wrap();
            self.cells.clear();
            const MAX_DEPTH: u16 = 2;
            layout_recursive(&self.tree, cur, area, 0, MAX_DEPTH, &mut self.cells);

            let painter = ui.painter();
            for cell in &self.cells {
                if !cell.is_leaf {
                    continue;
                }
                let base = color_for(self.tree.entries[cell.index].id);
                painter.rect_filled(cell.rect, 0.0, base);
                if cell.rect.width() >= 3.0 && cell.rect.height() >= 3.0 {
                    painter.rect_stroke(
                        cell.rect,
                        0.0,
                        egui::Stroke::new(0.5, egui::Color32::from_black_alpha(110)),
                    );
                }
            }
            for cell in self
                .cells
                .iter()
                .filter(|c| c.is_leaf && self.tree.is_dir(c.index))
            {
                let name = &self.tree.entries[cell.index].name;
                let w = cell.rect.width();
                let h = cell.rect.height();
                if w < 36.0 || h < 12.0 {
                    continue;
                }
                let label = if w >= 90.0 && h >= 16.0 {
                    format!("{}  {}", name, human(self.tree.subtree[cell.index]))
                } else {
                    name.clone()
                };
                let font_size = if h >= 16.0 { 11.0 } else { 9.5 };
                let pos = cell.rect.left_top() + egui::vec2(4.0, 2.0);
                painter.text(
                    pos,
                    egui::Align2::LEFT_TOP,
                    &label,
                    egui::FontId::proportional(font_size),
                    egui::Color32::WHITE,
                );
            }

            let response = ui.allocate_rect(area, egui::Sense::click());
            let ptr = response.interact_pointer_pos().or(response.hover_pos());
            self.hovered = None;
            if let Some(pos) = ptr {
                for cell in self.cells.iter().rev() {
                    if cell.rect.contains(pos) {
                        self.hovered = Some(cell.index);
                        break;
                    }
                }
            }
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    if let Some(target) = self
                        .cells
                        .iter()
                        .find(|c| c.depth == 0 && self.tree.is_dir(c.index) && c.rect.contains(pos))
                    {
                        if !self.tree.children_of(target.index).is_empty() {
                            self.zoom.push(target.index);
                        }
                    }
                }
            }
            if let Some(i) = self.hovered {
                response.on_hover_text(format!(
                    "{}\n{}",
                    self.tree.path_of(i),
                    human(self.tree.subtree[i])
                ));
            }
        });
    }
}

fn layout_recursive(
    tree: &Tree,
    root: usize,
    area: egui::Rect,
    depth: u16,
    max_depth: u16,
    out: &mut Vec<Cell>,
) {
    let kids: Vec<(usize, u64)> = tree
        .children_of(root)
        .iter()
        .map(|&i| (i, tree.subtree[i]))
        .collect();
    if kids.is_empty() {
        return;
    }
    const MIN_RECURSE_SIDE: f32 = 12.0;
    let placed = squarified(kids, area);
    for (idx, rect) in placed {
        let short = rect.width().min(rect.height());
        let is_dir = tree.is_dir(idx);
        let has_kids = !tree.children_of(idx).is_empty();
        let can_recurse = is_dir && has_kids && short >= MIN_RECURSE_SIDE && depth < max_depth;
        if can_recurse {
            out.push(Cell {
                index: idx,
                rect,
                depth,
                is_leaf: false,
            });
            layout_recursive(tree, idx, rect, depth + 1, max_depth, out);
        } else {
            out.push(Cell {
                index: idx,
                rect,
                depth,
                is_leaf: true,
            });
        }
    }
}

fn color_for(id: u32) -> egui::Color32 {
    let mut h = id.wrapping_mul(2654435761);
    h ^= h >> 16;
    let hue = (h % 360) as f32 / 360.0;
    let (r, g, b) = hsv_to_rgb(hue, 0.65, 0.42);
    egui::Color32::from_rgb(r, g, b)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let i = (h * 6.0).floor() as i32;
    let f = h * 6.0 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
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
