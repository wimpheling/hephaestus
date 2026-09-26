use super::catalog::{
    Catalog, execution_layouts, layout_tag, operational_image, valid_reference, validate_revision,
};
use super::{BUILDER_KEY, VERIFIER_KEY, WORKFLOW_VERSION};
use crate::{
    cli::RepositoryImageEnableArgs,
    context::DevContext,
    process::{DevError, Result, remove_path},
};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};

/// Shows the generated configuration without starting a VM, daemon, or image
/// operation.
pub fn status(context: &DevContext) -> Result<()> {
    let path = context.repository_image_workflow_file();
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let values = workflow_values(&contents)?;
            let state = workflow_prerequisite_error(context, &values);
            println!(
                "repository images {}",
                if state.is_some() {
                    "enabled but unavailable"
                } else {
                    "enabled and usable"
                }
            );
            for key in ["platform_revision", "builder_vm_image", "verifier_vm_image"] {
                let value = values.get(key).map_or("missing", String::as_str);
                println!("{key:18} {value}");
            }
            if let Some(reason) = state {
                println!("unavailable reason {reason}");
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("repository images disabled");
            println!("enable with: cargo dev repository-images enable --revision <revision>");
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn workflow_prerequisite_error(
    context: &DevContext,
    values: &BTreeMap<String, String>,
) -> Option<String> {
    let required = |key| values.get(key).map(String::as_str);
    let revision = required("platform_revision")
        .expect("workflow_values validates required platform revision");
    let installation = context.platform_image_installations().join(revision);
    let catalog_path = installation.join("catalog.json");
    let Some(catalog) = fs::read(&catalog_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Catalog>(&bytes).ok())
    else {
        return Some(String::from(
            "missing or invalid installed platform catalog",
        ));
    };
    for (key, layout_key, tag_key) in [
        (BUILDER_KEY, "builder_layout", "builder_layout_tag"),
        (VERIFIER_KEY, "verifier_layout", "verifier_layout_tag"),
    ] {
        let layout = Path::new(
            required(layout_key).expect("workflow_values validates required operation layout"),
        );
        if !layout.join("index.json").is_file() || !layout.join("oci-layout").is_file() {
            return Some(format!("missing verified {key} OCI layout"));
        }
        let expected_tag =
            required(tag_key).expect("workflow_values validates required operation layout tag");
        let Some(parent) = layout.parent() else {
            return Some(format!("invalid {key} OCI layout path"));
        };
        match layout_tag(&parent.join("release-input.json")) {
            Ok(tag) if tag == expected_tag => {}
            Ok(_) => {
                return Some(format!(
                    "{key} immutable OCI layout tag does not match workflow"
                ));
            }
            Err(_) => return Some(format!("missing or invalid {key} release evidence")),
        }
    }
    if !Path::new(
        required("base_layout_manifest")
            .expect("workflow_values validates required base-layout manifest"),
    )
    .is_file()
    {
        return Some(String::from("missing execution-base layout manifest"));
    }
    for key in [BUILDER_KEY, VERIFIER_KEY] {
        if operational_image(&catalog, key).is_err() {
            return Some(format!("installed catalog lacks usable {key}"));
        }
    }
    None
}

/// Generates private, immutable-reference-only local workflow configuration.
pub fn enable(context: &DevContext, arguments: &RepositoryImageEnableArgs) -> Result<()> {
    validate_revision(&arguments.revision)?;
    let installation = context
        .platform_image_installations()
        .join(&arguments.revision);
    let release = context.platform_image_releases().join(&arguments.revision);
    let catalog_path = installation.join("catalog.json");
    let catalog: Catalog = serde_json::from_slice(&fs::read(&catalog_path)?).map_err(|error| {
        DevError::Invalid(format!(
            "installed platform image catalog is invalid ({}): {error}",
            catalog_path.display()
        ))
    })?;
    let builder = operational_image(&catalog, BUILDER_KEY)?;
    let verifier = operational_image(&catalog, VERIFIER_KEY)?;
    let builder_layout = release.join(BUILDER_KEY).join("image");
    let verifier_layout = release.join(VERIFIER_KEY).join("image");
    for layout in [&builder_layout, &verifier_layout] {
        if !layout.join("index.json").is_file() || !layout.join("oci-layout").is_file() {
            return Err(DevError::Invalid(format!(
                "platform release is missing a verified OCI layout: {}",
                layout.display()
            )));
        }
    }
    let builder_layout_tag = layout_tag(&release.join(BUILDER_KEY).join("release-input.json"))?;
    let verifier_layout_tag = layout_tag(&release.join(VERIFIER_KEY).join("release-input.json"))?;
    let base_layouts = execution_layouts(&catalog, &release)?;

    let root = context.repository_image_workflow_root();
    fs::create_dir_all(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let base_layout_manifest = root.join("base-layouts.json");
    let base_layout_bytes = serde_json::to_vec(&base_layouts).map_err(|error| {
        DevError::Invalid(format!(
            "repository-image base-layout manifest is invalid: {error}"
        ))
    })?;
    write_new_or_matching(&base_layout_manifest, &base_layout_bytes, 0o600)?;
    let workflow = context.repository_image_workflow_file();
    let contents = format!(
        "version={WORKFLOW_VERSION}\nplatform_revision={}\nbuilder_vm_image={builder}\nbuilder_layout={}\nbuilder_layout_tag={builder_layout_tag}\nverifier_vm_image={verifier}\nverifier_layout={}\nverifier_layout_tag={verifier_layout_tag}\nbase_layout_manifest={}\n",
        arguments.revision,
        builder_layout.display(),
        verifier_layout.display(),
        base_layout_manifest.display(),
    );
    match fs::read_to_string(&workflow) {
        Ok(existing) if existing == contents => {
            println!(
                "repository-image workflow is already enabled for {}",
                arguments.revision
            );
            return Ok(());
        }
        Ok(_) => {
            return Err(DevError::Invalid(format!(
                "repository-image workflow is already enabled with different immutable inputs; disable it before changing revisions: {}",
                workflow.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&workflow)?
        .write_all(contents.as_bytes())?;
    println!(
        "enabled repository-image VM workflow for {}",
        arguments.revision
    );
    Ok(())
}

/// Disables the generated workflow configuration without removing immutable
/// platform releases, installation receipts, registry records, or roots.
pub fn disable(context: &DevContext) -> Result<()> {
    remove_path(&context.repository_image_workflow_file())?;
    println!("repository-image VM workflow disabled");
    Ok(())
}

/// Removes only generated local repository-image workflow state.
pub fn clean(context: &DevContext) -> Result<()> {
    remove_path(&context.repository_image_workflow_root())?;
    println!("removed local repository-image workflow state");
    Ok(())
}

fn workflow_values(contents: &str) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for line in contents.lines() {
        let (key, value) = line.split_once('=').ok_or_else(|| {
            DevError::Invalid(String::from("repository-image workflow file is malformed"))
        })?;
        if key.is_empty()
            || value.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            || value.bytes().any(|byte| byte.is_ascii_control())
            || values
                .insert(String::from(key), String::from(value))
                .is_some()
        {
            return Err(DevError::Invalid(String::from(
                "repository-image workflow file is malformed",
            )));
        }
    }
    (values.get("version").is_some_and(|version| version == "1")
        && values.contains_key("platform_revision")
        && values
            .get("builder_vm_image")
            .is_some_and(|value| valid_reference(value))
        && values
            .get("builder_layout")
            .is_some_and(|value| Path::new(value).is_absolute())
        && values.contains_key("builder_layout_tag")
        && values
            .get("verifier_vm_image")
            .is_some_and(|value| valid_reference(value))
        && values
            .get("verifier_layout")
            .is_some_and(|value| Path::new(value).is_absolute())
        && values.contains_key("verifier_layout_tag")
        && values
            .get("base_layout_manifest")
            .is_some_and(|value| Path::new(value).is_absolute()))
    .then_some(values)
    .ok_or_else(|| DevError::Invalid(String::from("repository-image workflow file is incomplete")))
}

fn write_new_or_matching(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
    match fs::read(path) {
        Ok(existing) if existing == contents => Ok(()),
        Ok(_) => Err(DevError::Invalid(format!(
            "immutable repository-image workflow input already differs: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(mode)
                .open(path)?
                .write_all(contents)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
