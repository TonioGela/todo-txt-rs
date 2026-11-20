//! Rendering and theming.
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Tabs},
    Frame,
};

use crate::{todo::{self, Axis}, App, Prompt};

const HELP: &str = "tab axis · ←/→ value · ↑/↓ move · x done · a add · e edit · d show done · / filter · q quit";

/// Every color the UI uses. `nord()` is the default; add more constructors for other palettes.
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub muted: Color,
    pub border: Color,
    pub accent: Color,
    pub selected_bg: Color,
    pub selected_fg: Color,
    pub project: Color,
    pub context: Color,
    pub error: Color,
    /// Colors for priorities A, B, C. Anything lower uses `fg`.
    pub priorities: [Color; 3],
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Theme {
    pub fn nord() -> Theme {
        Theme {
            bg: rgb(0x2E3440),          // nord0
            fg: rgb(0xD8DEE9),          // nord4
            muted: rgb(0x4C566A),       // nord3
            border: rgb(0x4C566A),      // nord3
            accent: rgb(0x88C0D0),      // nord8
            selected_bg: rgb(0x434C5E), // nord2
            selected_fg: rgb(0xECEFF4), // nord6
            project: rgb(0xA3BE8C),     // nord14
            context: rgb(0xB48EAD),     // nord15
            error: rgb(0xBF616A),       // nord11
            priorities: [
                rgb(0xBF616A), // nord11
                rgb(0xD08770), // nord12
                rgb(0xEBCB8B), // nord13
            ],
        }
    }

    fn priority_color(&self, letter: char) -> Color {
        let i = (letter as u8 - b'A') as usize;
        self.priorities.get(i).copied().unwrap_or(self.fg)
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(t.bg).fg(t.fg)), area);
    let [panel, keymap] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

    let block = Block::bordered().border_style(Style::default().fg(t.border));
    let inner = block.inner(panel);
    f.render_widget(block, panel);

    // With no project, context or priority in the file there is nothing to filter by,
    // so the tabs and values rows are left out and the list takes the whole panel.
    let visible = app.visible();
    let list_area = if visible.is_empty() {
        inner
    } else {
        let [tabs_row, values_row, list_area] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .areas(inner);
        let tabs_area = underlined(f, tabs_row, t.border);
        let values_area = underlined(f, values_row, t.border);

        let selected = visible
            .iter()
            .position(|&a| a == Axis::ALL[app.slot])
            .unwrap_or(0);
        let tabs = Tabs::new(visible.iter().map(|a| a.title()))
            .select(selected)
            .style(Style::default().fg(t.muted))
            .highlight_style(Style::default().fg(t.accent).add_modifier(Modifier::BOLD))
            .divider("  ");
        f.render_widget(tabs, tabs_area);
        f.render_widget(values_line(app), values_area);
        list_area
    };

    let src = app.source();
    let open = app.lines.len();
    let rows = app.rows();
    let list = List::new(rows.iter().map(|&i| {
        // Done items are rendered muted, without the priority and filter colors.
        if i >= open {
            ListItem::new(Line::styled(src[i].to_string(), Style::default().fg(t.muted)))
        } else {
            ListItem::new(line_spans(src[i], t))
        }
    }))
        .highlight_style(
            Style::default()
                .bg(t.selected_bg)
                .fg(t.selected_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut list_state = ListState::default().with_selected((!rows.is_empty()).then_some(app.item));
    f.render_stateful_widget(list, list_area, &mut list_state);

    match &app.input {
        Some(buf) => {
            let prompt = match app.prompt {
                Prompt::Add => "add> ",
                Prompt::Edit(_) => "edit> ",
                Prompt::Filter => "/",
            };
            f.render_widget(
                Line::from(vec![
                    Span::styled(prompt, Style::default().fg(t.accent)),
                    Span::raw(buf.as_str()),
                ]),
                keymap,
            );
            let x = keymap.x + (prompt.len() + app.cursor) as u16;
            f.set_cursor_position((x, keymap.y));
        }
        None if !app.status.is_empty() => f.render_widget(
            Line::styled(app.status.as_str(), Style::default().fg(t.error)),
            keymap,
        ),
        None if !app.query.is_empty() => f.render_widget(
            Line::styled(
                format!("/{} · esc clears", app.query),
                Style::default().fg(t.accent),
            ),
            keymap,
        ),
        None => f.render_widget(Line::styled(HELP, Style::default().fg(t.muted)), keymap),
    }
}

/// Draws a rule under `area` and returns the space above it.
fn underlined(f: &mut Frame, area: Rect, color: Color) -> Rect {
    let rule = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(color));
    let inner = rule.inner(area);
    f.render_widget(rule, area);
    inner
}

/// The values of the focused filter, with the selected one highlighted.
fn values_line(app: &App) -> Line<'static> {
    let t = &app.theme;
    let axis = Axis::ALL[app.slot];
    if !app.visible().contains(&axis) {
        return Line::default();
    }
    let mut spans = Vec::new();
    for (i, (value, _)) in app.values(axis).iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let style = if i == app.filters[app.slot] {
            Style::default()
                .bg(t.selected_bg)
                .fg(t.selected_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg)
        };
        spans.push(Span::styled(format!(" {} ", display(value)), style));
    }
    Line::from(spans)
}

/// "+Nix" -> "Nix", "@home" -> "home", "(A)" -> "A".
fn display(value: &str) -> &str {
    value
        .trim_start_matches(['+', '@'])
        .trim_start_matches('(')
        .trim_end_matches(')')
}

/// Colors priority, +project and @context words; everything else is plain.
fn line_spans(line: &str, t: &Theme) -> Line<'static> {
    let pri = todo::priority(line);
    let mut spans = Vec::new();
    for (n, word) in line.split(' ').enumerate() {
        if n > 0 {
            spans.push(Span::raw(" "));
        }
        let style = match (n, pri, word.as_bytes().first()) {
            (0, Some(p), _) => Style::default()
                .fg(t.priority_color(p))
                .add_modifier(Modifier::BOLD),
            (_, _, Some(b'+')) => Style::default().fg(t.project),
            (_, _, Some(b'@')) => Style::default().fg(t.context),
            _ => Style::default().fg(t.fg),
        };
        spans.push(Span::styled(word.to_string(), style));
    }
    Line::from(spans)
}
