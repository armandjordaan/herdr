use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};

use crate::app::state::{AppState, GotoItem, GotoTarget, Mode};

const GOTO_PAGE_STEP: usize = 8;

pub(crate) fn open_goto(state: &mut AppState) {
    state.goto.filter.clear();
    state.goto.items = rebuild_items(state);
    state.goto.list = state
        .goto
        .items
        .iter()
        .position(|item| item.is_current)
        .unwrap_or(0);
    state.mode = Mode::Goto;
}

pub(crate) fn handle_goto_key(state: &mut AppState, key: KeyEvent) {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => leave_goto(state),
        (KeyCode::Enter, _) => apply_goto(state),
        (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            state.goto.list = state.goto.list.saturating_sub(1);
        }
        (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
            if !state.goto.items.is_empty() {
                state.goto.list = (state.goto.list + 1).min(state.goto.items.len() - 1);
            }
        }
        (KeyCode::PageUp, _) => {
            state.goto.list = state.goto.list.saturating_sub(GOTO_PAGE_STEP);
        }
        (KeyCode::PageDown, _) => {
            if !state.goto.items.is_empty() {
                state.goto.list = state
                    .goto
                    .list
                    .saturating_add(GOTO_PAGE_STEP)
                    .min(state.goto.items.len() - 1);
            }
        }
        (KeyCode::Home, _) => {
            state.goto.list = 0;
        }
        (KeyCode::End, _) => {
            if !state.goto.items.is_empty() {
                state.goto.list = state.goto.items.len() - 1;
            }
        }
        (KeyCode::Backspace, _) => {
            state.goto.filter.pop();
            rerank(state);
        }
        (KeyCode::Char(c), mods)
            if mods == KeyModifiers::empty() || mods == KeyModifiers::SHIFT =>
        {
            state.goto.filter.push(c);
            rerank(state);
        }
        _ => {}
    }
}

fn leave_goto(state: &mut AppState) {
    state.goto.filter.clear();
    state.goto.items.clear();
    state.goto.list = 0;
    state.mode = if state.active.is_some() {
        Mode::Terminal
    } else {
        Mode::Navigate
    };
}

fn apply_goto(state: &mut AppState) {
    let Some(item) = state.goto.items.get(state.goto.list).cloned() else {
        leave_goto(state);
        return;
    };
    match item.target {
        GotoTarget::Space { ws_idx } => {
            state.switch_workspace(ws_idx);
        }
        GotoTarget::Tab { ws_idx, tab_idx } => {
            state.switch_workspace(ws_idx);
            state.switch_tab(tab_idx);
        }
        GotoTarget::Agent {
            ws_idx,
            tab_idx,
            pane_id,
        } => {
            state.switch_workspace(ws_idx);
            state.switch_tab(tab_idx);
            if let Some(tab) = state
                .workspaces
                .get_mut(ws_idx)
                .and_then(|ws| ws.tabs.get_mut(tab_idx))
            {
                tab.layout.focus_pane(pane_id);
            }
        }
    }
    leave_goto(state);
}

