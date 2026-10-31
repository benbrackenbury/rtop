//! Layout and list logic. No terminal, so the phone-sized cases can be tested.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// Shorter than a classic 24-line terminal: one status line, rest is the list.
const COMPACT_MAX_ROWS: u16 = 23;
/// A 24-line terminal gets one meter line. Per-core bars start once there is room.
const CORES_MIN_ROWS: u16 = 28;
const FOOTER_MIN_ROWS: u16 = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    Pid,
    User,
    State,
    Cpu,
    Mem,
    Res,
    Time,
    Cmd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Cpu,
    Mem,
    Time,
    Pid,
    Name,
}

impl SortKey {
    pub fn default_desc(self) -> bool {
        matches!(self, SortKey::Cpu | SortKey::Mem | SortKey::Time)
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Cpu => "cpu",
            SortKey::Mem => "mem",
            SortKey::Time => "time",
            SortKey::Pid => "pid",
            SortKey::Name => "name",
        }
    }

    pub fn cycle(self, forward: bool) -> Self {
        const ORDER: [SortKey; 5] = [
            SortKey::Cpu,
            SortKey::Mem,
            SortKey::Time,
            SortKey::Pid,
            SortKey::Name,
        ];
        let i = ORDER.iter().position(|k| *k == self).unwrap_or(0);
        let n = ORDER.len();
        let j = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        ORDER[j]
    }

    pub fn column(self) -> Col {
        match self {
            SortKey::Cpu => Col::Cpu,
            SortKey::Mem => Col::Mem,
            SortKey::Time => Col::Time,
            SortKey::Pid => Col::Pid,
            SortKey::Name => Col::Cmd,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Header {
    None,
    Summary,
    Meters,
    Cores { rows: u16, cols: u16, group: usize },
}

impl Header {
    pub fn rows(&self) -> u16 {
        match self {
            Header::None => 0,
            Header::Summary => 1,
            Header::Meters => 2,
            Header::Cores { rows, .. } => 1 + *rows,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    pub columns: Vec<(Col, u16)>,
    pub header: Header,
    pub header_rows: u16,
    pub body_rows: u16,
    pub footer_rows: u16,
}

impl Screen {
    pub fn show_col_header(&self) -> bool {
        // Phone columns are pid / cpu / mem / cmd; the numbers already read.
        // Wider compact panes keep titles so USER / TIME stay obvious.
        self.body_rows >= 2
            && !self.columns.is_empty()
            && (self.columns.len() > 4 || !matches!(self.header, Header::Summary | Header::None))
    }

    pub fn process_rows(&self) -> u16 {
        if self.show_col_header() {
            self.body_rows.saturating_sub(1)
        } else {
            self.body_rows
        }
    }
}

pub fn screen(width: u16, height: u16, ncores: usize, force_footer: bool) -> Screen {
    let columns = pick_columns(width);
    let header = choose_header(width, height, ncores);
    let mut header_rows = header.rows().min(height);
    let want_footer = if height <= COMPACT_MAX_ROWS {
        force_footer
    } else {
        force_footer || height >= FOOTER_MIN_ROWS
    };
    let mut footer_rows = if want_footer && height > 0 { 1 } else { 0 };
    if header_rows + footer_rows > height {
        let overflow = header_rows + footer_rows - height;
        header_rows -= overflow.min(header_rows);
        footer_rows = height - header_rows;
    }
    let body_rows = height - header_rows - footer_rows;
    Screen {
        columns,
        header,
        header_rows,
        body_rows,
        footer_rows,
    }
}

fn choose_header(width: u16, height: u16, ncores: usize) -> Header {
    if height <= 2 {
        return Header::None;
    }
    if height <= COMPACT_MAX_ROWS {
        return Header::Summary;
    }
    if height < CORES_MIN_ROWS || ncores == 0 {
        return Header::Meters;
    }
    // Processes keep about 60% of the screen. Core bars use what is left.
    let min_processes = ((height as usize) * 3 / 5).max(8);
    let budget = (height as usize).saturating_sub(min_processes + 1 + 1 + 1);
    if budget < 2 {
        return Header::Meters;
    }
    let cell_w: usize = if width >= 100 { 16 } else { 12 };
    let cols = (width as usize / cell_w).max(1);
    let group = ncores.div_ceil(budget * cols).max(1);
    let cells = ncores.div_ceil(group);
    let rows = cells.div_ceil(cols).max(1);
    Header::Cores {
        rows: rows as u16,
        cols: cols as u16,
        group,
    }
}

pub fn pick_columns(width: u16) -> Vec<(Col, u16)> {
    if width == 0 {
        return Vec::new();
    }
    if width < 20 {
        return vec![(Col::Cmd, width)];
    }
    let user_w = if width >= 120 { 16 } else { 10 };
    let mut cols = vec![
        (Col::Pid, 7u16),
        (Col::User, user_w),
        (Col::State, 3),
        (Col::Cpu, 6),
        (Col::Mem, 5),
        (Col::Res, 6),
        (Col::Time, 9),
        (Col::Cmd, 0),
    ];
    let min_cmd = (width / 4).max(8);
    for drop in [
        Col::State,
        Col::User,
        Col::Res,
        Col::Time,
        Col::Pid,
        Col::Mem,
    ] {
        if columns_fit(&cols, width, min_cmd) {
            break;
        }
        cols.retain(|(c, _)| *c != drop);
    }
    if !columns_fit(&cols, width, min_cmd.min(4)) {
        for (c, w) in &mut cols {
            if *c == Col::Cpu {
                *w = (*w).min(4);
            }
        }
    }
    let gaps = (cols.len() as u16).saturating_sub(1);
    let fixed: u16 = cols
        .iter()
        .filter(|(c, _)| *c != Col::Cmd)
        .map(|(_, w)| *w)
        .sum();
    let cmd_w = width.saturating_sub(fixed.saturating_add(gaps)).max(1);
    for (c, w) in &mut cols {
        if *c == Col::Cmd {
            *w = cmd_w;
        }
    }
    cols
}

fn columns_fit(cols: &[(Col, u16)], width: u16, min_cmd: u16) -> bool {
    let gaps = (cols.len() as u16).saturating_sub(1);
    let fixed: u16 = cols
        .iter()
        .filter(|(c, _)| *c != Col::Cmd)
        .map(|(_, w)| *w)
        .sum();
    fixed.saturating_add(gaps).saturating_add(min_cmd) <= width
}

#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub name: String,
    pub cmd: String,
    pub cpu: f32,
    pub mem: u64,
    pub mem_pct: f32,
    pub time_ms: u64,
    pub state: char,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewRow {
    pub proc_index: usize,
    pub depth: u8,
}

pub fn build_view(
    procs: &[Proc],
    filter: &str,
    sort: SortKey,
    desc: bool,
    tree: bool,
) -> Vec<ViewRow> {
    let q = filter.trim().to_ascii_lowercase();
    let mut idxs: Vec<usize> = procs
        .iter()
        .enumerate()
        .filter(|(_, p)| matches_filter(p, &q))
        .map(|(i, _)| i)
        .collect();
    if idxs.is_empty() {
        return Vec::new();
    }
    if tree && q.is_empty() {
        return flatten_tree(procs, &idxs, sort, desc);
    }
    idxs.sort_by(|&a, &b| cmp_proc(&procs[a], &procs[b], sort, desc));
    idxs.into_iter()
        .map(|proc_index| ViewRow {
            proc_index,
            depth: 0,
        })
        .collect()
}

fn matches_filter(p: &Proc, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    p.name.to_ascii_lowercase().contains(q)
        || p.cmd.to_ascii_lowercase().contains(q)
        || p.user.to_ascii_lowercase().contains(q)
        || p.pid.to_string().contains(q)
}

fn cmp_proc(a: &Proc, b: &Proc, sort: SortKey, desc: bool) -> Ordering {
    let ord = match sort {
        SortKey::Cpu => a.cpu.total_cmp(&b.cpu),
        SortKey::Mem => a.mem.cmp(&b.mem),
        SortKey::Time => a.time_ms.cmp(&b.time_ms),
        SortKey::Pid => a.pid.cmp(&b.pid),
        SortKey::Name => a
            .name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase()),
    };
    let ord = if desc { ord.reverse() } else { ord };
    ord.then(a.pid.cmp(&b.pid))
}

fn flatten_tree(procs: &[Proc], idxs: &[usize], sort: SortKey, desc: bool) -> Vec<ViewRow> {
    let ids: HashSet<u32> = idxs.iter().map(|&i| procs[i].pid).collect();
    let mut kids: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut roots = Vec::new();
    for &i in idxs {
        let ppid = procs[i].ppid;
        if ppid != procs[i].pid && ids.contains(&ppid) {
            kids.entry(ppid).or_default().push(i);
        } else {
            roots.push(i);
        }
    }
    let cmp = |a: &usize, b: &usize| cmp_proc(&procs[*a], &procs[*b], sort, desc);
    roots.sort_by(cmp);
    for kids in kids.values_mut() {
        kids.sort_by(cmp);
    }
    let mut out = Vec::with_capacity(idxs.len());
    let mut seen = HashSet::new();
    for root in roots {
        walk(root, 0, procs, &kids, &mut seen, &mut out);
    }
    if out.len() != idxs.len() {
        let have: HashSet<u32> = out.iter().map(|r| procs[r.proc_index].pid).collect();
        let mut rest: Vec<usize> = idxs
            .iter()
            .copied()
            .filter(|&i| !have.contains(&procs[i].pid))
            .collect();
        rest.sort_by(cmp);
        for proc_index in rest {
            out.push(ViewRow {
                proc_index,
                depth: 0,
            });
        }
    }
    out
}

fn walk(
    idx: usize,
    depth: u8,
    procs: &[Proc],
    kids: &HashMap<u32, Vec<usize>>,
    seen: &mut HashSet<u32>,
    out: &mut Vec<ViewRow>,
) {
    let pid = procs[idx].pid;
    if !seen.insert(pid) || depth > 64 {
        return;
    }
    out.push(ViewRow {
        proc_index: idx,
        depth: depth.min(6),
    });
    if let Some(children) = kids.get(&pid) {
        for &child in children {
            walk(child, depth.saturating_add(1), procs, kids, seen, out);
        }
    }
}

/// Full command line on a wide column, short name when the column is a phone width.
pub fn command_text(p: &Proc, depth: u8, col_w: usize) -> String {
    let use_name = col_w < 36 || p.cmd.is_empty();
    let body = if use_name || p.name.is_empty() {
        if p.name.is_empty() { &p.cmd } else { &p.name }
    } else {
        &p.cmd
    };
    let depth = if col_w < 24 { depth.min(2) } else { depth };
    if depth == 0 {
        body.to_string()
    } else {
        format!("{}{body}", " ".repeat(depth as usize))
    }
}

pub fn sync_scroll(selected: usize, offset: usize, visible: usize, len: usize) -> usize {
    if len == 0 || visible == 0 {
        return 0;
    }
    let offset = if selected < offset {
        selected
    } else if selected >= offset + visible {
        selected + 1 - visible
    } else {
        offset
    };
    offset.min(len.saturating_sub(visible))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Text,
    Dim,
    Accent,
    Warm,
    Hot,
    Alert,
}

pub struct Headline {
    pub cpu: f32,
    pub mem_pct: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub load: [f64; 3],
    pub uptime: u64,
    pub procs: usize,
    pub shown: usize,
    pub running: usize,
    pub paused: bool,
    pub sort: SortKey,
    pub desc: bool,
    pub filtered: bool,
}

pub fn summary(h: &Headline, width: usize) -> Vec<(String, Tone)> {
    let mut parts = Vec::new();
    if h.paused {
        parts.push(("PAUSE".to_string(), Tone::Alert));
    }
    parts.push((
        format!("cpu {:.0}%", finite(h.cpu).clamp(0.0, 100.0)),
        heat_tone(h.cpu),
    ));
    parts.push((
        format!("mem {:.0}%", finite(h.mem_pct).clamp(0.0, 100.0)),
        heat_tone(h.mem_pct),
    ));
    parts.push((sort_label(h.sort, h.desc), Tone::Accent));
    let count = if h.filtered {
        format!("{}/{}", h.shown, h.procs)
    } else {
        format!("{} proc", h.procs)
    };
    parts.push((count, Tone::Text));
    if h.mem_total > 0 {
        parts.push((
            format!("{}/{}", human(h.mem_used), human(h.mem_total)),
            Tone::Text,
        ));
    }
    parts.push((
        format!("ld {:.2} {:.2} {:.2}", h.load[0], h.load[1], h.load[2]),
        Tone::Text,
    ));
    if h.running > 0 {
        parts.push((format!("{} run", h.running), Tone::Text));
    }
    if h.swap_total > 0 && h.swap_used > 0 {
        parts.push((
            format!("sw {}/{}", human(h.swap_used), human(h.swap_total)),
            Tone::Text,
        ));
    }
    parts.push((fmt_uptime(h.uptime), Tone::Dim));
    if width >= 72 {
        parts.insert(0, ("rtop".to_string(), Tone::Accent));
    }
    fit_parts(parts, width)
}

pub fn heat_tone(pct: f32) -> Tone {
    if !pct.is_finite() {
        return Tone::Text;
    }
    if pct >= 80.0 {
        Tone::Hot
    } else if pct >= 50.0 {
        Tone::Warm
    } else {
        Tone::Text
    }
}

pub fn sort_label(sort: SortKey, desc: bool) -> String {
    format!("{}{}", sort.label(), if desc { "↓" } else { "↑" })
}

pub fn fit_parts(mut parts: Vec<(String, Tone)>, width: usize) -> Vec<(String, Tone)> {
    if width == 0 {
        return Vec::new();
    }
    while parts.len() > 1 && parts_width(&parts) > width {
        parts.pop();
    }
    if let Some((text, _)) = parts.first_mut()
        && text.chars().count() > width
    {
        let clipped = clip(text, width);
        *text = clipped;
    }
    parts
}

fn parts_width(parts: &[(String, Tone)]) -> usize {
    if parts.is_empty() {
        return 0;
    }
    let text: usize = parts.iter().map(|(s, _)| s.chars().count()).sum();
    text + (parts.len() - 1) * 2
}

pub fn hints(width: usize) -> Vec<(&'static str, &'static str)> {
    const ALL: &[(&str, &str)] = &[
        ("q", "quit"),
        ("/", "filter"),
        ("x", "kill"),
        ("c", "cpu"),
        ("m", "mem"),
        ("t", "tree"),
        ("z", "pause"),
        ("?", "help"),
        ("s", "sort"),
        ("r", "rev"),
    ];
    let mut out = Vec::new();
    let mut used = 0;
    for (key, action) in ALL {
        let w = key.chars().count() + 1 + action.chars().count();
        let extra = if out.is_empty() { 0 } else { 2 };
        if used + extra + w > width {
            break;
        }
        used += extra + w;
        out.push((*key, *action));
    }
    out
}

pub fn help_text(height: u16, width: u16) -> String {
    const FULL: &[&str] = &[
        "rtop",
        "",
        "j k or arrows    move",
        "g G, home end    top and bottom",
        "ctrl-u ctrl-d    page",
        "/                filter    esc clears it",
        "c m              sort by cpu or memory",
        "n p              sort by name or pid",
        "s or left right  cycle sort    r reverse",
        "t                tree",
        "x                kill",
        "  t or enter     term",
        "  9              kill",
        "z                pause",
        "q                quit",
        "?                close help",
    ];
    const SHORT: &[&str] = &[
        "j k arrows   move",
        "/ filter     esc clear",
        "c cpu   m mem   s sort   r rev",
        "t tree  x kill  z pause",
        "q quit  ? close",
        "kill: t term, 9 kill",
    ];
    let lines: &[&str] = if height >= 20 { FULL } else { SHORT };
    let width = width as usize;
    let mut out = String::new();
    for (i, line) in lines.iter().take(height as usize).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&clip(line, width));
    }
    out
}

pub fn group_cores(cores: &[f32], group: usize) -> Vec<(String, f32)> {
    let group = group.max(1);
    cores
        .chunks(group)
        .enumerate()
        .map(|(i, chunk)| {
            let start = i * group + 1;
            let end = i * group + chunk.len();
            let label = if start == end {
                format!("{start}")
            } else {
                format!("{start}-{end}")
            };
            let pct = chunk.iter().copied().sum::<f32>() / chunk.len() as f32;
            (label, pct)
        })
        .collect()
}

pub fn bar_fill(pct: f32, width: usize) -> usize {
    if width == 0 {
        return 0;
    }
    let pct = finite(pct).clamp(0.0, 100.0);
    if pct == 0.0 {
        return 0;
    }
    let n = (pct / 100.0 * width as f32).round() as usize;
    // A non-zero reading should still show a sliver, or a 4% core looks idle.
    n.clamp(1, width)
}

pub fn human(bytes: u64) -> String {
    const UNITS: [(u64, &str); 5] = [
        (1 << 40, "T"),
        (1 << 30, "G"),
        (1 << 20, "M"),
        (1 << 10, "K"),
        (1, "B"),
    ];
    if bytes < 1024 {
        return format!("{bytes}B");
    }
    for (unit, suffix) in UNITS {
        if bytes >= unit {
            let v = bytes as f64 / unit as f64;
            if v >= 10.0 {
                return format!("{v:.0}{suffix}");
            }
            return format!("{v:.1}{suffix}");
        }
    }
    format!("{bytes}B")
}

pub fn fmt_cpu(cpu: f32) -> String {
    let cpu = finite(cpu);
    if cpu >= 100.0 {
        format!("{cpu:>5.0}")
    } else {
        format!("{cpu:>5.1}")
    }
}

pub fn fmt_pct(pct: f32) -> String {
    let pct = finite(pct).clamp(0.0, 100.0);
    if pct >= 100.0 {
        format!("{pct:>4.0}")
    } else {
        format!("{pct:>4.1}")
    }
}

pub fn fmt_time(ms: u64) -> String {
    let secs = ms / 1000;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h >= 100 {
        format!("{h}h")
    } else if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

pub fn fmt_uptime(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 {
        format!("up {d}d{h}h")
    } else if h > 0 {
        format!("up {h}h{m}m")
    } else {
        format!("up {m}m")
    }
}

/// Confirm line for a kill. The keys stay visible when the name does not fit.
pub fn kill_prompt(pid: u32, name: &str, width: usize) -> String {
    let tail = "  t term  9 kill  esc";
    let head = format!("kill {pid} ");
    let room = width.saturating_sub(head.chars().count() + tail.chars().count() + 1);
    let name = clip(name, room);
    let line = if name.is_empty() {
        format!("kill {pid}?{tail}")
    } else {
        format!("kill {pid} {name}?{tail}")
    };
    clip(&line, width)
}

pub fn clip(s: &str, width: usize) -> String {
    s.chars().take(width).collect()
}

pub fn pad_end(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        clip(s, width)
    } else {
        format!("{s}{}", " ".repeat(width - n))
    }
}

pub fn pad_start(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        clip(s, width)
    } else {
        format!("{}{s}", " ".repeat(width - n))
    }
}

