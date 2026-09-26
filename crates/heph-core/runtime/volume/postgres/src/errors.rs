use volume_trait::VolumeError;

/// Verifies that a lease request comes from the owning host.
pub fn assert_host(owner: &str, requested: &str) -> Result<(), VolumeError> {
    if owner == requested {
        Ok(())
    } else {
        Err(VolumeError::WrongHost {
            owner_host: owner.to_owned(),
            requested_host: requested.to_owned(),
        })
    }
}

/// Builds an invalid metadata error with a stable message.
pub fn invalid(message: &'static str) -> VolumeError {
    VolumeError::Metadata(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message,
    )))
}

/// Converts an adapter error into the volume metadata error boundary.
pub fn metadata(error: impl std::error::Error + Send + Sync + 'static) -> VolumeError {
    VolumeError::Metadata(Box::new(error))
}
