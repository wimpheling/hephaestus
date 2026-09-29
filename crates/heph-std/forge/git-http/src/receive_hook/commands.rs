use super::{ReceiveHookError, repository::valid_object_name};
use std::io::BufRead;

#[derive(Debug)]
pub(super) struct ReceiveCommand {
    pub(super) old: String,
    pub(super) new: String,
    pub(super) reference: String,
}

pub(super) fn parse_commands(
    commands: impl BufRead,
    maximum: u16,
) -> Result<Vec<ReceiveCommand>, ReceiveHookError> {
    let mut parsed = Vec::new();
    for line in commands.lines().take(usize::from(maximum) + 1) {
        if parsed.len() == usize::from(maximum) {
            return Err(ReceiveHookError::InvalidCommandBatch);
        }
        let line = line.map_err(ReceiveHookError::Io)?;
        let mut fields = line.split(' ');
        let (Some(old), Some(new), Some(reference), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(ReceiveHookError::InvalidCommandBatch);
        };
        if !valid_object_name(old)
            || !valid_object_name(new)
            || old.len() != new.len()
            || !reference.starts_with("refs/")
            || reference.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(ReceiveHookError::InvalidCommandBatch);
        }
        parsed.push(ReceiveCommand {
            old: old.to_owned(),
            new: new.to_owned(),
            reference: reference.to_owned(),
        });
    }
    if parsed.is_empty() {
        return Err(ReceiveHookError::InvalidCommandBatch);
    }
    Ok(parsed)
}
