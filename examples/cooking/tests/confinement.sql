-- Explicit raw-storage scan surfaces. Keep in sync with readable public tables.
SELECT 'agent_attachments' AS surface, row_to_json(stored)::text AS payload FROM public.agent_attachments stored
UNION ALL
SELECT 'agent_capability_bindings' AS surface, row_to_json(stored)::text AS payload FROM public.agent_capability_bindings stored
UNION ALL
SELECT 'agent_config_revisions' AS surface, row_to_json(stored)::text AS payload FROM public.agent_config_revisions stored
UNION ALL
SELECT 'agent_families' AS surface, row_to_json(stored)::text AS payload FROM public.agent_families stored
UNION ALL
SELECT 'agent_git_capability_bindings' AS surface, row_to_json(stored)::text AS payload FROM public.agent_git_capability_bindings stored
UNION ALL
SELECT 'agent_instance_events' AS surface, row_to_json(stored)::text AS payload FROM public.agent_instance_events stored
UNION ALL
SELECT 'agent_instance_revisions' AS surface, row_to_json(stored)::text AS payload FROM public.agent_instance_revisions stored
UNION ALL
SELECT 'agent_instance_state_volumes' AS surface, row_to_json(stored)::text AS payload FROM public.agent_instance_state_volumes stored
UNION ALL
SELECT 'agent_instance_volume_leases' AS surface, row_to_json(stored)::text AS payload FROM public.agent_instance_volume_leases stored
UNION ALL
SELECT 'agent_instances' AS surface, row_to_json(stored)::text AS payload FROM public.agent_instances stored
UNION ALL
SELECT 'agent_secret_bindings' AS surface, row_to_json(stored)::text AS payload FROM public.agent_secret_bindings stored
UNION ALL
SELECT 'agent_updates' AS surface, row_to_json(stored)::text AS payload FROM public.agent_updates stored
UNION ALL
SELECT 'application_aggregate_versions' AS surface, row_to_json(stored)::text AS payload FROM public.application_aggregate_versions stored
UNION ALL
SELECT 'application_event_scopes' AS surface, row_to_json(stored)::text AS payload FROM public.application_event_scopes stored
UNION ALL
SELECT 'application_events' AS surface, row_to_json(stored)::text AS payload FROM public.application_events stored
UNION ALL
SELECT 'authorization_audit_events' AS surface, row_to_json(stored)::text AS payload FROM public.authorization_audit_events stored
UNION ALL
SELECT 'brokered_secret_audit_events' AS surface, row_to_json(stored)::text AS payload FROM public.brokered_secret_audit_events stored
UNION ALL
SELECT 'brokered_secret_lease_snapshots' AS surface, row_to_json(stored)::text AS payload FROM public.brokered_secret_lease_snapshots stored
UNION ALL
SELECT 'brokered_secret_rules' AS surface, row_to_json(stored)::text AS payload FROM public.brokered_secret_rules stored
UNION ALL
SELECT 'build_attempts' AS surface, row_to_json(stored)::text AS payload FROM public.build_attempts stored
UNION ALL
SELECT 'build_executions' AS surface, row_to_json(stored)::text AS payload FROM public.build_executions stored
UNION ALL
SELECT 'build_request_images' AS surface, row_to_json(stored)::text AS payload FROM public.build_request_images stored
UNION ALL
SELECT 'build_request_sources' AS surface, row_to_json(stored)::text AS payload FROM public.build_request_sources stored
UNION ALL
SELECT 'build_requests' AS surface, row_to_json(stored)::text AS payload FROM public.build_requests stored
UNION ALL
SELECT 'build_state_transitions' AS surface, row_to_json(stored)::text AS payload FROM public.build_state_transitions stored
UNION ALL
SELECT 'build_verifications' AS surface, row_to_json(stored)::text AS payload FROM public.build_verifications stored
UNION ALL
SELECT 'capability_audit_events' AS surface, row_to_json(stored)::text AS payload FROM public.capability_audit_events stored
UNION ALL
SELECT 'command_inbox' AS surface, row_to_json(stored)::text AS payload FROM public.command_inbox stored
UNION ALL
SELECT 'control_requests' AS surface, row_to_json(stored)::text AS payload FROM public.control_requests stored
UNION ALL
SELECT 'deferred_agent_triggers' AS surface, row_to_json(stored)::text AS payload FROM public.deferred_agent_triggers stored
UNION ALL
SELECT 'developer_personal_access_tokens' AS surface, row_to_json(stored)::text AS payload FROM public.developer_personal_access_tokens stored
UNION ALL
SELECT 'external_identities' AS surface, row_to_json(stored)::text AS payload FROM public.external_identities stored
UNION ALL
SELECT 'gateway_authorization_snapshot_bindings' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_authorization_snapshot_bindings stored
UNION ALL
SELECT 'gateway_authorization_snapshots' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_authorization_snapshots stored
UNION ALL
SELECT 'gateway_brokered_secret_rules' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_brokered_secret_rules stored
UNION ALL
SELECT 'gateway_configure_commands' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_configure_commands stored
UNION ALL
SELECT 'gateway_derived_configurations' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_derived_configurations stored
UNION ALL
SELECT 'gateway_install_command_results' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_install_command_results stored
UNION ALL
SELECT 'gateway_install_commands' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_install_commands stored
UNION ALL
SELECT 'gateway_invocations' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_invocations stored
UNION ALL
SELECT 'gateway_lifecycle_transitions' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_lifecycle_transitions stored
UNION ALL
SELECT 'gateway_mailbox_binding_commands' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_mailbox_binding_commands stored
UNION ALL
SELECT 'gateway_mailbox_binding_grants' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_mailbox_binding_grants stored
UNION ALL
SELECT 'gateway_mailbox_bindings' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_mailbox_bindings stored
UNION ALL
SELECT 'gateway_mailbox_publications' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_mailbox_publications stored
UNION ALL
SELECT 'gateway_revisions' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_revisions stored
UNION ALL
SELECT 'gateway_routes' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_routes stored
UNION ALL
SELECT 'gateway_runtime_authority_sessions' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_runtime_authority_sessions stored
UNION ALL
SELECT 'gateway_secret_bindings' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_secret_bindings stored
UNION ALL
SELECT 'gateway_secret_leases' AS surface, row_to_json(stored)::text AS payload FROM public.gateway_secret_leases stored
UNION ALL
SELECT 'gateways' AS surface, row_to_json(stored)::text AS payload FROM public.gateways stored
UNION ALL
SELECT 'git_receives' AS surface, row_to_json(stored)::text AS payload FROM public.git_receives stored
UNION ALL
SELECT 'git_ref_updates' AS surface, row_to_json(stored)::text AS payload FROM public.git_ref_updates stored
UNION ALL
SELECT 'git_refs' AS surface, row_to_json(stored)::text AS payload FROM public.git_refs stored
UNION ALL
SELECT 'mailbox_allocation_commands' AS surface, row_to_json(stored)::text AS payload FROM public.mailbox_allocation_commands stored
UNION ALL
SELECT 'mailbox_deliveries' AS surface, row_to_json(stored)::text AS payload FROM public.mailbox_deliveries stored
UNION ALL
SELECT 'mailbox_delivery_attempts' AS surface, row_to_json(stored)::text AS payload FROM public.mailbox_delivery_attempts stored
UNION ALL
SELECT 'mailbox_events' AS surface, row_to_json(stored)::text AS payload FROM public.mailbox_events stored
UNION ALL
SELECT 'mailbox_operator_audit' AS surface, row_to_json(stored)::text AS payload FROM public.mailbox_operator_audit stored
UNION ALL
SELECT 'mailbox_outbox_claims' AS surface, row_to_json(stored)::text AS payload FROM public.mailbox_outbox_claims stored
UNION ALL
SELECT 'mailbox_payloads' AS surface, row_to_json(stored)::text AS payload FROM public.mailbox_payloads stored
UNION ALL
SELECT 'mailboxes' AS surface, row_to_json(stored)::text AS payload FROM public.mailboxes stored
UNION ALL
SELECT 'oci_image_materialization_jobs' AS surface, row_to_json(stored)::text AS payload FROM public.oci_image_materialization_jobs stored
UNION ALL
SELECT 'oci_images' AS surface, row_to_json(stored)::text AS payload FROM public.oci_images stored
UNION ALL
SELECT 'organization_members' AS surface, row_to_json(stored)::text AS payload FROM public.organization_members stored
UNION ALL
SELECT 'organization_secret_managers' AS surface, row_to_json(stored)::text AS payload FROM public.organization_secret_managers stored
UNION ALL
SELECT 'organizations' AS surface, row_to_json(stored)::text AS payload FROM public.organizations stored
UNION ALL
SELECT 'outbox' AS surface, row_to_json(stored)::text AS payload FROM public.outbox stored
UNION ALL
SELECT 'personal_access_token_audit_events' AS surface, row_to_json(stored)::text AS payload FROM public.personal_access_token_audit_events stored
UNION ALL
SELECT 'product_event_dead_letters' AS surface, row_to_json(stored)::text AS payload FROM public.product_event_dead_letters stored
UNION ALL
SELECT 'product_event_outbox' AS surface, row_to_json(stored)::text AS payload FROM public.product_event_outbox stored
UNION ALL
SELECT 'project_capability_granters' AS surface, row_to_json(stored)::text AS payload FROM public.project_capability_granters stored
UNION ALL
SELECT 'project_maintainers' AS surface, row_to_json(stored)::text AS payload FROM public.project_maintainers stored
UNION ALL
SELECT 'project_secret_roles' AS surface, row_to_json(stored)::text AS payload FROM public.project_secret_roles stored
UNION ALL
SELECT 'projects' AS surface, row_to_json(stored)::text AS payload FROM public.projects stored
UNION ALL
SELECT 'registry_namespaces' AS surface, row_to_json(stored)::text AS payload FROM public.registry_namespaces stored
UNION ALL
SELECT 'registry_notification_inbox' AS surface, row_to_json(stored)::text AS payload FROM public.registry_notification_inbox stored
UNION ALL
SELECT 'registry_publication_evidence' AS surface, row_to_json(stored)::text AS payload FROM public.registry_publication_evidence stored
UNION ALL
SELECT 'registry_publication_platforms' AS surface, row_to_json(stored)::text AS payload FROM public.registry_publication_platforms stored
UNION ALL
SELECT 'registry_publications' AS surface, row_to_json(stored)::text AS payload FROM public.registry_publications stored
UNION ALL
SELECT 'release_agents' AS surface, row_to_json(stored)::text AS payload FROM public.release_agents stored
UNION ALL
SELECT 'release_artifacts' AS surface, row_to_json(stored)::text AS payload FROM public.release_artifacts stored
UNION ALL
SELECT 'release_capability_requirements' AS surface, row_to_json(stored)::text AS payload FROM public.release_capability_requirements stored
UNION ALL
SELECT 'release_command_inbox' AS surface, row_to_json(stored)::text AS payload FROM public.release_command_inbox stored
UNION ALL
SELECT 'release_git_capability_ceilings' AS surface, row_to_json(stored)::text AS payload FROM public.release_git_capability_ceilings stored
UNION ALL
SELECT 'releases' AS surface, row_to_json(stored)::text AS payload FROM public.releases stored
UNION ALL
SELECT 'repositories' AS surface, row_to_json(stored)::text AS payload FROM public.repositories stored
UNION ALL
SELECT 'repository_managers' AS surface, row_to_json(stored)::text AS payload FROM public.repository_managers stored
UNION ALL
SELECT 'repository_oci_image_definitions' AS surface, row_to_json(stored)::text AS payload FROM public.repository_oci_image_definitions stored
UNION ALL
SELECT 'repository_oci_image_preparation_events' AS surface, row_to_json(stored)::text AS payload FROM public.repository_oci_image_preparation_events stored
UNION ALL
SELECT 'repository_oci_image_production_jobs' AS surface, row_to_json(stored)::text AS payload FROM public.repository_oci_image_production_jobs stored
UNION ALL
SELECT 'repository_secret_roles' AS surface, row_to_json(stored)::text AS payload FROM public.repository_secret_roles stored
UNION ALL
SELECT 'result_artifacts' AS surface, row_to_json(stored)::text AS payload FROM public.result_artifacts stored
UNION ALL
SELECT 'review_proposals' AS surface, row_to_json(stored)::text AS payload FROM public.review_proposals stored
UNION ALL
SELECT 'run_authorization_snapshot_bindings' AS surface, row_to_json(stored)::text AS payload FROM public.run_authorization_snapshot_bindings stored
UNION ALL
SELECT 'run_authorization_snapshots' AS surface, row_to_json(stored)::text AS payload FROM public.run_authorization_snapshots stored
UNION ALL
SELECT 'run_events' AS surface, row_to_json(stored)::text AS payload FROM public.run_events stored
UNION ALL
SELECT 'run_git_authority_snapshots' AS surface, row_to_json(stored)::text AS payload FROM public.run_git_authority_snapshots stored
UNION ALL
SELECT 'run_instance_provenance' AS surface, row_to_json(stored)::text AS payload FROM public.run_instance_provenance stored
UNION ALL
SELECT 'run_requests' AS surface, row_to_json(stored)::text AS payload FROM public.run_requests stored
UNION ALL
SELECT 'run_results' AS surface, row_to_json(stored)::text AS payload FROM public.run_results stored
UNION ALL
SELECT 'run_secret_provenance' AS surface, row_to_json(stored)::text AS payload FROM public.run_secret_provenance stored
UNION ALL
SELECT 'run_workspaces' AS surface, row_to_json(stored)::text AS payload FROM public.run_workspaces stored
UNION ALL
SELECT 'runs' AS surface, row_to_json(stored)::text AS payload FROM public.runs stored
UNION ALL
SELECT 'runtime_authority_sessions' AS surface, row_to_json(stored)::text AS payload FROM public.runtime_authority_sessions stored
UNION ALL
SELECT 'runtime_git_credentials' AS surface, row_to_json(stored)::text AS payload FROM public.runtime_git_credentials stored
UNION ALL
SELECT 'secret_audit_events' AS surface, row_to_json(stored)::text AS payload FROM public.secret_audit_events stored
UNION ALL
SELECT 'secret_command_inbox' AS surface, row_to_json(stored)::text AS payload FROM public.secret_command_inbox stored
UNION ALL
SELECT 'secret_grants' AS surface, row_to_json(stored)::text AS payload FROM public.secret_grants stored
UNION ALL
SELECT 'secret_imports' AS surface, row_to_json(stored)::text AS payload FROM public.secret_imports stored
UNION ALL
SELECT 'secret_leases' AS surface, row_to_json(stored)::text AS payload FROM public.secret_leases stored
UNION ALL
SELECT 'secret_runtime_mounts' AS surface, row_to_json(stored)::text AS payload FROM public.secret_runtime_mounts stored
UNION ALL
SELECT 'secret_runtime_sessions' AS surface, row_to_json(stored)::text AS payload FROM public.secret_runtime_sessions stored
UNION ALL
SELECT 'secret_versions' AS surface, row_to_json(stored)::text AS payload FROM public.secret_versions stored
UNION ALL
SELECT 'secrets' AS surface, row_to_json(stored)::text AS payload FROM public.secrets stored
UNION ALL
SELECT 'user_profiles' AS surface, row_to_json(stored)::text AS payload FROM public.user_profiles stored
UNION ALL
SELECT 'users' AS surface, row_to_json(stored)::text AS payload FROM public.users stored
