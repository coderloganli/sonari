-- Removes every table left by features this project does not have.
--
-- Cross-checked against the source: none of these names appears anywhere in
-- crates/. They belong to the SDK surface, entitlements, device delivery,
-- event ingestion, the admin console and the phone-number login — all removed.
-- Personas live in sonari.toml and identity is derived from a uid, so nothing
-- here has a reader left.

drop table if exists admins cascade;
drop table if exists api_keys cascade;
drop table if exists auth_permission_catalog cascade;
drop table if exists character_profile_migration_archive cascade;
drop table if exists character_scenes cascade;
drop table if exists device_sdk_events cascade;
drop table if exists device_sdk_output_commands cascade;
drop table if exists device_sdk_output_delivery_attempts cascade;
drop table if exists device_sdk_support_policies cascade;
drop table if exists entitlement_grants cascade;
drop table if exists entitlement_usage_events cascade;
drop table if exists event_sdk_batches cascade;
drop table if exists event_sdk_items cascade;
drop table if exists event_sdk_rejected_items cascade;
drop table if exists notification_reads cascade;
drop table if exists sdk_active_configs cascade;
drop table if exists sdk_apps cascade;
drop table if exists sdk_call_sessions cascade;
drop table if exists sdk_capability_audit_logs cascade;
drop table if exists sdk_config_activation_events cascade;
drop table if exists sdk_config_drafts cascade;
drop table if exists sdk_config_versions cascade;
drop table if exists sdk_credential_abuse_counters cascade;
drop table if exists sdk_credential_abuse_enforcements cascade;
drop table if exists sdk_credential_nonces cascade;
drop table if exists sdk_credential_secret_reveals cascade;
drop table if exists sdk_credentials cascade;
drop table if exists sdk_customer_idempotency_records cascade;
drop table if exists sdk_customer_nonces cascade;
drop table if exists sdk_customer_session_bindings cascade;
drop table if exists sdk_partner_content_configs cascade;
drop table if exists sdk_partners cascade;
drop table if exists sdk_permission_snapshots cascade;
drop table if exists sdk_policy_audit_logs cascade;
drop table if exists sdk_runtime_snapshots cascade;
drop table if exists sdk_security_audit_logs cascade;
drop table if exists sdk_sessions cascade;
drop table if exists sdk_signing_keys cascade;
drop table if exists sdk_token_records cascade;
drop table if exists sdk_users cascade;
drop table if exists sms_codes cascade;
drop table if exists voiceprint_tts_bindings cascade;