fn finite(n: f32) -> f32 {
    if n.is_finite() { n.max(0.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths_ok(width: u16) {
        let cols = pick_columns(width);
        let sum: u16 = cols.iter().map(|(_, w)| *w).sum();
        let gaps = (cols.len() as u16).saturating_sub(1);
        assert_eq!(sum + gaps, width, "{width} -> {cols:?}");
        if let Some((Col::Cmd, w)) = cols.last() {
            assert!(*w >= 1);
        }
    }

    #[test]
    fn columns_fill_the_width() {
        for width in [20, 32, 40, 48, 60, 80, 100, 120, 200] {
            widths_ok(width);
        }
    }

    #[test]
    fn phone_width_keeps_pid_cpu_mem_and_a_name() {
        let cols = pick_columns(40);
        let names: Vec<_> = cols.iter().map(|(c, _)| *c).collect();
        assert_eq!(names, vec![Col::Pid, Col::Cpu, Col::Mem, Col::Cmd]);
        let cmd = cols.iter().find(|(c, _)| *c == Col::Cmd).unwrap().1;
        assert!(cmd >= 16, "cmd width {cmd}");
    }

    #[test]
    fn desktop_width_keeps_the_extra_columns() {
        let names: Vec<_> = pick_columns(100).into_iter().map(|(c, _)| c).collect();
        assert!(names.contains(&Col::User));
        assert!(names.contains(&Col::Time));
        assert!(names.contains(&Col::Res));
    }

    #[test]
    fn short_terminal_is_one_line_and_mostly_processes() {
        let s = screen(40, 12, 8, false);
        assert_eq!(s.header, Header::Summary);
        assert_eq!(s.footer_rows, 0);
        assert!(!s.show_col_header());
        assert!(s.process_rows() >= 10, "rows {}", s.process_rows());
        assert_eq!(s.header_rows + s.body_rows + s.footer_rows, 12);

        let phone = screen(40, 20, 8, false);
        assert_eq!(phone.header, Header::Summary);
        assert!(!phone.show_col_header());
        assert_eq!(phone.footer_rows, 0);
    }

    #[test]
    fn classic_terminal_has_meters_and_a_footer() {
        let s = screen(80, 24, 8, false);
        assert_eq!(s.header, Header::Meters);
        assert_eq!(s.footer_rows, 1);
        assert!(s.process_rows() >= 18, "rows {}", s.process_rows());
    }

    #[test]
    fn tall_terminal_uses_a_core_grid_and_keeps_the_list() {
        let s = screen(160, 50, 16, false);
        match s.header {
            Header::Cores { rows, .. } => assert!((1..=8).contains(&rows), "core rows {rows}"),
            other => panic!("expected cores, got {other:?}"),
        }
        assert!(s.process_rows() >= 30, "rows {}", s.process_rows());
    }

    #[test]
    fn many_cores_on_a_medium_screen_get_grouped() {
        let s = screen(80, 30, 128, false);
        match s.header {
            Header::Cores { group, rows, .. } => {
                assert!(group > 1, "group {group}");
                assert!(rows <= 10, "rows {rows}");
            }
            other => panic!("expected cores, got {other:?}"),
        }
        assert!(s.process_rows() >= 15, "rows {}", s.process_rows());
    }

    #[test]
    fn geometry_always_adds_up() {
        for height in 1..45 {
            for width in [40, 80] {
                let s = screen(width, height, 8, false);
                assert_eq!(s.header_rows + s.body_rows + s.footer_rows, height);
                if height <= COMPACT_MAX_ROWS {
                    assert!(s.header_rows <= 1);
                    assert_eq!(s.footer_rows, 0);
                }
            }
        }
    }

    #[test]
    fn prompt_takes_a_footer_row_on_a_phone() {
        let s = screen(40, 12, 8, true);
        assert_eq!(s.footer_rows, 1);
        assert!(s.process_rows() >= 8);
    }

    fn proc(pid: u32, ppid: u32, name: &str, cpu: f32, mem: u64) -> Proc {
        Proc {
            pid,
            ppid,
            user: "me".into(),
            name: name.into(),
            cmd: format!("/bin/{name} --flag"),
            cpu,
            mem,
            mem_pct: 0.0,
            time_ms: 0,
            state: 'S',
        }
    }

    #[test]
    fn sort_breaks_cpu_ties_by_pid() {
        let procs = vec![
            proc(2, 0, "b", 5.0, 1),
            proc(1, 0, "a", 5.0, 9),
            proc(3, 0, "c", 1.0, 1),
        ];
        let view = build_view(&procs, "", SortKey::Cpu, true, false);
        let pids: Vec<_> = view.iter().map(|r| procs[r.proc_index].pid).collect();
        assert_eq!(pids, vec![1, 2, 3]);
    }

    #[test]
    fn filter_matches_name_cmd_and_pid_and_flattens_the_tree() {
        let procs = vec![
            proc(1, 0, "init", 0.0, 1),
            proc(2, 1, "nginx", 3.0, 1),
            proc(10, 1, "other", 0.0, 1),
        ];
        let view = build_view(&procs, "NGINX", SortKey::Pid, false, true);
        assert_eq!(view.len(), 1);
        assert_eq!(procs[view[0].proc_index].pid, 2);
        assert_eq!(view[0].depth, 0);

        let by_pid = build_view(&procs, "10", SortKey::Pid, false, false);
        assert_eq!(procs[by_pid[0].proc_index].name, "other");
    }

    #[test]
    fn tree_nests_children_and_survives_a_cycle() {
        let procs = vec![
            proc(1, 0, "init", 1.0, 1),
            proc(3, 1, "b", 9.0, 1),
            proc(2, 1, "a", 1.0, 1),
            proc(4, 2, "a-child", 0.0, 1),
        ];
        let view = build_view(&procs, "", SortKey::Name, false, true);
        let got: Vec<_> = view
            .iter()
            .map(|r| (procs[r.proc_index].pid, r.depth))
            .collect();
        assert_eq!(got, vec![(1, 0), (2, 1), (4, 2), (3, 1)]);

        let cyc = vec![proc(1, 2, "a", 0.0, 1), proc(2, 1, "b", 0.0, 1)];
        let view = build_view(&cyc, "", SortKey::Pid, false, true);
        assert_eq!(view.len(), 2);
    }

    #[test]
    fn narrow_command_uses_the_short_name() {
        let p = proc(1, 0, "nginx", 0.0, 1);
        assert_eq!(command_text(&p, 0, 20), "nginx");
        assert_eq!(command_text(&p, 0, 60), "/bin/nginx --flag");
        assert_eq!(command_text(&p, 2, 60), "  /bin/nginx --flag");
    }

    #[test]
    fn scroll_keeps_the_selection_on_screen() {
        assert_eq!(sync_scroll(10, 0, 5, 20), 6);
        assert_eq!(sync_scroll(2, 6, 5, 20), 2);
        assert_eq!(sync_scroll(3, 2, 5, 20), 2);
        assert_eq!(sync_scroll(0, 0, 5, 0), 0);
    }

    #[test]
    fn summary_keeps_cpu_on_a_narrow_line_and_grows() {
        let h = Headline {
            cpu: 12.0,
            mem_pct: 48.0,
            mem_used: 8 << 30,
            mem_total: 16 << 30,
            swap_used: 0,
            swap_total: 0,
            load: [1.2, 0.8, 0.5],
            uptime: 90_000,
            procs: 140,
            shown: 140,
            running: 2,
            paused: false,
            sort: SortKey::Cpu,
            desc: true,
            filtered: false,
        };
        let narrow = summary(&h, 32);
        let text = narrow
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>()
            .join("  ");
        assert!(text.chars().count() <= 32, "{text}");
        assert!(text.contains("cpu"));
        assert!(text.contains("mem"));
        assert!(!text.contains("ld "));

        let wide = summary(&h, 100);
        let text = wide
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>()
            .join("  ");
        assert!(text.chars().count() <= 100, "{text}");
        assert!(text.contains("rtop"));
        assert!(text.contains("ld "));
        assert!(text.starts_with("rtop"));
    }

    #[test]
    fn human_and_bars_and_groups() {
        assert_eq!(human(0), "0B");
        assert_eq!(human(1024), "1.0K");
        assert_eq!(human(1536), "1.5K");
        assert_eq!(human(10 << 20), "10M");
        assert_eq!(bar_fill(0.0, 10), 0);
        assert_eq!(bar_fill(100.0, 10), 10);
        assert_eq!(bar_fill(50.0, 10), 5);
        let groups = group_cores(&[10.0, 30.0, 50.0, 70.0], 2);
        assert_eq!(groups[0].0, "1-2");
        assert!((groups[0].1 - 20.0).abs() < 0.01);
        assert_eq!(groups[1].0, "3-4");
    }

    #[test]
    fn hints_shrink_and_help_fits_a_short_screen() {
        let h = hints(40);
        let w: usize = h
            .iter()
            .map(|(k, a)| k.chars().count() + 1 + a.chars().count())
            .sum::<usize>()
            + h.len().saturating_sub(1) * 2;
        assert!(w <= 40, "{h:?} is {w}");
        assert!(h.iter().any(|(k, _)| *k == "x"));
        let help = help_text(12, 40);
        assert!(help.lines().count() <= 12);
        assert!(help.contains("kill"));
    }

    #[test]
    fn kill_prompt_keeps_the_keys_on_a_narrow_line() {
        let line = kill_prompt(38108, "rtop-635660d488514ce7", 40);
        assert!(line.chars().count() <= 40, "{line}");
        assert!(line.contains("t term"));
        assert!(line.contains("9 kill"));
        assert!(line.contains("38108"));
    }
}
