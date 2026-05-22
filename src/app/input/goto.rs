use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};

use crate::app::state::{AppState, GotoCategory, GotoItem, GotoTarget, Mode};
use crate::detect::AgentState;
use crate::terminal::TerminalRuntimeRegistry;

const GOTO_PAGE_STEP: usize = 8;

pub(crate) fn open_goto(state: &mut AppState, terminal_runtimes: &TerminalRuntimeRegistry) {
    state.goto.filter.clear();
    state.goto.category = None;
    state.goto.items = rebuild_items(state, terminal_runtimes);
    state.goto.list = state
        .goto
        .items
        .iter()
        .position(|item| item.is_current)
        .unwrap_or(0);
    state.mode = Mode::Goto;
}

pub(crate) fn open_goto_with_category(
    state: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    category: GotoCategory,
) {
    open_goto(state, terminal_runtimes);
    state.goto.category = Some(category);
    rerank(state, terminal_runtimes);
}

pub(crate) fn handle_goto_key(
    state: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    key: KeyEvent,
) {
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
            rerank(state, terminal_runtimes);
        }
        (KeyCode::Char('t'), KeyModifiers::ALT) => {
            toggle_category(state, terminal_runtimes, GotoCategory::Tabs)
        }
        (KeyCode::Char('w'), KeyModifiers::ALT) => {
            toggle_category(state, terminal_runtimes, GotoCategory::Workspaces)
        }
        (KeyCode::Char('a'), KeyModifiers::ALT) => {
            toggle_category(state, terminal_runtimes, GotoCategory::Agents)
        }
        (KeyCode::Char('b'), KeyModifiers::ALT) => {
            toggle_category(state, terminal_runtimes, GotoCategory::BlockedAgents)
        }
        (KeyCode::Char(c), mods)
            if mods == KeyModifiers::empty() || mods == KeyModifiers::SHIFT =>
        {
            state.goto.filter.push(c);
            rerank(state, terminal_runtimes);
        }
        _ => {}
    }
}

fn toggle_category(
    state: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    category: GotoCategory,
) {
    state.goto.category = if state.goto.category == Some(category) {
        None
    } else {
        Some(category)
    };
    rerank(state, terminal_runtimes);
}

