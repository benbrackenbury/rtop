use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use crate::view::{
    self, Header, Headline, Screen, Tone, bar_fill, clip, command_text, fmt_cpu, fmt_pct, fmt_time,
    group_cores, heat_tone, human, pad_end, pad_start,
};
use crate::{App, Mode};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let view = app.view();
    app.track_selection(&view);
    let screen = view::screen(
        area.width,
        area.height,
        app.snap.cores.len(),
        app.wants_footer(),
    );
    app.visible = (screen.process_rows() as usize).max(1);
    app.offset = view::sync_scroll(app.selected, app.offset, app.visible, view.len());

    if matches!(app.mode, Mode::Help) {
        let help = view::help_text(area.height, area.width);
        let paragraph = Paragraph::new(help).style(Style::new().fg(Color::White).bg(Color::Black));
        frame.render_widget(paragraph, area);
        return;
    }

    let mut constraints = Vec::new();
    if screen.header_rows > 0 {
        constraints.push(Constraint::Length(screen.header_rows));
    }
    constraints.push(Constraint::Fill(1));
    if screen.footer_rows > 0 {
        constraints.push(Constraint::Length(screen.footer_rows));
    }
    let chunks = Layout::vertical(constraints).split(area);
    let mut i = 0;
    if screen.header_rows > 0 {
        draw_header(frame, app, &view, &screen, chunks[i]);
        i += 1;
    }
    if screen.body_rows > 0 {
        draw_table(frame, app, &view, &screen, chunks[i]);
        i += 1;
    }
    if screen.footer_rows > 0 && i < chunks.len() {
        draw_footer(frame, app, chunks[i]);
    }
}

fn draw_header(frame: &mut Frame, app: &App, view: &[view::ViewRow], screen: &Screen, area: Rect) {
    let headline = Headline {
        cpu: app.snap.cpu,
        mem_pct: app.snap.mem_pct(),
        mem_used: app.snap.mem_used,
        mem_total: app.snap.mem_total,
        swap_used: app.snap.swap_used,
        swap_total: app.snap.swap_total,
        load: app.snap.load,
        uptime: app.snap.uptime,
        procs: app.snap.procs.len(),
        shown: view.len(),
        running: app.snap.running,
        paused: app.paused,
        sort: app.sort,
        desc: app.desc,
        filtered: !app.filter.is_empty(),
    };
    let mut lines = vec![summary_line(&headline, area.width as usize)];
    match &screen.header {
        Header::Meters if screen.header_rows >= 2 => {
            lines.push(meter_line(&app.snap, area.width));
        }
        Header::Cores { rows, cols, group } => {
            let fit = screen.header_rows.saturating_sub(1).min(*rows);
            lines.extend(core_lines(&app.snap.cores, *cols, *group, fit, area.width));
        }
        _ => {}
    }
    lines.truncate(screen.header_rows as usize);
    frame.render_widget(Paragraph::new(lines), area);
}

fn summary_line(headline: &Headline, width: usize) -> Line<'static> {
    let parts = view::summary(headline, width);
    let mut spans = Vec::new();
    for (i, (text, tone)) in parts.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(text, tone_style(tone)));
    }
    Line::from(spans)
}

