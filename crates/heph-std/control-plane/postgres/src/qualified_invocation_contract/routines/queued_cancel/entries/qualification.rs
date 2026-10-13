use super::Entry;

pub(super) const QUEUED_CREATE_ORIGINAL140: Entry = Entry {
    name: "queued_create_original140",
    signature: "public.queued_create_original140(uuid)",
    arguments: "p_instance uuid",
    result: "uuid",
    names: &["p_instance"],
    types: &["pg_catalog.uuid"],
    modes: &["i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_PROCESSED_CLOSURE140: Entry = Entry {
    name: "queued_processed_closure140",
    signature: "public.queued_processed_closure140(uuid,uuid,jsonb)",
    arguments: "p_run uuid, p_instance uuid, p_original jsonb",
    result: "void",
    names: &["p_run", "p_instance", "p_original"],
    types: &["pg_catalog.uuid", "pg_catalog.uuid", "pg_catalog.jsonb"],
    modes: &["i", "i", "i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_CANCEL_CLOSURE140: Entry = Entry {
    name: "queued_cancel_closure140",
    signature: "public.queued_cancel_closure140(uuid,uuid,jsonb,jsonb)",
    arguments: "p_run uuid, p_instance uuid, p_original jsonb, p_cancel jsonb",
    result: "void",
    names: &["p_run", "p_instance", "p_original", "p_cancel"],
    types: &[
        "pg_catalog.uuid",
        "pg_catalog.uuid",
        "pg_catalog.jsonb",
        "pg_catalog.jsonb",
    ],
    modes: &["i", "i", "i", "i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_CANCEL_ORIGINAL140: Entry = Entry {
    name: "queued_cancel_original140",
    signature: "public.queued_cancel_original140(uuid,jsonb)",
    arguments: "p_run uuid, p_cancel jsonb",
    result: "jsonb",
    names: &["p_run", "p_cancel"],
    types: &["pg_catalog.uuid", "pg_catalog.jsonb"],
    modes: &["i", "i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_REMOVAL_ORIGIN140: Entry = Entry {
    name: "queued_removal_origin140",
    signature: "public.queued_removal_origin140(uuid)",
    arguments: "p_instance uuid",
    result: "jsonb",
    names: &["p_instance"],
    types: &["pg_catalog.uuid"],
    modes: &["i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_ORDINARY_ORIGINAL140: Entry = Entry {
    name: "queued_ordinary_original140",
    signature: "public.queued_ordinary_original140(uuid,jsonb)",
    arguments: "p_run uuid, p_cancel jsonb",
    result: "jsonb",
    names: &["p_run", "p_cancel"],
    types: &["pg_catalog.uuid", "pg_catalog.jsonb"],
    modes: &["i", "i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_ORDINARY_ZERO140: Entry = Entry {
    name: "queued_ordinary_zero140",
    signature: "public.queued_ordinary_zero140(runs)",
    arguments: "execution runs",
    result: "boolean",
    names: &["execution"],
    types: &["public.runs"],
    modes: &["i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_DRAIN_ZERO140: Entry = Entry {
    name: "queued_drain_zero140",
    signature: "public.queued_drain_zero140(runs)",
    arguments: "execution runs",
    result: "boolean",
    names: &["execution"],
    types: &["public.runs"],
    modes: &["i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_ORDINARY_EVENTS140: Entry = Entry {
    name: "queued_ordinary_events140",
    signature: "public.queued_ordinary_events140(runs,jsonb,boolean)",
    arguments: "execution runs, p_original jsonb, p_prepared boolean",
    result: "boolean",
    names: &["execution", "p_original", "p_prepared"],
    types: &["public.runs", "pg_catalog.jsonb", "pg_catalog.bool"],
    modes: &["i", "i", "i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_DRAIN_EVENTS140: Entry = Entry {
    name: "queued_drain_events140",
    signature: "public.queued_drain_events140(runs,boolean)",
    arguments: "execution runs, p_prepared boolean",
    result: "boolean",
    names: &["execution", "p_prepared"],
    types: &["public.runs", "pg_catalog.bool"],
    modes: &["i", "i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_ORDINARY_QUALIFY140: Entry = Entry {
    name: "queued_ordinary_qualify140",
    signature: "public.queued_ordinary_qualify140(uuid,jsonb,text,text,bytea,text)",
    arguments: "p_run uuid, p_original jsonb, p_namespace text, p_host text, p_config bytea, p_mode text",
    result: "runs",
    names: &[
        "p_run",
        "p_original",
        "p_namespace",
        "p_host",
        "p_config",
        "p_mode",
    ],
    types: &[
        "pg_catalog.uuid",
        "pg_catalog.jsonb",
        "pg_catalog.text",
        "pg_catalog.text",
        "pg_catalog.bytea",
        "pg_catalog.text",
    ],
    modes: &["i", "i", "i", "i", "i", "i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_CLEANUP_TARGET140: Entry = Entry {
    name: "queued_cleanup_target140",
    signature: "public.queued_cleanup_target140(runs,boolean,boolean,boolean)",
    arguments: "execution runs, p_completing boolean, p_events boolean, p_ordinary boolean",
    result: "void",
    names: &["execution", "p_completing", "p_events", "p_ordinary"],
    types: &[
        "public.runs",
        "pg_catalog.bool",
        "pg_catalog.bool",
        "pg_catalog.bool",
    ],
    modes: &["i", "i", "i", "i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_RESUME_ASSOCIATIONS140: Entry = Entry {
    name: "queued_resume_associations140",
    signature: "public.queued_resume_associations140(uuid)",
    arguments: "p_instance uuid",
    result: "boolean",
    names: &["p_instance"],
    types: &["pg_catalog.uuid"],
    modes: &["i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};

pub(super) const QUEUED_INVOCATION_RESUME140: Entry = Entry {
    name: "queued_invocation_resume140",
    signature: "public.queued_invocation_resume140(uuid,text,text,jsonb)",
    arguments: "p_run uuid, p_namespace text, p_host text, p_platform jsonb",
    result: "boolean",
    names: &["p_run", "p_namespace", "p_host", "p_platform"],
    types: &[
        "pg_catalog.uuid",
        "pg_catalog.text",
        "pg_catalog.text",
        "pg_catalog.jsonb",
    ],
    modes: &["i", "i", "i", "i"],
    table_result: false,
    returns_set: false,
    volatility: "s",
    language: "plpgsql",
    worker: false,
};
