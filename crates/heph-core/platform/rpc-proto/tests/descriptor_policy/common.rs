use buffa::ExtensionSet as _;
use buffa_descriptor::{DescriptorPool, FieldKind, MessageDescriptor, SingularKind};
use rpc_proto::messages::hephaestus::options::v1::SENSITIVE;
use std::{collections::BTreeSet, sync::Arc};

pub fn pool() -> Arc<DescriptorPool> {
    Arc::new(rpc_proto::descriptor_pool().expect("checked-in descriptor set must decode"))
}

pub fn message_field_is(
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    field_name: &str,
    expected_type: &str,
) -> bool {
    message.field_by_name(field_name).is_some_and(|field| {
        matches!(
            field.kind(),
            FieldKind::Singular(SingularKind::Message(index))
                if pool.message(index).full_name() == expected_type
        )
    })
}

pub fn enum_field_is(
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    field_name: &str,
    expected_type: &str,
) -> bool {
    message.field_by_name(field_name).is_some_and(|field| {
        matches!(
            field.kind(),
            FieldKind::Singular(SingularKind::Enum(index))
                if pool.enumeration(index).full_name() == expected_type
        )
    })
}

pub fn sensitive_fields(pool: &DescriptorPool) -> BTreeSet<String> {
    pool.messages()
        .iter()
        .filter(|message| message.full_name().starts_with("hephaestus."))
        .flat_map(|message| {
            message.fields().iter().filter_map(move |field| {
                field
                    .options()
                    .and_then(|options| options.extension(&SENSITIVE))
                    .filter(|sensitive| *sensitive)
                    .map(|_| format!("{}.{}", message.full_name(), field.name()))
            })
        })
        .collect()
}

pub fn reachable_sensitive_field(
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    visited: &mut BTreeSet<String>,
) -> Option<String> {
    if !visited.insert(message.full_name().to_owned()) {
        return None;
    }
    for field in message.fields() {
        if field
            .options()
            .and_then(|options| options.extension(&SENSITIVE))
            .unwrap_or(false)
        {
            return Some(format!("{}.{}", message.full_name(), field.name()));
        }
        let nested = match field.kind() {
            FieldKind::Singular(SingularKind::Message(index))
            | FieldKind::List(SingularKind::Message(index))
            | FieldKind::Map {
                value: SingularKind::Message(index),
                ..
            } => Some(index),
            _ => None,
        };
        if let Some(found) =
            nested.and_then(|index| reachable_sensitive_field(pool, pool.message(index), visited))
        {
            return Some(found);
        }
    }
    None
}

pub fn message_reaches_named_message(
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    target: &str,
    visited: &mut BTreeSet<String>,
) -> bool {
    if message.full_name() == target {
        return true;
    }
    if !visited.insert(message.full_name().to_owned()) {
        return false;
    }
    message.fields().iter().any(|field| {
        let nested = match field.kind() {
            FieldKind::Singular(SingularKind::Message(index))
            | FieldKind::List(SingularKind::Message(index))
            | FieldKind::Map {
                value: SingularKind::Message(index),
                ..
            } => Some(index),
            _ => None,
        };
        nested.is_some_and(|index| {
            message_reaches_named_message(pool, pool.message(index), target, visited)
        })
    })
}

pub fn reachable_actor_field(
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    visited: &mut BTreeSet<String>,
) -> Option<String> {
    if !visited.insert(message.full_name().to_owned()) {
        return None;
    }
    for field in message.fields() {
        if field.name().contains("actor") {
            return Some(format!("{}.{}", message.full_name(), field.name()));
        }
        let nested = match field.kind() {
            FieldKind::Singular(SingularKind::Message(index))
            | FieldKind::List(SingularKind::Message(index))
            | FieldKind::Map {
                value: SingularKind::Message(index),
                ..
            } => Some(index),
            _ => None,
        };
        if let Some(found) =
            nested.and_then(|index| reachable_actor_field(pool, pool.message(index), visited))
        {
            return Some(found);
        }
    }
    None
}

pub fn contains_forbidden_sensitive_name(name: &str) -> bool {
    [
        "plaintext",
        "ciphertext",
        "credential",
        "password",
        "private_key",
        "api_key",
        "access_token",
        "refresh_token",
    ]
    .iter()
    .any(|forbidden| name.contains(forbidden))
}
