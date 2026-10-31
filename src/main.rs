mod ui;
mod view;

use std::io::{self, ErrorKind};
use std::ops::Deref;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Pid, Process, ProcessRefreshKind, ProcessStatus,
    ProcessesToUpdate, RefreshKind, Signal, System, UpdateKind, Users,
};

use view::{Proc, SortKey, ViewRow};

const TICK: Duration = Duration::from_secs(1);

struct Snap {
    procs: Vec<Proc>,
    cores: Vec<f32>,
    cpu: f32,
    mem_used: u64,
    mem_total: u64,
    swap_used: u64,
    swap_total: u64,
    load: [f64; 3],
    uptime: u64,
    running: usize,
}

impl Snap {
    fn mem_pct(&self) -> f32 {
        if self.mem_total == 0 {
            0.0
        } else {
            (self.mem_used as f64 / self.mem_total as f64 * 100.0) as f32
        }
    }
}

enum Mode {
    Normal,
    Filter,
    Kill { pid: u32, name: String },
    Help,
}

struct App {
    sys: System,
    users: Option<Users>,
    snap: Snap,
    mode: Mode,
    filter: String,
    sort: SortKey,
    desc: bool,
    tree: bool,
    paused: bool,
    selected: usize,
    selected_pid: Option<u32>,
    offset: usize,
    visible: usize,
    status: Option<(Instant, String)>,
    quit: bool,
    last_sample: Instant,
}