fn leave_goto(state: &mut AppState) {
    state.goto.filter.clear();
    state.goto.items.clear();
    state.goto.list = 0;
    state.goto.category = None;
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

pub(crate) fn rebuild_items(
    state: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<GotoItem> {
    let mut items = Vec::new();
    let active_ws = state.active;
    let focused_pane = active_ws
        .and_then(|i| state.workspaces.get(i))
        .and_then(|ws| ws.focused_pane_id());

    for (ws_idx, ws) in state.workspaces.iter().enumerate() {
        let ws_name = ws.display_name_from(&state.terminals, terminal_runtimes);
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

fn rerank(state: &mut AppState, terminal_runtimes: &TerminalRuntimeRegistry) {
    let all: Vec<GotoItem> = rebuild_items(state, terminal_runtimes)
        .into_iter()
        .filter(|item| matches_category(item, state.goto.category))
        .collect();

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

fn matches_category(item: &GotoItem, category: Option<GotoCategory>) -> bool {
    match category {
        None => true,
        Some(GotoCategory::Workspaces) => matches!(item.target, GotoTarget::Space { .. }),
        Some(GotoCategory::Tabs) => matches!(item.target, GotoTarget::Tab { .. }),
        Some(GotoCategory::Agents) => matches!(item.target, GotoTarget::Agent { .. }),
        Some(GotoCategory::BlockedAgents) => {
            matches!(item.target, GotoTarget::Agent { .. })
                && matches!(item.agent_status, Some((AgentState::Blocked, _)))
        }
    }
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
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        assert_eq!(state.mode, Mode::Goto);
        assert!(!state.goto.items.is_empty());
        let selected = &state.goto.items[state.goto.list];
        assert!(selected.is_current);
    }

    #[test]
    fn rebuild_skips_non_agent_panes() {
        let state = state_with_two_workspaces();
        let items = rebuild_items(&state, &TerminalRuntimeRegistry::new());
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

        let items = rebuild_items(&state, &TerminalRuntimeRegistry::new());
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

        let items = rebuild_items(&state, &TerminalRuntimeRegistry::new());
        let agents = items
            .iter()
            .filter(|i| matches!(i.target, GotoTarget::Agent { .. }))
            .count();
        assert_eq!(agents, 0);
    }

    #[test]
    fn enter_jumps_to_selected_space() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
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
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );
        assert_eq!(state.active, Some(1));
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn esc_closes_without_navigating() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
        );
        assert_eq!(state.mode, Mode::Terminal);
        assert_eq!(state.active, Some(0));
        assert!(state.goto.items.is_empty());
    }

    #[test]
    fn page_up_down_jumps_by_step_and_clamps() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        let total = state.goto.items.len();
        assert!(total > 0);

        state.goto.list = 0;
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::empty()),
        );
        let expected_down = GOTO_PAGE_STEP.min(total - 1);
        assert_eq!(state.goto.list, expected_down);

        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::empty()),
        );
        assert_eq!(state.goto.list, expected_down.saturating_sub(GOTO_PAGE_STEP));

        // PageDown past the end clamps to the last item.
        state.goto.list = total - 1;
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::empty()),
        );
        assert_eq!(state.goto.list, total - 1);

        // PageUp at the top stays at 0.
        state.goto.list = 0;
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::empty()),
        );
        assert_eq!(state.goto.list, 0);
    }

    #[test]
    fn home_and_end_jump_to_edges() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        let total = state.goto.items.len();
        assert!(total > 0);

        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::End, KeyModifiers::empty()),
        );
        assert_eq!(state.goto.list, total - 1);

        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Home, KeyModifiers::empty()),
        );
        assert_eq!(state.goto.list, 0);
    }

    #[test]
    fn alt_w_filters_to_workspaces_only() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
        );
        assert_eq!(state.goto.category, Some(GotoCategory::Workspaces));
        assert!(!state.goto.items.is_empty());
        assert!(state
            .goto
            .items
            .iter()
            .all(|item| matches!(item.target, GotoTarget::Space { .. })));
    }

    #[test]
    fn alt_t_filters_to_tabs_only() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT),
        );
        assert_eq!(state.goto.category, Some(GotoCategory::Tabs));
        assert!(!state.goto.items.is_empty());
        assert!(state
            .goto
            .items
            .iter()
            .all(|item| matches!(item.target, GotoTarget::Tab { .. })));
    }

    #[test]
    fn alt_b_filters_to_blocked_agents_only() {
        use crate::detect::{Agent, AgentState};
        let mut state = state_with_two_workspaces();
        // Make one agent blocked and another idle so we can confirm the filter.
        for ws_idx in 0..2 {
            let ws = &state.workspaces[ws_idx];
            let pane_id = ws.tabs[0].root_pane;
            let terminal_id = ws.tabs[0]
                .panes
                .get(&pane_id)
                .unwrap()
                .attached_terminal_id
                .clone();
            let terminal = state.terminals.get_mut(&terminal_id).unwrap();
            let agent_state = if ws_idx == 0 {
                AgentState::Idle
            } else {
                AgentState::Blocked
            };
            terminal.set_detected_state(Some(Agent::Claude), agent_state);
        }
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
        );
        assert_eq!(state.goto.category, Some(GotoCategory::BlockedAgents));
        assert_eq!(state.goto.items.len(), 1);
        assert!(matches!(
            state.goto.items[0].agent_status,
            Some((AgentState::Blocked, _))
        ));
    }

    #[test]
    fn open_goto_with_category_applies_blocked_filter() {
        use crate::detect::{Agent, AgentState};
        let mut state = state_with_two_workspaces();
        for ws_idx in 0..2 {
            let ws = &state.workspaces[ws_idx];
            let pane_id = ws.tabs[0].root_pane;
            let terminal_id = ws.tabs[0]
                .panes
                .get(&pane_id)
                .unwrap()
                .attached_terminal_id
                .clone();
            let terminal = state.terminals.get_mut(&terminal_id).unwrap();
            let agent_state = if ws_idx == 0 {
                AgentState::Idle
            } else {
                AgentState::Blocked
            };
            terminal.set_detected_state(Some(Agent::Claude), agent_state);
        }
        open_goto_with_category(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            GotoCategory::BlockedAgents,
        );
        assert_eq!(state.mode, Mode::Goto);
        assert_eq!(state.goto.category, Some(GotoCategory::BlockedAgents));
        assert_eq!(state.goto.items.len(), 1);
        assert!(matches!(
            state.goto.items[0].agent_status,
            Some((AgentState::Blocked, _))
        ));
    }

    #[test]
    fn alt_key_toggles_category_off() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        let total = state.goto.items.len();
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
        );
        assert_eq!(state.goto.category, Some(GotoCategory::Workspaces));
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
        );
        assert_eq!(state.goto.category, None);
        assert_eq!(state.goto.items.len(), total);
    }

    #[test]
    fn alt_key_replaces_prior_category() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
        );
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT),
        );
        assert_eq!(state.goto.category, Some(GotoCategory::Tabs));
    }

    #[test]
    fn plain_letter_keys_still_type_into_filter() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        for ch in ['t', 'w', 'a', 'b'] {
            handle_goto_key(
                &mut state,
                &TerminalRuntimeRegistry::new(),
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()),
            );
        }
        assert_eq!(state.goto.filter, "twab");
        assert_eq!(state.goto.category, None);
    }

    #[test]
    fn typing_filters_items() {
        let mut state = state_with_two_workspaces();
        open_goto(&mut state, &TerminalRuntimeRegistry::new());
        let before = state.goto.items.len();
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty()),
        );
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()),
        );
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::empty()),
        );
        handle_goto_key(
            &mut state,
            &TerminalRuntimeRegistry::new(),
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
