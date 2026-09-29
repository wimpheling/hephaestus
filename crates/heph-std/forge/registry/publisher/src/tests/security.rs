use super::{outputs::*, support::*};

#[cfg(unix)]
#[test]
fn rejects_path_escape_and_symbolic_links() {
    use std::os::unix::fs::symlink;
    let (root, config, mut material, intent) = setup();
    material.layout = root.path().join("outside");
    fs::create_dir(&material.layout).expect("outside");
    let publisher = scripted_publisher(config.clone(), ScriptedRunner::default());
    let issued = token();
    assert!(matches!(
        publisher.publish(&intent, &material, issued.token()),
        Err(PublisherError::UnsafePath)
    ));
    let target = root.path().join("layouts/image");
    let link = root.path().join("layouts/link");
    symlink(target, &link).expect("link");
    material.layout = link;
    let publisher = scripted_publisher(config.clone(), ScriptedRunner::default());
    let issued = token();
    assert!(matches!(
        publisher.publish(&intent, &material, issued.token()),
        Err(PublisherError::UnsafePath)
    ));
    material.layout = root.path().join("layouts/image");
    material.evidence.scan = write_file(root.path(), "outside-scan.json", b"scan");
    let publisher = scripted_publisher(config, ScriptedRunner::default());
    assert!(matches!(
        publisher.publish(&intent, &material, issued.token()),
        Err(PublisherError::UnsafePath)
    ));
}

#[test]
fn commands_and_debug_redact_bearer_token() {
    let (_root, config, material, intent) = setup();
    let runner = ScriptedRunner::with(successful_outputs(&intent, &material));
    let commands = runner.command_log();
    let publisher = scripted_publisher(config, runner);
    let issued = token();
    let secret = issued.token().as_str().to_owned();
    publisher
        .publish(&intent, &material, issued.token())
        .expect("verified");
    let copy_disables_ambient_credentials = commands
        .lock()
        .expect("commands")
        .iter()
        .find(|command| {
            command
                .arguments()
                .first()
                .is_some_and(|argument| argument == "copy")
        })
        .is_some_and(|command| {
            command
                .arguments()
                .iter()
                .any(|argument| argument == "--dest-no-creds")
        });
    assert!(
        copy_disables_ambient_credentials,
        "publication must not read ambient Skopeo auth.json"
    );
    let debug = format!("{:?}", issued.token());
    assert!(!debug.contains(&secret));
    for command in commands.lock().expect("commands").iter() {
        assert!(!format!("{command:?}").contains(&secret));
        if command
            .arguments()
            .iter()
            .any(|argument| argument.to_string_lossy().contains(&secret))
        {
            let token_position = command
                .arguments()
                .iter()
                .position(|argument| argument.to_string_lossy().contains(&secret))
                .expect("bearer token argument");
            assert_eq!(command.sensitive_argument_positions, vec![token_position]);
        }
        assert!(
            command
                .environment_entries()
                .iter()
                .all(|(_, value)| !value.to_string_lossy().contains(&secret))
        );
    }
}
