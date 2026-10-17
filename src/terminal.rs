use anyhow::Result;
use eframe::egui::{self, Color32, Key, Vec2};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use std::{
    io::{Read, Write},
    path::Path,
    sync::mpsc::{self, Receiver},
};

pub struct Terminal {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    receiver: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    pub focused: bool,
    size: (u16, u16),
    error: Option<String>,
}
impl Terminal {
    pub fn new(path: &Path, ctx: &egui::Context) -> Result<Self> {
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows: 12,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let shell = if cfg!(windows) {
            std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into())
        } else {
            std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into())
        };
        let mut command = CommandBuilder::new(shell);
        command.cwd(path);
        command.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (sender, receiver) = mpsc::sync_channel(64);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if sender.send(buffer[..n].to_vec()).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                }
            }
        });
        Ok(Self {
            master: pair.master,
            child,
            writer,
            receiver,
            parser: vt100::Parser::new(12, 100, 1000),
            focused: false,
            size: (12, 100),
            error: None,
        })
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.poll();
        if let Some(error) = &self.error {
            ui.colored_label(Color32::RED, error);
        }
        let font = egui::FontId::monospace(13.0);
        let cell = Vec2::new(7.8, 16.0);
        let rows = (ui.available_height() / cell.y).clamp(2.0, 100.0) as u16;
        let columns = (ui.available_width() / cell.x).clamp(10.0, 300.0) as u16;
        if self.size != (rows, columns) {
            self.size = (rows, columns);
            self.parser.set_size(rows, columns);
            if let Err(error) = self.master.resize(PtySize {
                rows,
                cols: columns,
                pixel_width: 0,
                pixel_height: 0,
            }) {
                self.error = Some(error.to_string());
            }
        }
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(columns as f32 * cell.x, rows as f32 * cell.y),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                true,
                self.parser.screen().contents(),
            )
        });
        if response.clicked() {
            response.request_focus();
            self.focused = true;
        }
        if ui.input(|i| i.pointer.any_click()) && !response.hovered() {
            self.focused = false;
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, Color32::from_rgb(23, 26, 32));
        for row in 0..rows {
            for col in 0..columns {
                if let Some(item) = self.parser.screen().cell(row, col) {
                    let position = rect.min + Vec2::new(col as f32 * cell.x, row as f32 * cell.y);
                    let bg = color(item.bgcolor(), Color32::from_rgb(23, 26, 32));
                    painter.rect_filled(egui::Rect::from_min_size(position, cell), 0.0, bg);
                    painter.text(
                        position,
                        egui::Align2::LEFT_TOP,
                        item.contents(),
                        font.clone(),
                        color(item.fgcolor(), Color32::from_rgb(225, 229, 235)),
                    );
                }
            }
        }
        if self.focused {
            let (row, col) = self.parser.screen().cursor_position();
            painter.rect_stroke(
                egui::Rect::from_min_size(
                    rect.min + Vec2::new(col as f32 * cell.x, row as f32 * cell.y),
                    cell,
                ),
                0.0,
                egui::Stroke::new(1.0_f32, Color32::WHITE),
                egui::StrokeKind::Inside,
            );
            let events = ui.input_mut(|input| std::mem::take(&mut input.events));
            for event in events {
                let bytes = match event {
                    egui::Event::Text(text) => Some(text.into_bytes()),
                    egui::Event::Paste(text) => Some(text.into_bytes()),
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if modifiers.ctrl && key.name().len() == 1 {
                            let byte = key.name().as_bytes()[0].to_ascii_uppercase();
                            Some(vec![byte.saturating_sub(b'@')])
                        } else {
                            match key {
                                Key::Enter => Some(b"\r".to_vec()),
                                Key::Backspace => Some(vec![127]),
                                Key::Tab => Some(b"\t".to_vec()),
                                Key::Escape => Some(vec![27]),
                                Key::ArrowUp => Some(b"\x1b[A".to_vec()),
                                Key::ArrowDown => Some(b"\x1b[B".to_vec()),
                                Key::ArrowRight => Some(b"\x1b[C".to_vec()),
                                Key::ArrowLeft => Some(b"\x1b[D".to_vec()),
                                Key::Home => Some(b"\x1b[H".to_vec()),
                                Key::End => Some(b"\x1b[F".to_vec()),
                                Key::Delete => Some(b"\x1b[3~".to_vec()),
                                _ => None,
                            }
                        }
                    }
                    _ => None,
                };
                if let Some(bytes) = bytes
                    && let Err(error) = self.writer.write_all(&bytes)
                {
                    self.error = Some(error.to_string());
                }
            }
        }
        if let Ok(Some(status)) = self.child.try_wait() {
            ui.label(format!("Shell exited: {status}"));
        }
    }

    pub fn poll(&mut self) {
        for _ in 0..64 {
            match self.receiver.try_recv() {
                Ok(bytes) => self.parser.process(&bytes),
                Err(_) => break,
            }
        }
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn color(color: vt100::Color, default: Color32) -> Color32 {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        vt100::Color::Idx(index) => {
            const ANSI: [[u8; 3]; 16] = [
                [0, 0, 0],
                [205, 49, 49],
                [13, 188, 121],
                [229, 229, 16],
                [36, 114, 200],
                [188, 63, 188],
                [17, 168, 205],
                [229, 229, 229],
                [102, 102, 102],
                [241, 76, 76],
                [35, 209, 139],
                [245, 245, 67],
                [59, 142, 234],
                [214, 112, 214],
                [41, 184, 219],
                [255, 255, 255],
            ];
            if index < 16 {
                let [r, g, b] = ANSI[index as usize];
                Color32::from_rgb(r, g, b)
            } else if index >= 232 {
                let c = 8 + 10 * (index - 232);
                Color32::from_gray(c)
            } else {
                let n = index - 16;
                let cube = |c| if c == 0 { 0 } else { 55 + 40 * c };
                Color32::from_rgb(cube(n / 36), cube((n / 6) % 6), cube(n % 6))
            }
        }
    }
}
