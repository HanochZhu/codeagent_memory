//! Shimmer progress for `cam index`, matching codegraph's index UI:
//! a dim rail, an amber spinner, and a bar whose highlight travels across
//! the filled segment. Drawing runs on its own thread so the animation keeps
//! moving while parsing and SQLite block the indexer.

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cam::code::{IndexPhase, IndexProgress};

const RST: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const BOLD: &str = "\x1b[1m";
const BAR_WIDTH: usize = 25;
const FRAME_MS: u64 = 50;
const ANIM_INTERVAL_MS: u64 = 150;
const FRAMES_PER_GLYPH: u64 = 3;

#[derive(Clone, Copy)]
struct Glyphs {
    corner_top: &'static str,
    corner_bottom: &'static str,
    rail: &'static str,
    spinner: &'static [&'static str],
    bar_filled: &'static str,
    bar_empty: &'static str,
    phase_done: &'static str,
    dash: &'static str,
}

const UNICODE: Glyphs = Glyphs {
    corner_top: "┌",
    corner_bottom: "└",
    rail: "│",
    spinner: &["·", "✢", "✳", "✶", "✻", "✽"],
    bar_filled: "█",
    bar_empty: "░",
    phase_done: "◆",
    dash: "—",
};

const ASCII: Glyphs = Glyphs {
    corner_top: "+",
    corner_bottom: "+",
    rail: "|",
    spinner: &[".", "*", "+", "x", "o", "O"],
    bar_filled: "#",
    bar_empty: "-",
    phase_done: "*",
    dash: "-",
};

#[derive(Clone, Copy)]
struct View {
    message: &'static str,
    percent: i32,
    count: usize,
}

impl View {
    fn idle() -> Self {
        Self {
            message: "",
            percent: -1,
            count: 0,
        }
    }
}

enum Msg {
    Update(View),
    Finish,
    Stop,
    Abort,
}

pub struct ShimmerProgress {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
    last_phase: Option<IndexPhase>,
    last_send: Option<Instant>,
    pending: Option<View>,
}

impl ShimmerProgress {
    pub fn start() -> Self {
        let glyphs = glyphs();
        print_intro(glyphs);
        let (tx, rx) = mpsc::channel();
        let thread = thread::spawn(move || worker(rx, glyphs));
        Self {
            tx,
            thread: Some(thread),
            last_phase: None,
            last_send: None,
            pending: None,
        }
    }

    pub fn on_progress(&mut self, event: IndexProgress) {
        let phase_changed = self.last_phase.is_some_and(|prev| prev != event.phase);
        if phase_changed {
            self.flush_pending();
            let _ = self.tx.send(Msg::Finish);
        }
        self.last_phase = Some(event.phase);
        let view = to_view(event);
        let complete = event.total > 0 && event.current == event.total;
        let due = self
            .last_send
            .is_none_or(|sent| sent.elapsed() >= Duration::from_millis(FRAME_MS));
        if phase_changed || complete || due {
            let _ = self.tx.send(Msg::Update(view));
            self.pending = None;
            self.last_send = Some(Instant::now());
        } else {
            self.pending = Some(view);
        }
    }