pub(crate) fn rebuild_items(state: &AppState) -> Vec<GotoItem> {
    let mut items = Vec::new();
    let active_ws = state.active;
    let focused_pane = active_ws
        .and_then(|i| state.workspaces.get(i))
        .and_then(|ws| ws.focused_pane_id());

    for (ws_idx, ws) in state.workspaces.iter().enumerate() {
        let ws_name = ws.display_name_from(&state.terminals, &state.terminal_runtimes);
        items.push(GotoItem {
            target: GotoTarget::Space { ws_idx },
            label: format!("[space] {ws_name}"),
            haystack: format!("space {ws_name}").to_lowercase(),
            is_current: Some(ws_idx) == active_ws,
            agent_status: None,
        });

        for (tab_idx, tab) in ws.tabs.iter().enumerate() {
            let tab_name = tab.display_name();
            let tab_label = format!("[tab]   {ws_name} \u{203a} {tab_name}");
            let tab_haystack = format!("tab {ws_name} {tab_name}").to_lowercase();
            items.push(GotoItem {
                target: GotoTarget::Tab { ws_idx, tab_idx },
                label: tab_label,
                haystack: tab_haystack,
                is_current: Some(ws_idx) == active_ws && ws.active_tab == tab_idx,
                agent_status: None,
            });

            for pane_id in tab.layout.pane_ids() {
                let Some(pane) = tab.panes.get(&pane_id) else {
                    continue;
                };
                let Some(terminal) = state.terminals.get(&pane.attached_terminal_id) else {
                    continue;
                };
                // Match the sidebar's Agents panel: only panes where the
                // terminal has an effective_agent_label (auto-detected agent
                // or hook authority). agent_name only overrides the display.
                let Some(effective) = terminal.effective_agent_label() else {
                    continue;
                };
                let agent_label = terminal
                    .agent_name
                    .clone()
                    .unwrap_or_else(|| effective.to_string());

                let status = crate::ui::status::state_label(terminal.state, pane.seen);
                let agent_label_view = format!(
                    "[agent] {ws_name} \u{203a} {tab_name} \u{203a} {agent_label}"
                );
                let agent_haystack = format!(
                    "agent {ws_name} {tab_name} {agent_label} {status}"
                )
                .to_lowercase();
                items.push(GotoItem {
                    target: GotoTarget::Agent {
                        ws_idx,
                        tab_idx,
                        pane_id,
                    },
                    label: agent_label_view,
                    haystack: agent_haystack,
                    is_current: Some(ws_idx) == active_ws
                        && ws.active_tab == tab_idx
                        && focused_pane == Some(pane_id),
                    agent_status: Some((terminal.state, pane.seen)),
                });
            }
        }
    }

    items
}