fn meter_line(snap: &crate::Snap, width: u16) -> Line<'static> {
    let width = width as usize;
    let cpu = snap.cpu.clamp(0.0, 100.0);
    let mem = snap.mem_pct();
    let cpu_pct = format!(" {cpu:.0}%");
    let mem_pair = format!(" {}/{}", human(snap.mem_used), human(snap.mem_total));
    let both_fixed =
        "cpu ".len() + cpu_pct.chars().count() + "  mem ".len() + mem_pair.chars().count();
    if width >= both_fixed + 8 && snap.mem_total > 0 {
        let bar_w = (width - both_fixed) / 2;
        let mut spans = Vec::new();
        spans.push(Span::styled("cpu ", tone_style(Tone::Dim)));
        spans.extend(bar_spans(cpu, bar_w));
        spans.push(Span::styled(cpu_pct, tone_style(heat_tone(cpu))));
        spans.push(Span::styled("  mem ", tone_style(Tone::Dim)));
        spans.extend(bar_spans(mem, width - both_fixed - bar_w));
        spans.push(Span::styled(mem_pair, tone_style(heat_tone(mem))));
        Line::from(spans)
    } else {
        let label = "cpu ";
        let bar_w = width.saturating_sub(label.len() + cpu_pct.chars().count());
        let mut spans = vec![Span::styled(label, tone_style(Tone::Dim))];
        spans.extend(bar_spans(cpu, bar_w));
        spans.push(Span::styled(cpu_pct, tone_style(heat_tone(cpu))));
        Line::from(spans)
    }
}

fn core_lines(cores: &[f32], cols: u16, group: usize, rows: u16, width: u16) -> Vec<Line<'static>> {
    let cells = group_cores(cores, group);
    let cols = (cols as usize).max(1);
    let mut lines = Vec::new();
    for row in 0..rows as usize {
        let start = row * cols;
        if start >= cells.len() {
            break;
        }
        let end = (start + cols).min(cells.len());
        lines.push(core_line(&cells[start..end], width));
    }
    lines
}

fn core_line(cells: &[(String, f32)], width: u16) -> Line<'static> {
    if cells.is_empty() || width == 0 {
        return Line::raw("");
    }
    let n = cells.len();
    let gaps = n.saturating_sub(1);
    let cell_w = (width as usize).saturating_sub(gaps) / n;
    if cell_w == 0 {
        return Line::raw("");
    }
    let label_w = cells
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(1)
        .min(cell_w.saturating_sub(4).max(1));
    let mut spans = Vec::new();
    for (i, (label, pct)) in cells.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let pct_s = format!("{:>3.0}%", finite_pct(*pct));
        let label = pad_start(&clip(label, label_w), label_w);
        let chrome = label_w + 1 + pct_s.chars().count();
        let bar_w = cell_w.saturating_sub(chrome + 1);
        if bar_w >= 1 {
            let fill = bar_fill(*pct, bar_w);
            spans.push(Span::styled(label, tone_style(Tone::Dim)));
            spans.push(Span::raw(" "));
            spans.push(Span::styled("█".repeat(fill), tone_style(heat_tone(*pct))));
            spans.push(Span::styled(
                "░".repeat(bar_w - fill),
                tone_style(Tone::Dim),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(pct_s, tone_style(heat_tone(*pct))));
        } else {
            let body = pad_end(&format!("{label} {pct_s}"), cell_w);
            spans.push(Span::styled(body, tone_style(heat_tone(*pct))));
        }
    }
    Line::from(spans)
}

fn bar_spans(pct: f32, width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let fill = bar_fill(pct, width);
    vec![
        Span::styled("█".repeat(fill), tone_style(heat_tone(pct))),
        Span::styled("░".repeat(width - fill), tone_style(Tone::Dim)),
    ]
}

