use crossterm::event::KeyEvent;

use crate::{App, handle_key_impl};

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
        handle_key_impl(self.app, key, self.width);
        InteractionResult::Consumed
    }
}
