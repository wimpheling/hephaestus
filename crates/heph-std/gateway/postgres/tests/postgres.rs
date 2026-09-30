//! Opt-in real `PostgreSQL` proof for gateway mailbox and service authority.

mod postgres {
    mod acceptance;
    mod configuration;
    mod installation;
    mod mailbox_publication;
    mod management;
    mod outbox;
    mod service_declaration;
    mod service_declaration_followup;
    mod service_launch;
    mod support;
}