fn draw_table(frame: &mut Frame, app: &App, view: &[view::ViewRow], screen: &Screen, area: Rect) {
    if screen.columns.is_empty() {
        return;
    }
    let header = if screen.show_col_header() {
        let cells = screen.columns.iter().map(|(col, width)| {
            let title = col_title(*col, *width as usize, app);
            let style = if app.sort.column() == *col {
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            };
            Cell::from(title).style(style)
        });
        Some(Row::new(cells))
    } else {
        None
    };

    let start = app.offset.min(view.len());
    let end = (start + screen.process_rows() as usize).min(view.len());
    let rows: Vec<Row> = if view.is_empty() {
        let cells = screen.columns.iter().map(|(col, _)| {
            if *col == view::Col::Cmd {
                Cell::from("no matching processes")
            } else {
                Cell::from("")
            }
        });
        vec![Row::new(cells)]
    } else {
        view[start..end]
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let proc = &app.snap.procs[row.proc_index];
                let selected = start + i == app.selected;
                let cells = screen.columns.iter().map(|(col, width)| {
                    let text = cell_text(*col, proc, row.depth, *width as usize);
                    let mut style = if selected {
                        Style::new()
                    } else {
                        cell_style(*col, proc)
                    };
                    if proc.state == 'Z' && !selected {
                        style = style.fg(Color::Red);
                    }
                    Cell::from(text).style(style)
                });
                let mut table_row = Row::new(cells);
                if selected {
                    table_row = table_row.style(Style::new().add_modifier(Modifier::REVERSED));
                }
                table_row
            })
            .collect()
    };

    let widths: Vec<Constraint> = screen
        .columns
        .iter()
        .map(|(col, width)| {
            if *col == view::Col::Cmd {
                Constraint::Fill(1)
            } else {
                Constraint::Length(*width)
            }
        })
        .collect();
    let mut table = Table::new(rows, widths).column_spacing(1);
    if let Some(header) = header {
        table = table.header(header);
    }
    frame.render_widget(table, area);
}

fn col_title(col: view::Col, width: usize, app: &App) -> String {
    let mut name = match col {
        view::Col::Pid => "PID",
        view::Col::User => "USER",
        view::Col::State => "S",
        view::Col::Cpu => "CPU%",
        view::Col::Mem => "MEM%",
        view::Col::Res => "RES",
        view::Col::Time => "TIME",
        view::Col::Cmd => "COMMAND",
    }
    .to_string();
    if app.sort.column() == col {
        name.push(if app.desc { '↓' } else { '↑' });
    }
    match col {
        view::Col::Pid | view::Col::Cpu | view::Col::Mem | view::Col::Res | view::Col::Time => {
            pad_start(&name, width)
        }
        _ => name,
    }
}

fn cell_text(col: view::Col, proc: &view::Proc, depth: u8, width: usize) -> String {
    match col {
        view::Col::Pid => pad_start(&proc.pid.to_string(), width),
        view::Col::User => proc.user.clone(),
        view::Col::State => proc.state.to_string(),
        view::Col::Cpu => pad_start(&fmt_cpu(proc.cpu), width),
        view::Col::Mem => pad_start(&fmt_pct(proc.mem_pct), width),
        view::Col::Res => pad_start(&human(proc.mem), width),
        view::Col::Time => pad_start(&fmt_time(proc.time_ms), width),
        view::Col::Cmd => command_text(proc, depth, width),
    }
}

fn cell_style(col: view::Col, proc: &view::Proc) -> Style {
    let tone = match col {
        view::Col::Cpu if proc.cpu >= 100.0 => Tone::Hot,
        view::Col::Cpu if proc.cpu >= 50.0 => Tone::Warm,
        view::Col::Mem if proc.mem_pct >= 40.0 => Tone::Hot,
        view::Col::Mem if proc.mem_pct >= 20.0 => Tone::Warm,
        _ => Tone::Text,
    };
    tone_style(tone)
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let line = match &app.mode {
        Mode::Kill { pid, name } => Line::from(Span::styled(
            view::kill_prompt(*pid, name, area.width as usize),
            tone_style(Tone::Alert),
        )),
        Mode::Filter => {
            let text = clip(&app.filter, (area.width as usize).saturating_sub(2));
            Line::from(vec![
                Span::styled("/", tone_style(Tone::Accent)),
                Span::raw(text),
                Span::styled("█", tone_style(Tone::Accent)),
            ])
        }
        _ if !app.filter.is_empty() => {
            let text = clip(&app.filter, (area.width as usize).saturating_sub(12));
            Line::from(vec![
                Span::styled("/", tone_style(Tone::Accent)),
                Span::raw(text),
                Span::styled("  esc clear", tone_style(Tone::Dim)),
            ])
        }
        _ => {
            if let Some(status) = app.fresh_status() {
                Line::from(Span::styled(status.to_string(), tone_style(Tone::Warm)))
            } else {
                hint_line(area.width as usize)
            }
        }
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn hint_line(width: usize) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (key, action)) in view::hints(width).into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            key,
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(action, tone_style(Tone::Dim)));
    }
    Line::from(spans)
}

fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Text => Style::new(),
        Tone::Dim => Style::new().fg(Color::DarkGray),
        Tone::Accent => Style::new().fg(Color::Cyan),
        Tone::Warm => Style::new().fg(Color::Yellow),
        Tone::Hot => Style::new().fg(Color::Red),
        Tone::Alert => Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}

fn finite_pct(n: f32) -> f32 {
    if n.is_finite() {
        n.clamp(0.0, 999.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn frame(width: u16, height: u16, app: &mut App) -> Vec<String> {
        let (lines, _) = render(width, height, app);
        lines
    }

    fn render(width: u16, height: u16, app: &mut App) -> (Vec<String>, Vec<bool>) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let lines = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect();
        let reversed = (0..height)
            .map(|y| {
                (0..width).any(|x| {
                    buf[(x, y)]
                        .modifier
                        .contains(ratatui::style::Modifier::REVERSED)
                })
            })
            .collect();
        (lines, reversed)
    }

    #[test]
    fn layouts_for_phone_classic_and_wide() {
        let mut app = App::boot().unwrap();
        assert!(!app.snap.procs.is_empty());

        let (phone, reversed) = render(40, 12, &mut app);
        assert!(phone[0].contains("cpu"), "summary: {:?}", phone[0]);
        assert!(reversed[2], "selected process row is not highlighted");
        assert!(phone[0].contains("mem"), "summary: {:?}", phone[0]);
        assert!(phone[1].contains("PID"), "header: {:?}", phone[1]);
        assert!(phone[1].contains("CPU"), "header: {:?}", phone[1]);
        assert!(
            !phone[11].contains("quit"),
            "footer ate a phone row: {:?}",
            phone[11]
        );
        let listed = phone
            .iter()
            .skip(2)
            .filter(|line| line.chars().any(|c| c.is_ascii_digit()))
            .count();
        assert!(
            listed >= 8,
            "expected processes on a 12-row screen, got {phone:?}"
        );

        let classic = frame(80, 24, &mut app);
        assert!(classic[0].contains("cpu"), "{:?}", classic[0]);
        assert!(classic[1].contains('█'), "meter: {:?}", classic[1]);
        assert!(classic[2].contains("PID"), "{:?}", classic[2]);
        assert!(classic[23].contains("quit"), "footer: {:?}", classic[23]);

        let wide = frame(140, 40, &mut app);
        assert!(wide[0].contains("rtop"), "wide summary: {:?}", wide[0]);
        assert!(
            wide.iter().any(|line| line.contains('█')),
            "core bars missing"
        );
        assert!(
            wide[1].contains("% "),
            "core cells ran together: {:?}",
            wide[1]
        );
        assert!(wide[39].contains("quit"), "footer: {:?}", wide[39]);

        app.mode = Mode::Filter;
        app.filter = "grok".into();
        let filtering = frame(40, 12, &mut app);
        assert!(
            filtering[11].contains("grok"),
            "filter: {:?}",
            filtering[11]
        );
        assert!(
            filtering
                .iter()
                .take(11)
                .any(|line| line.to_ascii_lowercase().contains("grok")),
            "filtered list: {filtering:?}"
        );

        app.mode = Mode::Kill {
            pid: 123,
            name: "rtop-635660d488514ce7".into(),
        };
        let killing = frame(40, 12, &mut app);
        assert!(killing[11].contains("9 kill"), "kill: {:?}", killing[11]);
        assert!(killing[11].contains("123"));

        app.mode = Mode::Help;
        let help = frame(40, 12, &mut app);
        assert!(help.iter().any(|line| line.contains("move")), "{help:?}");
    }
}