    /// Mark the current phase done and stop the animation.
    pub fn finish(mut self) {
        self.flush_pending();
        let _ = self.tx.send(Msg::Stop);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    fn flush_pending(&mut self) {
        if let Some(view) = self.pending.take() {
            let _ = self.tx.send(Msg::Update(view));
        }
    }
}

impl Drop for ShimmerProgress {
    fn drop(&mut self) {
        if let Some(handle) = self.thread.take() {
            let _ = self.tx.send(Msg::Abort);
            let _ = handle.join();
        }
    }
}

fn worker(rx: Receiver<Msg>, glyphs: Glyphs) {
    let started = Instant::now();
    let mut view = View::idle();
    loop {
        let frame_at = Instant::now() + Duration::from_millis(FRAME_MS);
        loop {
            let now = Instant::now();
            if now >= frame_at {
                break;
            }
            match rx.recv_timeout(frame_at.saturating_duration_since(now)) {
                Ok(msg) => {
                    if !apply(&glyphs, &mut view, msg) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    finish_phase(&glyphs, &mut view);
                    return;
                }
            }
        }
        while let Ok(msg) = rx.try_recv() {
            if !apply(&glyphs, &mut view, msg) {
                return;
            }
        }
        render(&glyphs, started, &view);
    }
}

fn apply(glyphs: &Glyphs, view: &mut View, msg: Msg) -> bool {
    match msg {
        Msg::Update(next) => {
            *view = next;
            true
        }
        Msg::Finish => {
            finish_phase(glyphs, view);
            true
        }
        Msg::Stop => {
            finish_phase(glyphs, view);
            false
        }
        Msg::Abort => {
            write_tty("\r\x1b[K");
            false
        }
    }
}

fn to_view(event: IndexProgress) -> View {
    let message = phase_name(event.phase);
    if event.total > 0 {
        let percent = ((event.current as f64 / event.total as f64) * 100.0).round() as i32;
        View {
            message,
            percent: percent.clamp(0, 100),
            count: 0,
        }
    } else if event.current > 0 {
        View {
            message,
            percent: -1,
            count: event.current,
        }
    } else {
        View {
            message,
            percent: -1,
            count: 0,
        }
    }
}

fn phase_name(phase: IndexPhase) -> &'static str {
    match phase {
        IndexPhase::Scanning => "Scanning files",
        IndexPhase::Parsing => "Parsing code",
        IndexPhase::Storing => "Storing data",
        IndexPhase::Resolving => "Resolving refs",
    }
}

fn finish_phase(glyphs: &Glyphs, view: &mut View) {
    if view.message.is_empty() {
        return;
    }
    let detail = if view.percent >= 0 {
        format!(" {} done", glyphs.dash)
    } else if view.count > 0 {
        format!(" {} {} found", glyphs.dash, format_number(view.count))
    } else {
        String::new()
    };
    write_tty(&format!(
        "\r\x1b[K{DIM}{rail}{RST}  {GREEN}{mark}{RST} {msg}{detail}\n",
        rail = glyphs.rail,
        mark = glyphs.phase_done,
        msg = view.message,
    ));
    *view = View::idle();
}

fn render(glyphs: &Glyphs, started: Instant, view: &View) {
    if view.message.is_empty() {
        return;
    }
    let frame = started.elapsed().as_millis() as u64 / ANIM_INTERVAL_MS;
    write_tty(&format!("\r\x1b[K{}", render_line(glyphs, frame, view)));
}

fn render_line(glyphs: &Glyphs, frame: u64, view: &View) -> String {
    let glyph = glyphs.spinner[(frame / FRAMES_PER_GLYPH) as usize % glyphs.spinner.len()];
    let color = shimmer_color(frame);
    if view.percent >= 0 {
        let filled = ((BAR_WIDTH as f64) * (view.percent as f64) / 100.0).round() as usize;
        let filled = filled.min(BAR_WIDTH);
        let bar = render_bar(glyphs, frame, filled, BAR_WIDTH - filled);
        format!(
            "{DIM}{rail}{RST}  {color}{glyph}{RST} {msg}  {bar}  {pct}%",
            rail = glyphs.rail,
            msg = view.message,
            pct = view.percent,
        )
    } else if view.count > 0 {
        format!(
            "{DIM}{rail}{RST}  {color}{glyph}{RST} {msg}... {} found",
            format_number(view.count),
            rail = glyphs.rail,
            msg = view.message,
        )
    } else {
        format!(
            "{DIM}{rail}{RST}  {color}{glyph}{RST} {msg}...",
            rail = glyphs.rail,
            msg = view.message,
        )
    }
}