impl App {
    fn boot() -> io::Result<Self> {
        let kind = proc_kind();
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_usage())
                .with_memory(MemoryRefreshKind::everything())
                .with_processes(kind),
        )
        .map_err(|err| io::Error::other(err.to_string()))?;
        let users = Users::new_with_refreshed_list().ok();
        // CPU% is a delta. The first sample is always zero.
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let snap = take_snap(&sys, users.as_ref());
        Ok(Self {
            sys,
            users,
            snap,
            mode: Mode::Normal,
            filter: String::new(),
            sort: SortKey::Cpu,
            desc: true,
            tree: false,
            paused: false,
            selected: 0,
            selected_pid: None,
            offset: 0,
            visible: 10,
            status: None,
            quit: false,
            last_sample: Instant::now(),
        })
    }

    fn sample(&mut self) {
        let kind = proc_kind();
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        self.snap = take_snap(&self.sys, self.users.as_ref());
        self.last_sample = Instant::now();
    }

    fn view(&self) -> Vec<ViewRow> {
        view::build_view(
            &self.snap.procs,
            &self.filter,
            self.sort,
            self.desc,
            self.tree,
        )
    }

    fn track_selection(&mut self, rows: &[ViewRow]) {
        if rows.is_empty() {
            self.selected = 0;
            return;
        }
        if let Some(pid) = self.selected_pid
            && let Some(i) = rows
                .iter()
                .position(|row| self.snap.procs[row.proc_index].pid == pid)
        {
            self.selected = i;
            return;
        }
        self.selected = self.selected.min(rows.len() - 1);
        self.selected_pid = Some(self.snap.procs[rows[self.selected].proc_index].pid);
    }

    fn move_to(&mut self, next: usize) {
        let rows = self.view();
        if rows.is_empty() {
            return;
        }
        self.selected = next.min(rows.len() - 1);
        self.selected_pid = Some(self.snap.procs[rows[self.selected].proc_index].pid);
    }

    fn wants_footer(&self) -> bool {
        !self.filter.is_empty()
            || matches!(self.mode, Mode::Filter | Mode::Kill { .. })
            || self.fresh_status().is_some()
    }

    fn fresh_status(&self) -> Option<&str> {
        self.status
            .as_ref()
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(3))
            .map(|(_, text)| text.as_str())
    }

    fn note(&mut self, text: String) {
        self.status = Some((Instant::now(), text));
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.sort == key {
            self.desc = !self.desc;
        } else {
            self.sort = key;
            self.desc = key.default_desc();
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            self.quit = true;
            return;
        }
        match &self.mode {
            Mode::Help => self.on_help_key(key),
            Mode::Filter => self.on_filter_key(key),
            Mode::Kill { .. } => self.on_kill_key(key),
            Mode::Normal => self.on_normal_key(key),
        }
    }

    fn on_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc | KeyCode::Char('?') => self.mode = Mode::Normal,
            _ => {}
        }
    }

    fn on_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Backspace | KeyCode::Delete => {
                self.filter.pop();
            }
            KeyCode::Up => self.move_to(self.selected.saturating_sub(1)),
            KeyCode::Down => self.move_to(self.selected.saturating_add(1)),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(ch);
            }
            _ => {}
        }
    }

    fn on_kill_key(&mut self, key: KeyEvent) {
        let Mode::Kill { pid, name } = &self.mode else {
            return;
        };
        let pid = *pid;
        let name = name.clone();
        match key.code {
            KeyCode::Esc | KeyCode::Char('n' | 'q') => self.mode = Mode::Normal,
            KeyCode::Enter | KeyCode::Char('t' | 'y') => {
                self.send_signal(pid, &name, Signal::Term);
                self.mode = Mode::Normal;
            }
            KeyCode::Char('9') => {
                self.send_signal(pid, &name, Signal::Kill);
                self.mode = Mode::Normal;
            }
            _ => {}
        }
    }

    fn on_normal_key(&mut self, key: KeyEvent) {
        let page = self.visible.max(1);
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Up | KeyCode::Char('k') => self.move_to(self.selected.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.move_to(self.selected.saturating_add(1)),
            KeyCode::PageUp => self.move_to(self.selected.saturating_sub(page)),
            KeyCode::PageDown => self.move_to(self.selected.saturating_add(page)),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_to(self.selected.saturating_sub(page));
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_to(self.selected.saturating_add(page));
            }
            KeyCode::Home | KeyCode::Char('g') => self.move_to(0),
            KeyCode::End | KeyCode::Char('G') => self.move_to(usize::MAX),
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Esc => self.filter.clear(),
            KeyCode::Char('c') => self.set_sort(SortKey::Cpu),
            KeyCode::Char('m') => self.set_sort(SortKey::Mem),
            KeyCode::Char('n') => self.set_sort(SortKey::Name),
            KeyCode::Char('p') => self.set_sort(SortKey::Pid),
            KeyCode::Char('s') | KeyCode::Right => {
                self.sort = self.sort.cycle(true);
                self.desc = self.sort.default_desc();
            }
            KeyCode::Left => {
                self.sort = self.sort.cycle(false);
                self.desc = self.sort.default_desc();
            }
            KeyCode::Char('r') => self.desc = !self.desc,
            KeyCode::Char('t') => self.tree = !self.tree,
            KeyCode::Char('z') => {
                self.paused = !self.paused;
                if !self.paused {
                    self.sample();
                }
            }
            KeyCode::Char('x') => self.start_kill(),
            _ => {}
        }
    }

    fn start_kill(&mut self) {
        let rows = self.view();
        let Some(row) = rows.get(self.selected) else {
            self.note("nothing selected".into());
            return;
        };
        let proc = &self.snap.procs[row.proc_index];
        self.mode = Mode::Kill {
            pid: proc.pid,
            name: proc.name.clone(),
        };
    }

    fn send_signal(&mut self, pid: u32, name: &str, signal: Signal) {
        let label = match signal {
            Signal::Kill => "kill",
            Signal::Term => "term",
            _ => "signal",
        };
        let Some(proc) = self.sys.process(Pid::from_u32(pid)) else {
            self.note(format!("{pid} {name} is already gone"));
            return;
        };
        match proc.kill_with(signal) {
            Some(true) => self.note(format!("{label} sent to {pid} {name}")),
            Some(false) => self.note(format!("could not {label} {pid} {name}")),
            None => self.note(format!("{label} is not supported here")),
        }
    }
}

fn proc_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_user(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_exe(UpdateKind::OnlyIfNotSet)
        .without_tasks()
}

