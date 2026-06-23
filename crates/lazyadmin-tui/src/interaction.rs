use crossterm::event::{KeyCode, KeyEvent};
use tracing::info;

use crate::{
    App, Command, InputMode, ViewKind, capture_selected_listener_id, cycle_pane,
    handle_confirmation_key, handle_key_impl, key_to_command_with_bindings, mark_overview_seen,
    rebuild_view_model, restore_selected_listener_id, scroll_rows, set_active_view,
    sync_row_selection,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InteractionResult {
    Consumed,
}

pub(crate) struct InteractionCore<'a> {
    app: &'a mut App,
    width: u16,
}

impl<'a> InteractionCore<'a> {
    pub(crate) fn new(app: &'a mut App, width: u16) -> Self {
        Self { app, width }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> InteractionResult {
        if handle_confirmation_key(self.app, key) {
            return InteractionResult::Consumed;
        }
        self.dismiss_overview_hint();
        if self.handle_search_key(key) {
            return InteractionResult::Consumed;
        }
        if matches!(self.app.mode, InputMode::Normal) && self.handle_sort_key(key) {
            return InteractionResult::Consumed;
        }
        handle_key_impl(self.app, key, self.width);
        InteractionResult::Consumed
    }

    fn dismiss_overview_hint(&mut self) {
        if self.app.active_view == ViewKind::Overview && self.app.overview_hint_visible {
            self.app.overview_hint_visible = false;
            if let Err(err) = mark_overview_seen() {
                self.app
                    .set_status(format!("overview hint flag not saved: {err}"));
            }
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> bool {
        if !matches!(self.app.mode, InputMode::Search) {
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                self.app.query.clear();
                self.app.mode = InputMode::Normal;
                let target = self
                    .app
                    .return_view_on_clear
                    .take()
                    .unwrap_or(self.app.search_origin_view);
                if self.app.active_view == ViewKind::Search {
                    set_active_view(self.app, target, self.width);
                }
                self.app.set_status("search cleared");
                rebuild_view_model(self.app, self.width);
            }
            KeyCode::Enter => {
                if self.app.active_view == ViewKind::Search
                    && self.app.vm.search.search_hit_count() > 0
                    && !self.app.query.trim().is_empty()
                {
                    self.app.mode = InputMode::Normal;
                    crate::jump_to_search_result(self.app, self.width);
                } else if !self.app.query.trim().is_empty() {
                    let target = self
                        .app
                        .return_view_on_clear
                        .take()
                        .unwrap_or(self.app.search_origin_view);
                    set_active_view(self.app, target, self.width);
                    self.app.mode = InputMode::Normal;
                } else {
                    self.app.mode = InputMode::Normal;
                }
            }
            KeyCode::Backspace => {
                self.app.query.pop();
                if self.app.query.is_empty() {
                    if self.app.active_view == ViewKind::Search {
                        let target = self
                            .app
                            .return_view_on_clear
                            .unwrap_or(self.app.search_origin_view);
                        set_active_view(self.app, target, self.width);
                    } else {
                        rebuild_view_model(self.app, self.width);
                    }
                } else {
                    if self.app.active_view != ViewKind::Search {
                        self.app.return_view_on_clear = Some(self.app.active_view);
                        self.app.search_origin_view = self.app.active_view;
                    }
                    set_active_view(self.app, ViewKind::Search, self.width);
                }
            }
            KeyCode::Char(c) => {
                if self.app.query.is_empty() && self.app.active_view != ViewKind::Search {
                    self.app.return_view_on_clear = Some(self.app.active_view);
                    self.app.search_origin_view = self.app.active_view;
                }
                info!(
                    origin_view = ?self.app.search_origin_view,
                    "tui.search.activate"
                );
                set_active_view(self.app, ViewKind::Search, self.width);
                self.app.query.push(c);
                rebuild_view_model(self.app, self.width);
            }
            KeyCode::Up => scroll_rows(self.app, -1),
            KeyCode::Down => scroll_rows(self.app, 1),
            KeyCode::PageUp => scroll_rows(self.app, -10),
            KeyCode::PageDown => scroll_rows(self.app, 10),
            KeyCode::Tab => {
                self.app.mode = InputMode::Normal;
                cycle_pane(self.app, 1, self.width);
            }
            KeyCode::BackTab => {
                self.app.mode = InputMode::Normal;
                cycle_pane(self.app, -1, self.width);
            }
            KeyCode::Home => {
                self.app.selected_row = 0;
                sync_row_selection(self.app);
            }
            KeyCode::End => {
                let count = self.app.vm.search.search_hit_count();
                self.app.selected_row = count.saturating_sub(1);
                sync_row_selection(self.app);
            }
            _ => {}
        }
        true
    }

    fn handle_sort_key(&mut self, key: KeyEvent) -> bool {
        let Some(command) = key_to_command_with_bindings(key, &self.app.keybindings) else {
            return false;
        };
        let label = match command {
            Command::SortNext => {
                let (captured_id, old_index) = capture_selected_listener_id(self.app);
                self.app.listener_sort = self.app.listener_sort.next_column();
                rebuild_view_model(self.app, self.width);
                restore_selected_listener_id(self.app, captured_id, old_index);
                Some(self.app.listener_sort.label())
            }
            Command::SortPrev => {
                let (captured_id, old_index) = capture_selected_listener_id(self.app);
                self.app.listener_sort = self.app.listener_sort.prev_column();
                rebuild_view_model(self.app, self.width);
                restore_selected_listener_id(self.app, captured_id, old_index);
                Some(self.app.listener_sort.label())
            }
            Command::SortToggle => {
                let (captured_id, old_index) = capture_selected_listener_id(self.app);
                self.app.listener_sort = self.app.listener_sort.toggle_direction();
                rebuild_view_model(self.app, self.width);
                restore_selected_listener_id(self.app, captured_id, old_index);
                Some(self.app.listener_sort.label())
            }
            _ => None,
        };
        if let Some(label) = label {
            self.app.set_status(format!(
                "sorted by {} {}",
                label,
                self.app.listener_sort.indicator()
            ));
            true
        } else {
            false
        }
    }
}
