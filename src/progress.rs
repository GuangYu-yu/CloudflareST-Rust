use std::{
    fmt::Write as FmtWrite,
    io::{self, stdout, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const PROGRESS_BAR_BRIGHTNESS: [f64; 2] = [0.5, 0.3];
const PROGRESS_BAR_SPEED: f64 = 0.2;
const WAVE_WIDTH: f64 = 16.0;
const SPEED_FACTOR: f64 = 0.3;
const SATURATION_BASE: f64 = 0.6;
const REFRESH_INTERVAL_MS: u64 = 40;

struct TextData {
    pos: usize,
    msg: String,
    prefix: String,
}

struct BarInner {
    text: Mutex<Arc<TextData>>,
    is_done: AtomicBool,
    total: usize,
    start_str: String,
    end_str: String,
}

pub(crate) struct Bar {
    inner: Arc<BarInner>,
    handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl BarInner {
    fn run_render_loop(&self) {
        let mut stdout_handle = stdout().lock();
        let start_instant = Instant::now();
        let mut bar_str = String::new();
        let mut output_buffer = String::new();

        loop {
            let term_width = get_terminal_width();
            let reserved_space = 20 + self.start_str.len() + self.end_str.len() + 10;
            let bar_length = term_width.saturating_sub(reserved_space);

            bar_str.reserve(bar_length * 10);
            output_buffer.reserve(256);

            self.render_once(&mut stdout_handle, &start_instant, &mut bar_str, &mut output_buffer, bar_length);

            if self.is_done.load(Ordering::Acquire) {
                let _ = writeln!(stdout_handle);
                break;
            }

            thread::sleep(Duration::from_millis(REFRESH_INTERVAL_MS));
        }
    }

    fn render_once(
        &self,
        stdout_handle: &mut io::StdoutLock<'_>,
        start_instant: &Instant,
        bar_str: &mut String,
        output_buffer: &mut String,
        bar_length: usize,
    ) {
        let text_snapshot = {
            let guard = self.text.lock().unwrap();
            Arc::clone(&*guard)
        };

        let current_pos = text_snapshot.pos;

        bar_str.clear();
        output_buffer.clear();

        const UNFILLED_BG: (u8, u8, u8) = (70, 70, 70);

        let progress = (current_pos.min(self.total) as f64) / self.total.max(1) as f64;
        let filled = (progress * bar_length as f64) as usize;
        let phase = (start_instant.elapsed().as_secs_f64() * PROGRESS_BAR_SPEED) % 1.0;

        let mut percent_buf = [b' '; 10];
        let mut cursor = io::Cursor::new(&mut percent_buf[..]);
        let _ = write!(cursor, " {:>4.1}% ", progress * 100.0);
        let percent_len = cursor.position() as usize;

        let start_index = (bar_length / 2).saturating_sub(percent_len / 2)
            .min(bar_length.saturating_sub(percent_len));
        let end_index = start_index + percent_len;

        for i in 0..bar_length {
            let is_filled = i < filled;
            let hue = (1.0 - i as f64 / bar_length as f64 + phase) % 1.0;

            let t = (start_instant.elapsed().as_secs_f64() * SPEED_FACTOR).fract();
            let bar_length_f64 = bar_length as f64;

            let mu = t * bar_length_f64;
            let i_f64 = i as f64;

            let distance_raw = (i_f64 - mu).abs();
            let distance_wrap = bar_length_f64 - distance_raw;
            let distance = distance_raw.min(distance_wrap);

            let distance_ratio = distance / WAVE_WIDTH;
            let attenuation = (-distance_ratio * distance_ratio).exp();
            let brightness = PROGRESS_BAR_BRIGHTNESS[0] + PROGRESS_BAR_BRIGHTNESS[1] * attenuation;
            let saturation = SATURATION_BASE * (0.6 + 0.4 * attenuation);
            let (r, g, b) = hsv_to_rgb(hue, saturation, brightness);

            let (bg_r, bg_g, bg_b) = if is_filled {
                (r, g, b)
            } else {
                UNFILLED_BG
            };

            if i >= start_index && i < end_index {
                let c = percent_buf[i - start_index] as char;
                let _ = write!(
                    bar_str,
                    "\x1b[48;2;{};{};{}m\x1b[1;97m{}\x1b[0m",
                    bg_r, bg_g, bg_b, c
                );
            } else {
                let _ = write!(
                    bar_str,
                    "\x1b[48;2;{};{};{}m \x1b[0m",
                    bg_r, bg_g, bg_b
                );
            }
        }

        let _ = write!(
            output_buffer,
            "\r\x1b[K\x1b[33m{}\x1b[0m {} {} \x1b[32m{}\x1b[0m {}",
            text_snapshot.msg, bar_str, self.start_str, text_snapshot.prefix, self.end_str
        );

        if let Err(e) = stdout_handle.write_all(output_buffer.as_bytes())
            && e.kind() == io::ErrorKind::BrokenPipe {
            return;
        }

        let _ = stdout_handle.flush();
    }
}

impl Bar {
    pub(crate) fn new(count: usize, start_str: &str, end_str: &str) -> Self {
        let inner = Arc::new(BarInner {
            text: Mutex::new(Arc::new(TextData {
                pos: 0,
                msg: String::new(),
                prefix: String::new(),
            })),
            is_done: AtomicBool::new(false),
            total: count.max(1),
            start_str: start_str.to_string(),
            end_str: end_str.to_string(),
        });

        let inner_clone = Arc::clone(&inner);
        let handle = thread::spawn(move || {
            inner_clone.run_render_loop();
        });

        Self {
            inner,
            handle: Mutex::new(Some(handle)),
        }
    }

    pub(crate) fn update(&self, pos: usize, msg: impl Into<String>, suffix: impl Into<String>) {
        if self.inner.is_done.load(Ordering::Relaxed) { return; }

        let new_data = Arc::new(TextData {
            pos,
            msg: msg.into(),
            prefix: suffix.into(),
        });

        if let Ok(mut guard) = self.inner.text.lock() {
            *guard = new_data;
        }
    }

    pub(crate) fn set_suffix(&self, suffix: impl Into<String>) {
        if self.inner.is_done.load(Ordering::Relaxed) { return; }

        if let Ok(mut guard) = self.inner.text.lock() {
            let current = &**guard;
            *guard = Arc::new(TextData {
                pos: current.pos,
                msg: current.msg.clone(),
                prefix: suffix.into(),
            });
        }
    }

    pub(crate) fn done(&self) {
        if self.inner.is_done.swap(true, Ordering::SeqCst) { return; }

        if let Ok(mut guard) = self.handle.lock()
            && let Some(h) = guard.take() {
            let _ = h.join();
        }

        let _ = stdout().flush();
    }
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let i = (h * 6.0).floor() as i32;
    let f = h * 6.0 - i as f64;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

#[cfg(target_os = "windows")]
fn get_terminal_width() -> usize {
    use windows_sys::Win32::System::Console::{GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE, CONSOLE_SCREEN_BUFFER_INFO};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle == 0 as _ || handle == INVALID_HANDLE_VALUE {
            return 80;
        }
        let mut csbi: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(handle, &mut csbi) != 0 {
            let w = csbi.srWindow.Right - csbi.srWindow.Left + 1;
            return w as usize;
        }
    }
    80
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn get_terminal_width() -> usize {
    use libc::{ioctl, winsize, TIOCGWINSZ, STDOUT_FILENO};
    unsafe {
        let mut ws: winsize = std::mem::zeroed();
        if ioctl(STDOUT_FILENO, TIOCGWINSZ, &mut ws) == 0 {
            return ws.ws_col as usize;
        }
    }
    80
}
