use super::super::SeededInstance;
use super::{RetirementContext, instance_client, mutation_context, opaque};
use rpc_proto::messages::hephaestus::instance::v1::{
    GetInstanceRequest, RemovalState, RemoveAttachmentRequest,
};

pub(crate) async fn remove_attachment_and_assert(
    ctx: &RetirementContext<'_>,
    instance: &SeededInstance,
    operation: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let instance_api = instance_client(
        ctx,
        "/hephaestus.instance.v1.AgentInstanceService/RemoveAttachment",
    )?;
    let remove = RemoveAttachmentRequest {
        context: mutation_context(operation).into(),
        attachment_id: opaque(instance.attachment).into(),
        ..Default::default()
    };
    let first = instance_api
        .remove_attachment(remove.clone())
        .await?
        .into_owned();
    assert_eq!(first.state.to_i32(), RemovalState::Removed as i32);
    assert!(first.receipt.is_set());
    let replay = instance_api.remove_attachment(remove).await?.into_owned();
    assert_eq!(replay.state.to_i32(), RemovalState::Removed as i32);
    assert_eq!(replay.receipt, first.receipt);

    let instance_read = instance_client(
        ctx,
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
    )?
    .get_instance(GetInstanceRequest {
        instance_id: opaque(instance.instance).into(),
        ..Default::default()
    })
    .await?
    .into_owned()
    .instance
    .into_option()
    .ok_or("retired attachment instance missing")?;
    let attachment = instance_read
        .attachments
        .iter()
        .find(|attachment| {
            attachment
                .id
                .as_option()
                .is_some_and(|id| id.value == instance.attachment.to_string())
        })
        .ok_or("removed attachment history missing")?;
    assert!(!attachment.enabled);
    assert!(attachment.removed_at.is_set());
    Ok(())
}