fn take_snap(sys: &System, users: Option<&Users>) -> Snap {
    let mem_total = sys.total_memory();
    let mut running = 0;
    let mut procs = Vec::with_capacity(sys.processes().len());
    for proc in sys.processes().values() {
        let state = status_char(proc.status());
        if state == 'R' {
            running += 1;
        }
        procs.push(to_proc(proc, users, mem_total, state));
    }
    let load = System::load_average().unwrap_or_default();
    Snap {
        procs,
        cores: sys.cpus().iter().map(|cpu| finite(cpu.usage())).collect(),
        cpu: finite(sys.global_cpu_usage()).clamp(0.0, 100.0),
        mem_used: sys.used_memory(),
        mem_total,
        swap_used: sys.used_swap(),
        swap_total: sys.total_swap(),
        load: [load.one, load.five, load.fifteen],
        uptime: System::uptime().unwrap_or(0),
        running,
    }
}

fn to_proc(proc: &Process, users: Option<&Users>, mem_total: u64, state: char) -> Proc {
    let mem = proc.memory();
    let mem_pct = if mem_total == 0 {
        0.0
    } else {
        (mem as f64 / mem_total as f64 * 100.0) as f32
    };
    let cmd = command_line(proc);
    let mut name = proc.name().to_string_lossy().into_owned();
    if name.is_empty() {
        name = cmd.clone();
    }
    if name.is_empty() {
        name = format!("pid {}", proc.pid());
    }
    Proc {
        pid: proc.pid().as_u32(),
        ppid: proc.parent().map(|pid| pid.as_u32()).unwrap_or(0),
        user: user_name(users, proc.user_id()),
        name,
        cmd,
        cpu: finite(proc.cpu_usage()),
        mem,
        mem_pct,
        time_ms: proc.accumulated_cpu_time(),
        state,
    }
}

fn command_line(proc: &Process) -> String {
    if proc.cmd().is_empty() {
        return proc
            .exe()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    proc.cmd()
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn user_name(users: Option<&Users>, uid: Option<&sysinfo::Uid>) -> String {
    let Some(uid) = uid else {
        return "-".into();
    };
    if let Some(users) = users
        && let Some(user) = users.get_user_by_id(uid)
    {
        return user.name().to_string();
    }
    format!("{}", uid.deref())
}

fn status_char(status: ProcessStatus) -> char {
    match status {
        ProcessStatus::Idle => 'I',
        ProcessStatus::Run => 'R',
        ProcessStatus::Sleep => 'S',
        ProcessStatus::Stop => 'T',
        ProcessStatus::Zombie => 'Z',
        ProcessStatus::Tracing => 't',
        ProcessStatus::Dead => 'D',
        ProcessStatus::Wakekill => 'W',
        ProcessStatus::Waking => 'w',
        ProcessStatus::Parked => 'P',
        ProcessStatus::LockBlocked => 'L',
        ProcessStatus::UninterruptibleDiskSleep => 'D',
        ProcessStatus::Suspended => 'B',
        ProcessStatus::Unknown(_) => '?',
    }
}

fn finite(n: f32) -> f32 {
    if n.is_finite() { n.max(0.0) } else { 0.0 }
}

fn run() -> io::Result<()> {
    let mut app = App::boot()?;
    let mut terminal = ratatui::try_init()?;
    let result = (|| {
        loop {
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            if app.quit {
                break;
            }
            let left = TICK.saturating_sub(app.last_sample.elapsed());
            if !left.is_zero() {
                match event::poll(left) {
                    Ok(false) => {}
                    Ok(true) => {
                        match event::read() {
                            Ok(Event::Key(key)) => app.on_key(key),
                            Ok(_) => {}
                            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                            Err(err) => return Err(err),
                        }
                        continue;
                    }
                    Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                    Err(err) => return Err(err),
                }
            }
            if !app.paused {
                app.sample();
            } else {
                app.last_sample = Instant::now();
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}

fn main() {
    if let Err(err) = run() {
        eprintln!("rtop: {err}");
        std::process::exit(1);
    }
}
