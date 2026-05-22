use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use super::status::{state_label, state_label_color};
use super::widgets::{centered_popup_rect, panel_contrast_fg};
use crate::app::state::GotoCategory;
use crate::app::AppState;

pub(super) fn render_goto_overlay(app: &AppState, frame: &mut Frame) {
    let area = frame.area();
    let target_w = (area.width as f32 * 0.7) as u16;
    let popup_w = target_w.clamp(40, 90).min(area.width.saturating_sub(4));
    let popup_h = (area.height.saturating_sub(4)).min(20).max(8);
    let Some(rect) = centered_popup_rect(area, popup_w, popup_h) else {
        return;
    };

    let p = &app.palette;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.accent))
        .style(Style::default().bg(p.panel_bg));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let [header_area, filter_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas::<4>(inner);

    let header = Line::from(vec![Span::styled(
        " Goto",
        Style::default().fg(p.text).add_modifier(Modifier::BOLD),
    )]);
    frame.render_widget(Paragraph::new(header), header_area);

    let prompt = Line::from(vec![
        Span::styled(" > ", Style::default().fg(p.accent)),
        Span::styled(app.goto.filter.as_str(), Style::default().fg(p.text)),
        Span::styled("_", Style::default().fg(p.overlay0)),
    ]);
    frame.render_widget(Paragraph::new(prompt), filter_area);

    render_footer(app, frame, footer_area);

    let total = app.goto.items.len();
    let height = list_area.height as usize;
    if total == 0 || height == 0 {
        let empty = Line::from(Span::styled(
            "  no matches",
            Style::default().fg(p.overlay0),
        ));
        frame.render_widget(Paragraph::new(empty), list_area);
        return;
    }

    let selected = app.goto.list.min(total.saturating_sub(1));
    let start = if selected >= height {
        selected + 1 - height
    } else {
        0
    };
    let end = (start + height).min(total);

    for (row, idx) in (start..end).enumerate() {
        let item = &app.goto.items[idx];
        let y = list_area.y + row as u16;
        let row_rect = Rect::new(list_area.x, y, list_area.width, 1);
        let is_selected = idx == selected;
        let base_style = if is_selected {
            Style::default()
                .fg(panel_contrast_fg(p))
                .bg(p.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.text)
        };
        let marker = if item.is_current { " *" } else { "  " };

        let mut spans: Vec<Span> = vec![Span::styled(
            format!(" {}", item.label),
            base_style,
        )];
        if let Some((agent_state, seen)) = item.agent_status {
            let status_fg = if is_selected {
                panel_contrast_fg(p)
            } else {
                state_label_color(agent_state, seen, p)
            };
            let status_style = if is_selected {
                base_style
            } else {
                Style::default().fg(status_fg)
            };
            spans.push(Span::styled(" ", base_style));
            spans.push(Span::styled(
                format!("[{}]", state_label(agent_state, seen)),
                status_style,
            ));
        }
        spans.push(Span::styled(marker.to_string(), base_style));

        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line).style(base_style), row_rect);
    }
}

fn render_footer(app: &AppState, frame: &mut Frame, area: Rect) {
    let p = &app.palette;
    let active = app.goto.category;
    let entries: [(&str, &str, GotoCategory); 4] = [
        ("alt+t", "tabs", GotoCategory::Tabs),
        ("alt+w", "workspaces", GotoCategory::Workspaces),
        ("alt+a", "agents", GotoCategory::Agents),
        ("alt+b", "blocked", GotoCategory::BlockedAgents),
    ];

    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for (i, (key, label, category)) in entries.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default().fg(p.overlay0)));
        }
        let is_active = active == Some(*category);
        let key_style = if is_active {
            Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.overlay1)
        };
        let label_style = if is_active {
            Style::default().fg(p.text).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.overlay0)
        };
        spans.push(Span::styled((*key).to_string(), key_style));
        spans.push(Span::styled(" ", label_style));
        spans.push(Span::styled((*label).to_string(), label_style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
