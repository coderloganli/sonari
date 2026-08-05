# ADR-0011: No multi-tenancy

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `scope`, `data`
- **Related**: ADR-0012

## Context

Sonari is deployed by the person who runs it. There is no scenario in its intended use where one deployment serves mutually distrusting parties.

The predecessor was multi-tenant, and the cost is measurable in its source: tenant identifiers appear at roughly 2,500 sites. Most sit in the tenant-management subsystem itself, but the tax spreads everywhere — every call-scoped function signature carries a tenant argument, every table carries tenant columns, every query carries a tenant predicate, and every one of those is a place to forget the predicate and leak across tenants.

Adding tenancy later to a system that lacks it is a large change. Carrying tenancy that is never exercised is a permanent tax on every signature, schema, and query, and the isolation is untested precisely because there is only ever one tenant.

## Decision

No tenant concept. No `tenant_id`, `workspace_id`, or equivalent in any type, table, or interface.

Multiple personas within one deployment are supported (a persona is configuration, not an isolation boundary) and carry no isolation semantics.

## Consequences

- Function signatures, schemas, and queries stay free of a dimension that would always hold one value.
- No class of bug where a missing predicate crosses an isolation boundary, because there is no boundary to cross.
- Someone needing multi-tenancy must fork. This is the correct trade for a self-hosted project.
- If tenancy is ever required, it is a substantial change — accepted knowingly.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A lightweight workspace grouping | All the cost — extra parameter, extra column, extra predicate — with none of the isolation guarantees, and no user for it |
| Tenant column present but always default | Same tax, plus the false impression that isolation exists and has been tested |
| Design for tenancy, enable later | Untested isolation is not isolation; enabling it later would require an audit of every query anyway |
