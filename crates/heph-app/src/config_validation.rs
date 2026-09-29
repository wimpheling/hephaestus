use super::{
    AppConfig, AppError, LocalCaddyAdministration, LocalCaddyConfigurationTemplate,
    OciImageReference, PermissionsExt, RootFilesystem, VmBackendConfig,
};

impl AppConfig {
    pub(super) fn validate(&self) -> Result<(), AppError> {
        if !self.git_http_backend.is_absolute() {
            return Err(AppError::Configuration(String::from(
                "git_http_backend must be absolute",
            )));
        }
        if !self.git_pre_receive_hook.is_absolute()
            || self.git_pre_receive_hook.file_name() != Some(std::ffi::OsStr::new("pre-receive"))
        {
            return Err(AppError::Configuration(String::from(
                "git_pre_receive_hook must be an absolute path named pre-receive",
            )));
        }
        if self.rpc_mediator_signing_key == [0; 32] {
            return Err(AppError::Configuration(String::from(
                "RPC mediator authentication key must not be all-zero",
            )));
        }
        if !self.secret_broker_socket.is_absolute() {
            return Err(AppError::Configuration(String::from(
                "secret_broker_socket must be absolute",
            )));
        }
        let backend = std::fs::metadata(&self.git_http_backend).map_err(|error| {
            AppError::Configuration(format!("git_http_backend cannot be inspected: {error}"))
        })?;
        if !backend.is_file() || backend.permissions().mode() & 0o111 == 0 {
            return Err(AppError::Configuration(String::from(
                "git_http_backend must be an executable file",
            )));
        }
        let hook = std::fs::symlink_metadata(&self.git_pre_receive_hook).map_err(|error| {
            AppError::Configuration(format!("git_pre_receive_hook cannot be inspected: {error}"))
        })?;
        if hook.file_type().is_symlink()
            || !hook.is_file()
            || hook.permissions().mode() & 0o111 == 0
        {
            return Err(AppError::Configuration(String::from(
                "git_pre_receive_hook must be a non-symlink executable file",
            )));
        }
        if !self.repository_root.is_absolute() {
            return Err(AppError::Configuration(String::from(
                "repository_root must be absolute",
            )));
        }
        if !self.build_workspace_root.is_absolute() || self.build_timeout.is_zero() {
            return Err(AppError::Configuration(String::from(
                "isolated build root must be absolute and timeout must be positive",
            )));
        }
        if !self.runtime_authority_handoff_root.is_absolute()
            || self.runtime_authority_handoff_key == [0; 32]
            || self.runtime_authority_session_ttl.is_zero()
        {
            return Err(AppError::Configuration(String::from(
                "runtime authority handoff root/key and positive session TTL are required",
            )));
        }
        self.validate_oci_builder()?;
        self.validate_gateway_edge()?;
        self.validate_root_images()?;
        if self.runtime_policy.version.trim().is_empty()
            || self.runtime_policy.max_vcpus == 0
            || self.runtime_policy.max_memory_mib == 0
        {
            return Err(AppError::Configuration(String::from(
                "runtime policy version and positive resource ceilings are required",
            )));
        }
        if self.worker_concurrency == 0 {
            return Err(AppError::Configuration(String::from(
                "worker_concurrency must be greater than zero",
            )));
        }
        if self.outbox_batch_size <= 0 {
            return Err(AppError::Configuration(String::from(
                "outbox_batch_size must be greater than zero",
            )));
        }
        self.validate_registry()?;
        if self.startup_timeout.is_zero() || self.shutdown_timeout.is_zero() {
            return Err(AppError::Configuration(String::from(
                "startup and shutdown timeouts must be greater than zero",
            )));
        }
        Ok(())
    }

    fn validate_registry(&self) -> Result<(), AppError> {
        if self.registry.reconciliation_lease.is_zero()
            || self.registry.reconciliation_interval.is_zero()
        {
            return Err(AppError::Configuration(String::from(
                "registry reconciliation durations must be greater than zero",
            )));
        }
        Ok(())
    }

