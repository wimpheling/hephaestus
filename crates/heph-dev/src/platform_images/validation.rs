use crate::process::{DevError, Result};

pub(super) fn validate_revision(revision: &str) -> Result<()> {
    let valid_length = matches!(revision.len(), 40 | 64);
    if valid_length
        && revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(DevError::Invalid(
            "platform image revision must be a lowercase 40- or 64-character hexadecimal commit"
                .into(),
        ))
    }
}
