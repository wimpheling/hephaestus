//! Real `PostgreSQL` UI gateway authority and worker privilege matrix.
//!
//! These tests use the same published release and installation fixture shape
//! as the browser authentication schema tests, then exercise the gateway
//! worker adapter through its public edge ports.

mod ui_browser_authority {
    mod admission_cases;
    mod child_recheck;
    mod parent_recheck;
    mod support;
}