    fn validate_gateway_edge(&self) -> Result<(), AppError> {
        let Some(gateway) = &self.gateway_edge else {
            return Ok(());
        };
        if !gateway.dispatcher_listen.ip().is_loopback()
            || gateway.public_authority.trim().is_empty()
        {
            return Err(AppError::Configuration(String::from(
                "gateway dispatcher must bind loopback and use a non-empty public authority",
            )));
        }
        LocalCaddyAdministration::new(&gateway.caddy_admin_url)
            .map_err(|error| AppError::Configuration(error.to_string()))?;
        let template = LocalCaddyConfigurationTemplate::new(
            &gateway.caddy_configuration_template,
            gateway.caddy_server_name.clone(),
        )
        .map_err(|error| AppError::Configuration(error.to_string()))?;
        if let Some(ui) = &gateway.ui_origin {
            let listener = ui.listener().ok_or_else(|| {
                AppError::Configuration(String::from(
                    "UI origin listener is required when UI origin is enabled",
                ))
            })?;
            if !listener.ip().is_loopback() || listener.port() == 0 {
                return Err(AppError::Configuration(String::from(
                    "UI origin listener must use a nonzero loopback address",
                )));
            }
            template
                .with_ui_namespace(ui.namespace().as_str(), listener)
                .map_err(|error| AppError::Configuration(error.to_string()))?;
        }
        Ok(())
    }

    fn validate_root_images(&self) -> Result<(), AppError> {
        if self.root_images.is_empty() {
            return Err(AppError::Configuration(String::from(
                "at least one root image mapping is required",
            )));
        }
        for (reference, root) in &self.root_images {
            OciImageReference::parse(reference.clone()).map_err(|error| {
                AppError::Configuration(format!(
                    "root image reference {reference:?} is not digest-pinned: {error}"
                ))
            })?;
            let (path, expected_directory) = match root {
                RootFilesystem::Directory { host_path } => (host_path, true),
                RootFilesystem::Disk { host_path, .. } => (host_path, false),
                _ => {
                    return Err(AppError::Configuration(format!(
                        "root image {reference:?} uses an unsupported filesystem variant"
                    )));
                }
            };
            if !path.is_absolute() {
                return Err(AppError::Configuration(format!(
                    "root image {reference:?} materialization path must be absolute"
                )));
            }
            let metadata = std::fs::symlink_metadata(path).map_err(|error| {
                AppError::Configuration(format!(
                    "root image {reference:?} materialization path cannot be inspected: {error}"
                ))
            })?;
            if metadata.file_type().is_symlink() || metadata.is_dir() != expected_directory {
                let expected = if expected_directory {
                    "a non-symlink directory"
                } else {
                    "a non-symlink disk file"
                };
                return Err(AppError::Configuration(format!(
                    "root image {reference:?} materialization path must be {expected}"
                )));
            }
        }
        Ok(())
    }

    fn validate_oci_builder(&self) -> Result<(), AppError> {
        let Some(worker) = &self.oci_builder else {
            return Ok(());
        };
        if worker.runtime.repository_root != self.repository_root
            || !worker.rootfs_root.is_absolute()
            || !worker.root_manifest.is_absolute()
            || !worker.guest_init.is_absolute()
            || !worker.verification_root.is_absolute()
            || !worker.scratch_root.is_absolute()
            || !worker.mkfs_ext4.is_absolute()
            || worker.vm_resources.vcpus == 0
            || worker.vm_resources.memory_mib == 0
            || worker.lease.is_zero()
            || worker.poll_interval.is_zero()
        {
            return Err(AppError::Configuration(String::from(
                "OCI builder roots, manifest, and durations must be explicit and valid",
            )));
        }
        if let VmBackendConfig::Libkrun(provider) = &self.vm_backend
            && !provider.image_roots.contains(&worker.rootfs_root)
        {
            return Err(AppError::Configuration(String::from(
                "libkrun image roots must include the OCI builder rootfs root",
            )));
        }
        for reference in [&worker.builder_vm_image, &worker.verifier_vm_image] {
            if !self.root_images.contains_key(reference.as_str()) {
                return Err(AppError::Configuration(String::from(
                    "OCI operational VM image is not present in the root image manifest",
                )));
            }
        }
        Ok(())
    }
}
