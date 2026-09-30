//! Static and managed UI installation integration scenarios.

use super::*;

#[path = "installation/pins.rs"]
mod pins;
#[path = "installation/static_ui.rs"]
mod static_ui;

#[path = "installation/guards.rs"]
mod guards;

#[path = "installation/global.rs"]
mod global;
#[path = "installation/lifecycle.rs"]
mod lifecycle;