fn rerank(state: &mut AppState) {
    let all = rebuild_items(state);

    if state.goto.filter.is_empty() {
        let selected_pos = all
            .iter()
            .position(|item| item.is_current)
            .unwrap_or(0);
        state.goto.items = all;
        state.goto.list = selected_pos.min(state.goto.items.len().saturating_sub(1));
        return;
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(
        &state.goto.filter,
        CaseMatching::Ignore,
        Normalization::Smart,
    );
    let mut scored: Vec<_> = all
        .into_iter()
        .filter_map(|item| {
            let mut buf = Vec::new();
            let haystack = nucleo_matcher::Utf32Str::new(&item.haystack, &mut buf);
            pattern
                .score(haystack, &mut matcher)
                .map(|score| (item, score))
        })
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1));
    state.goto.items = scored.into_iter().map(|(item, _)| item).collect();
    state.goto.list = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;

    fn state_with_two_workspaces() -> AppState {
        let mut state = AppState::test_new();
        state.workspaces = vec![
            crate::workspace::Workspace::test_new("alpha"),
            crate::workspace::Workspace::test_new("beta"),
        ];
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        state.ensure_test_terminals();
        state
    }

    #[test]
    fn open_goto_populates_items_and_preselects_current() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state);
        assert_eq!(state.mode, Mode::Goto);
        assert!(!state.goto.items.is_empty());
        let selected = &state.goto.items[state.goto.list];
        assert!(selected.is_current);
    }

    #[test]
    fn rebuild_skips_non_agent_panes() {
        let state = state_with_two_workspaces();
        let items = rebuild_items(&state);
        let spaces = items
            .iter()
            .filter(|i| matches!(i.target, GotoTarget::Space { .. }))
            .count();
        let tabs = items
            .iter()
            .filter(|i| matches!(i.target, GotoTarget::Tab { .. }))
            .count();
        let agents = items
            .iter()
            .filter(|i| matches!(i.target, GotoTarget::Agent { .. }))
            .count();
        assert_eq!(spaces, 2);
        assert!(tabs >= 2);
        assert_eq!(agents, 0, "plain shell panes must not appear as agents");
    }

    #[test]
    fn rebuild_emits_agent_row_when_terminal_has_detected_agent() {
        use crate::detect::{Agent, AgentState};

        let mut state = state_with_two_workspaces();
        // Simulate the detector having identified claude in the second workspace.
        let ws = &state.workspaces[1];
        let pane_id = ws.tabs[0].root_pane;
        let terminal_id = ws.tabs[0]
            .panes
            .get(&pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();
        let terminal = state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        terminal.set_agent_name("my-claude".into());

        let items = rebuild_items(&state);
        let agent_rows: Vec<_> = items
            .iter()
            .filter(|i| matches!(i.target, GotoTarget::Agent { .. }))
            .collect();
        assert_eq!(agent_rows.len(), 1);
        // agent_name overrides the displayed label.
        assert!(agent_rows[0].label.contains("my-claude"));
        // Agent rows carry a status so the picker can render idle/working/etc.
        assert_eq!(
            agent_rows[0].agent_status,
            Some((AgentState::Idle, true))
        );
        // And the haystack lets the user filter by that status.
        assert!(agent_rows[0].haystack.contains("idle"));
        // Non-agent rows stay status-less.
        assert!(items
            .iter()
            .filter(|i| !matches!(i.target, GotoTarget::Agent { .. }))
            .all(|i| i.agent_status.is_none()));
    }

    #[test]
    fn rebuild_skips_pane_with_agent_name_but_no_detected_agent() {
        // A bare agent_name (no auto-detection, no hook authority) is not
        // enough — matches the sidebar's filter so the picker stays
        // consistent with what the user sees in the Agents panel.
        let mut state = state_with_two_workspaces();
        let ws = &state.workspaces[1];
        let pane_id = ws.tabs[0].root_pane;
        let terminal_id = ws.tabs[0]
            .panes
            .get(&pane_id)
            .unwrap()
            .attached_terminal_id
            .clone();
        state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_agent_name("orphan".into());

        let items = rebuild_items(&state);
        let agents = items
            .iter()
            .filter(|i| matches!(i.target, GotoTarget::Agent { .. }))
            .count();
        assert_eq!(agents, 0);
    }

    #[test]
    fn enter_jumps_to_selected_space() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state);
        let beta_idx = state
            .goto
            .items
            .iter()
            .position(|item| {
                matches!(item.target, GotoTarget::Space { ws_idx } if ws_idx == 1)
            })
            .expect("beta space row");
        state.goto.list = beta_idx;
        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );
        assert_eq!(state.active, Some(1));
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn esc_closes_without_navigating() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state);
        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
        );
        assert_eq!(state.mode, Mode::Terminal);
        assert_eq!(state.active, Some(0));
        assert!(state.goto.items.is_empty());
    }

    #[test]
    fn page_up_down_jumps_by_step_and_clamps() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state);
        let total = state.goto.items.len();
        assert!(total > 0);

        state.goto.list = 0;
        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::empty()),
        );
        let expected_down = GOTO_PAGE_STEP.min(total - 1);
        assert_eq!(state.goto.list, expected_down);

        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::empty()),
        );
        assert_eq!(state.goto.list, expected_down.saturating_sub(GOTO_PAGE_STEP));

        // PageDown past the end clamps to the last item.
        state.goto.list = total - 1;
        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::empty()),
        );
        assert_eq!(state.goto.list, total - 1);

        // PageUp at the top stays at 0.
        state.goto.list = 0;
        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::empty()),
        );
        assert_eq!(state.goto.list, 0);
    }

    #[test]
    fn home_and_end_jump_to_edges() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state);
        let total = state.goto.items.len();
        assert!(total > 0);

        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::End, KeyModifiers::empty()),
        );
        assert_eq!(state.goto.list, total - 1);

        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::Home, KeyModifiers::empty()),
        );
        assert_eq!(state.goto.list, 0);
    }

    #[test]
    fn typing_filters_items() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state);
        let before = state.goto.items.len();
        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty()),
        );
        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()),
        );
        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::empty()),
        );
        handle_goto_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()),
        );
        assert!(state.goto.items.len() < before);
        assert!(state
            .goto
            .items
            .iter()
            .all(|item| item.haystack.contains("beta")));
    }
}