fn render_bar(glyphs: &Glyphs, frame: u64, filled: usize, empty: usize) -> String {
    if filled == 0 {
        return format!("{DIM}{}{RST}", glyphs.bar_empty.repeat(empty));
    }
    let shimmer_pos = ((frame % 24) as f64 / 24.0) * (filled as f64 + 6.0) - 3.0;
    let mut bar = String::new();
    for i in 0..filled {
        let dist = (i as f64 - shimmer_pos).abs();
        let t = (1.0 - dist / 3.0).max(0.0);
        let (r, g, b) = amber(t);
        bar.push_str(&format!(
            "\x1b[38;2;{r};{g};{b}m{BOLD}{}",
            glyphs.bar_filled
        ));
    }
    bar.push_str(&format!(
        "{RST}{DIM}{}{RST}",
        glyphs.bar_empty.repeat(empty)
    ));
    bar
}

fn shimmer_color(frame: u64) -> String {
    let t = (((frame as f64) * 2.0 * std::f64::consts::PI / 13.0).sin() + 1.0) / 2.0;
    let (r, g, b) = amber(t);
    format!("\x1b[38;2;{r};{g};{b}m{BOLD}")
}

fn amber(t: f64) -> (i32, i32, i32) {
    (lerp(160, 251, t), lerp(100, 191, t), lerp(9, 36, t))
}

fn lerp(a: i32, b: i32, t: f64) -> i32 {
    (a as f64 + (b - a) as f64 * t).round() as i32
}

fn format_number(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn glyphs() -> Glyphs {
    if supports_unicode() {
        UNICODE
    } else {
        ASCII
    }
}

fn supports_unicode() -> bool {
    if std::env::var("CAM_ASCII").ok().as_deref() == Some("1") {
        return false;
    }
    if std::env::var("CAM_UNICODE").ok().as_deref() == Some("1") {
        return true;
    }
    if cfg!(windows) {
        return false;
    }
    std::env::var("TERM").ok().as_deref() != Some("linux")
}

fn print_intro(glyphs: Glyphs) {
    let mut out = io::stderr().lock();
    let _ = writeln!(
        out,
        "{DIM}{top}{RST}  Indexing project",
        top = glyphs.corner_top
    );
    let _ = writeln!(out, "{DIM}{rail}{RST}", rail = glyphs.rail);
}

pub fn outro() {
    print_outro(glyphs());
}

fn print_outro(glyphs: Glyphs) {
    let mut out = io::stderr().lock();
    let _ = writeln!(out, "{DIM}{rail}{RST}", rail = glyphs.rail);
    let _ = writeln!(
        out,
        "{DIM}{bottom}{RST}  Done",
        bottom = glyphs.corner_bottom
    );
}

fn write_tty(text: &str) {
    let mut out = io::stderr().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_line_draws_a_full_bar() {
        let view = View {
            message: "Parsing code",
            percent: 100,
            count: 0,
        };
        let line = render_line(&UNICODE, 0, &view);
        assert!(line.contains("Parsing code"), "{line}");
        assert!(line.contains("100%"), "{line}");
        assert_eq!(line.matches('█').count(), BAR_WIDTH, "{line}");
        assert!(!line.contains('░'), "{line}");
    }

    #[test]
    fn unknown_total_shows_a_running_count() {
        let view = View {
            message: "Scanning files",
            percent: -1,
            count: 1280,
        };
        let line = render_line(&UNICODE, 3, &view);
        assert!(line.contains("Scanning files... 1,280 found"), "{line}");
        assert!(line.contains('✢'), "{line}");
    }

    #[test]
    fn zero_percent_is_an_empty_bar() {
        let view = View {
            message: "Storing data",
            percent: 0,
            count: 0,
        };
        let line = render_line(&ASCII, 0, &view);
        assert!(line.contains("Storing data"), "{line}");
        assert!(line.contains(" 0%"), "{line}");
        assert_eq!(line.matches('-').count(), BAR_WIDTH, "{line}");
        assert!(!line.contains('#'), "{line}");
    }
}
