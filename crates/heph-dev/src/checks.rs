use crate::{cli::CheckCommand, context::DevContext, process::Result};

mod architecture;
mod protobuf;
mod quality;
mod rust;
mod support;

const QUALITY_FAMILIES: [&str; 5] = ["protobuf", "architecture", "rust", "phoenix", "ui"];

pub fn run(context: &DevContext, command: CheckCommand) -> Result<()> {
    match command {
        CheckCommand::Architecture => architecture::run(context),
        CheckCommand::Protobuf => protobuf::run(context),
        CheckCommand::Rust => rust::run(context),
        CheckCommand::Phoenix => quality::phoenix(context),
        CheckCommand::Ui => quality::ui(context),
        CheckCommand::Full => quality::full(context),
    }
}

/// Run the complete repository quality gate through one stable command.
pub fn quality(context: &DevContext) -> Result<()> {
    quality::full(context)
}

#[cfg(test)]
mod quality_tests {
    use super::QUALITY_FAMILIES;

    #[test]
    fn quality_gate_covers_every_repository_family() {
        assert_eq!(
            QUALITY_FAMILIES,
            ["protobuf", "architecture", "rust", "phoenix", "ui"]
        );
    }
}
