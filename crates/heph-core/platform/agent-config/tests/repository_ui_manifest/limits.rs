//! Boundary tests for repository UI manifest collection limits.

use super::support::assert_code;
use agent_config::parse_repository_uis;
use agent_config::ui::{MAX_REPOSITORY_UIS, MAX_UI_APIS, MAX_UI_FILES};
use std::fmt::Write as _;

#[test]
fn enforces_manifest_ui_api_and_file_limits() {
    let mut uis = String::from("version = 1\n");
    for index in 0..=MAX_REPOSITORY_UIS {
        write!(
            &mut uis,
            "\n[[uis]]\nkey = \"ui-{index}\"\nscope = \"global\"\nlabel = \"UI {index}\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"ui-{index}\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/{index}.html\"\nmedia_type = \"text/html\"\n"
        )
        .expect("writing to a String cannot fail");
    }
    assert_code(
        &parse_repository_uis(uis.as_bytes()),
        "too_many_repository_uis",
    );

    let mut apis = String::from(
        "version = 1\n\n[[uis]]\nkey = \"api-ui\"\nscope = \"global\"\nlabel = \"API UI\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"api-ui\"\nui_kit_version = 1\ncache = \"no_store\"\n",
    );
    for index in 0..=MAX_UI_APIS {
        write!(
            &mut apis,
            "\n[[uis.apis]]\nkey = \"api-{index}\"\ngateway_name = \"release-api\"\nmethod = \"GET\"\nroute = \"/api-{index}\"\n"
        )
        .expect("writing to a String cannot fail");
    }
    apis.push_str(
        "\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
    );
    assert_code(
        &parse_repository_uis(apis.as_bytes()),
        "too_many_repository_ui_apis",
    );

    let mut files = String::from(
        "version = 1\n\n[[uis]]\nkey = \"files-ui\"\nscope = \"global\"\nlabel = \"Files UI\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"files-ui\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
    );
    for index in 0..MAX_UI_FILES {
        write!(
            &mut files,
            "\n[[uis.content.files]]\nroute = \"file-{index}.txt\"\nartifact = \"dist/file-{index}.txt\"\nmedia_type = \"text/plain\"\n"
        )
        .expect("writing to a String cannot fail");
    }
    assert_code(
        &parse_repository_uis(files.as_bytes()),
        "invalid_repository_ui_static_files",
    );
}
