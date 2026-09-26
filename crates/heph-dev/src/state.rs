//! Development state lifecycle and resource management.

mod database;
mod images;
mod resources;

use crate::{cli::StateSelection, context::DevContext, process::Result};

pub fn list(context: &DevContext) -> Result<()> {
    resources::list(context)
}

pub fn init(context: &DevContext, selection: &StateSelection) -> Result<()> {
    resources::init(context, selection)
}

pub fn clean(context: &DevContext, selection: &StateSelection) -> Result<()> {
    resources::clean(context, selection)
}

pub fn reinit(context: &DevContext, selection: &StateSelection) -> Result<()> {
    resources::reinit(context, selection)
}

#[cfg(test)]
#[path = "state/tests.rs"]
mod tests;
